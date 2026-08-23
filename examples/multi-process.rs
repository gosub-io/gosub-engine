//! A browser-shaped embedder that runs the engine with process isolation.
//!
//! The other examples are single-webview and single-process: they never spawn a
//! component, so they need none of the setup below. This one shows the whole
//! arrangement in one place.
//!
//! Two things are required:
//!
//! 1. Call [`child_process::dispatch_with`] as the first statement of `main`.
//!    The engine starts a component by re-exec'ing this binary with a role
//!    argument, so a child must reach its role before the embedder's own
//!    startup runs. In the parent this returns immediately. It takes the
//!    embedder's configuration type because a renderer runs the pipeline over
//!    your document and font types; the plain `dispatch` still works for an
//!    embedder that wants only the network and decoder processes.
//! 2. Turn on the isolation settings before [`GosubEngine::start`].
//!    `security.network_process` moves the network stack out;
//!    `security.image_decoder_process` decodes each image in a throwaway
//!    process; `security.renderer_process` renders pages out of process. All
//!    are read once, as the engine starts.
//!
//! Get (1) wrong and the engine says so and falls back to in-process networking
//! rather than misbehaving: the child would otherwise re-enter this `main` and
//! start spawning engines of its own, so it is refused outright.
//!
//! # What is actually separate today
//!
//! Three long-lived components. A steady-state tree looks like
//!
//! ```text
//! multi-process https://a https://b                    <- broker: tabs, DOM, cookies, storage
//!  |_ multi-process --gosub-child-role net 11          <- the network stack, one for the engine
//!  \_ multi-process --gosub-child-role fork-server 13  <- warmed fonts; renderers fork from here
//!      \_ (a renderer, per render, gone when the page is done)
//! ```
//!
//! plus one short-lived decoder per image, visible in `ps` only while a page
//! is loading - each decodes a single image and exits, so a decoder cannot
//! carry anything from one image into the next.
//!
//! Four things the tree does not show:
//!
//! * One network process serves the whole engine, not one per tab or zone. It
//!   holds no per-zone state; the connection pooling that is per-zone lives
//!   inside it.
//! * The fork server renders nothing itself: it holds a warmed font system so
//!   each forked renderer inherits it copy-on-write. Whether it exists depends
//!   on the font system: one that reads font files while shaping (Pango, Skia)
//!   gets throwaway renderers spawned per render instead, with read-only
//!   access to the font paths.
//! * A renderer has no network. Every image, stylesheet and web font is
//!   requested back through the broker, which performs the fetch where cookies
//!   and identity live, and hands back only bytes. Its rasterized tiles return
//!   as sealed shared memory, mapped rather than copied.
//! * Children show as low-priority in `ps` (`N` in the state column) because the
//!   sandbox lowers child scheduling priority along with its other limits.
//!
//! Run it with:
//!
//! ```sh
//! cargo run --example multi-process -- https://example.com https://example.org
//! ```
//!
//! Nothing is drawn - a `NullBackend` keeps the example about the process
//! model. The renderer processes still run the whole pipeline (parse, style,
//! layout, paint); with no rasterizer configured for them they simply produce
//! no pixels, which is what a `NullBackend` embedder wants anyway. A GUI
//! embedder supplies one through `RenderConfiguration::forked_tile_rasterizer`.

// Example code: panicking on bad input is the desired behavior, as in any test code.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use gosub_config::settings::Setting;
use gosub_engine::events::{EngineEvent, NavigationEvent, TabCommand};
use gosub_engine::storage::{InMemoryLocalStore, InMemorySessionStore, PartitionPolicy, StorageService};
use gosub_engine::zone::ZoneServices;
use gosub_engine::GosubEngine;
use gosub_render_pipeline::render::backends::null::NullBackend;
use gosub_render_pipeline::render::DefaultCompositor;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

const DEFAULT_URLS: &[&str] = &["https://example.com/", "https://example.org/"];

/// How long to keep the engine alive after the pages settle, so the process
/// tree can actually be inspected.
const HOLD_SECS: u64 = 20;

fn main() {
    // (1) Always first. In a child this runs the component role and exits, so
    // nothing below is reached there. `dispatch_with` rather than `dispatch`:
    // the renderer roles need this embedder's configuration type.
    gosub_engine::child_process::dispatch_with::<gosub_engine::DefaultRenderConfig>();

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
    for key in [
        "security.network_process",
        "security.image_decoder_process",
        "security.renderer_process",
    ] {
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
    println!("  ├─ decoder: one throwaway child per image, gone as soon as it has decoded");
    println!("  └─ fork-server: warmed fonts; each page renders in a renderer forked from it");
    println!("     a renderer has no network: its images, stylesheets and fonts are brokered back here");

    let services = ZoneServices {
        storage: Arc::new(StorageService::new(
            Arc::new(InMemoryLocalStore::new()),
            Arc::new(InMemorySessionStore::new()),
        )),
        cookie_store: None,
        cookie_jar: None,
        partition_policy: PartitionPolicy::None,
        places: None,
    };
    let mut zone = engine.create_zone(None, services, None).expect("create zone");

    // Several tabs in one zone, all fetching through the single network process.
    // A viewport and a draw loop are what make renderers happen: a tab that
    // never draws never renders, and the fork server would sit idle.
    let mut tabs = Vec::new();
    for url in &urls {
        let tab = zone.create_tab(Default::default(), None).await.expect("create tab");
        println!("tab {} -> {url}", tab.tab_id);
        tab.send(TabCommand::SetViewport {
            x: 0,
            y: 0,
            width: 1280,
            height: 800,
        })
        .await
        .expect("set viewport");
        tab.navigate(url.clone()).await.expect("navigate");
        tab.send(TabCommand::ResumeDrawing { fps: 10 })
            .await
            .expect("resume drawing");
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

    // Hold the tree open long enough to be looked at: renderers come and go
    // per render, so a process list taken after shutdown shows nothing.
    println!();
    println!(
        "holding for {HOLD_SECS}s — in another terminal:  pstree -ap {}",
        std::process::id()
    );
    tokio::time::sleep(Duration::from_secs(HOLD_SECS)).await;

    engine.close_zone(zone).await;
    engine.shutdown().await.expect("shutdown");
}
