//! A browser-shaped embedder that runs the engine with **process isolation**.
//!
//! The other examples are single-webview and single-process: they never spawn a
//! component, so they need none of the setup below and deliberately do not carry
//! it. This one exists to show the whole arrangement in one place.
//!
//! Two things are required, and only two:
//!
//! 1. **Call [`child_process::dispatch`] as the first statement of `main`.** The
//!    engine starts a component by re-exec'ing *this* binary with a role
//!    argument, so a child must reach its role before the embedder's own startup
//!    runs. In the parent this returns immediately.
//! 2. **Turn on the isolation settings before [`GosubEngine::start`].**
//!    `security.network_process` moves the network stack out;
//!    `security.image_decoder_process` decodes each image in a throwaway
//!    process. Both are read once, as the engine starts.
//!
//! Get (1) wrong and the engine says so and falls back to in-process networking
//! rather than misbehaving: the child would otherwise re-enter this `main` and
//! start spawning engines of its own, so it is refused outright.
//!
//! # What is actually separate today
//!
//! Two components. A steady-state tree looks like
//!
//! ```text
//! multi-process https://a https://b            <- broker: tabs, DOM, cookies, storage
//!  \_ multi-process --gosub-child-role net 11  <- the network stack, one for the engine
//! ```
//!
//! plus **one short-lived decoder per image**, which you will only catch in
//! `ps` if you look while a page is loading — each decodes a single image and
//! exits. That is deliberate: a decoder that exits cannot carry anything from
//! one image into the next.
//!
//! Three things the tree does not show:
//!
//! * **One network process serves the whole engine**, not one per tab or per
//!   zone. It holds no per-zone state; the connection pooling that *is* per-zone
//!   lives inside it. More tabs will not produce more network processes.
//! * **Parsing, layout, painting and cookies still run in the broker.** Per-origin
//!   renderers are the next phase. Until then this isolates the network
//!   capability and image decoding, not a fully multi-process browser.
//! * Children show as low-priority in `ps` (`N` in the state column) because the
//!   sandbox lowers child scheduling priority along with its other limits.
//!
//! Run it with:
//!
//! ```sh
//! cargo run --example multi-process -- https://example.com https://example.org
//! ```
//!
//! Nothing is drawn — a `NullBackend` keeps the example about the process model.

// Example code: panicking on bad input is the desired behavior, as in any test code.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use gosub_config::settings::Setting;
use gosub_engine::events::{EngineEvent, NavigationEvent};
use gosub_engine::storage::{InMemoryLocalStore, InMemorySessionStore, PartitionPolicy, StorageService};
use gosub_engine::zone::ZoneServices;
use gosub_engine::GosubEngine;
use gosub_render_pipeline::render::backends::null::NullBackend;
use gosub_render_pipeline::render::DefaultCompositor;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

const DEFAULT_URLS: &[&str] = &["https://example.com/", "https://example.org/"];

fn main() {
    // (1) Always first. In a child this runs the component role and exits, so
    // nothing below is reached there.
    gosub_engine::child_process::dispatch();

    simple_logger::SimpleLogger::new()
        .with_level(log::LevelFilter::Info)
        .env()
        .init()
        .unwrap();

    let urls: Vec<String> = {
        let args: Vec<String> = std::env::args().skip(1).collect();
        if args.is_empty() {
            DEFAULT_URLS.iter().map(|s| s.to_string()).collect()
        } else {
            args
        }
    };

    // A multi-threaded runtime is required, not a preference: resource loads
    // block on a reply from the I/O runtime, which needs a thread of its own to
    // produce it.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("build runtime");

    runtime.block_on(run(urls));
}

async fn run(urls: Vec<String>) {
    let mut engine: GosubEngine = GosubEngine::new(
        None,
        Arc::new(NullBackend::new()),
        Arc::new(DefaultCompositor::default()),
    );

    // (2) Before start(): the I/O runtime reads this as it comes up.
    for key in ["security.network_process", "security.image_decoder_process"] {
        engine
            .settings()
            .set(key, Setting::Bool(true))
            .unwrap_or_else(|e| panic!("enable {key}: {e}"));
    }

    let mut events = engine.subscribe_events();
    tokio::spawn(engine.start().expect("start engine"));

    // Say plainly which components are separate, so the process tree is not read
    // as a claim that everything is. Watch for the engine's own
    // "network stack running in a separate, sandboxed process" line just above:
    // without it, isolation silently fell back to in-process.
    println!(
        "broker pid {} — tabs, DOM, cookies and storage live here",
        std::process::id()
    );
    println!("  ├─ network: one child process for the whole engine (--gosub-child-role net)");
    println!("  └─ decoder: one throwaway child per image, gone as soon as it has decoded");
    println!("     parsing, layout and painting are not yet split out; that is a later phase");

    let services = ZoneServices {
        storage: Arc::new(StorageService::new(
            Arc::new(InMemoryLocalStore::new()),
            Arc::new(InMemorySessionStore::new()),
        )),
        cookie_store: None,
        cookie_jar: None,
        partition_policy: PartitionPolicy::None,
    };
    let mut zone = engine.create_zone(None, services, None).expect("create zone");

    // Several tabs in one zone, all fetching through the single network process.
    let mut tabs = Vec::new();
    for url in &urls {
        let tab = zone.create_tab(Default::default(), None).await.expect("create tab");
        println!("tab {} -> {url}", tab.tab_id);
        tab.navigate(url.clone()).await.expect("navigate");
        tabs.push(tab.tab_id);
    }

    // Wait for every tab to settle, or give up: an example should not hang on a
    // network that is not there.
    let mut settled: HashSet<_> = HashSet::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    while settled.len() < tabs.len() {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            println!("gave up waiting for {} tab(s)", tabs.len() - settled.len());
            break;
        }
        let Ok(Ok(event)) = tokio::time::timeout(remaining, events.recv()).await else {
            break;
        };
        let EngineEvent::Navigation { tab_id, event } = event else {
            continue;
        };
        match event {
            NavigationEvent::Finished { .. } => {
                println!("tab {tab_id}: loaded");
                settled.insert(tab_id);
            }
            NavigationEvent::Failed { error, .. } => {
                println!("tab {tab_id}: failed: {error}");
                settled.insert(tab_id);
            }
            _ => {}
        }
    }

    engine.close_zone(zone).await;
    engine.shutdown().await.expect("shutdown");
}
