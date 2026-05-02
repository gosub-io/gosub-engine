//! Minimal GoSub tutorial example.
//!
//! Demonstrates the core lifecycle:
//!   Engine → Zone → Tab → Navigate → Event loop → Shutdown
//!
//! Run with:
//!   cargo run --example tutorial -- https://example.com
//!
//! See docs/tutorial.md for a step-by-step walkthrough.

use gosub_engine::{
    cookies::DefaultCookieJar,
    events::{EngineEvent, NavigationEvent, TabCommand},
    render::{backends::null::NullBackend, DefaultCompositor, Viewport},
    storage::{InMemoryLocalStore, InMemorySessionStore, PartitionPolicy, StorageService},
    tab::{TabDefaults, TabHandle},
    zone::{ZoneConfig, ZoneServices},
    Action, EngineConfig, EngineError, GosubEngine,
};
use std::sync::{Arc, RwLock};

#[tokio::main]
async fn main() -> Result<(), EngineError> {
    let url = std::env::args().nth(1).unwrap_or_else(|| "https://example.com".into());

    // ── Step 1: Create the engine ────────────────────────────────────────────────
    //
    // GosubEngine is the central hub. It owns the event bus, networking stack,
    // and render backend. You provide:
    //   - a render backend (NullBackend here — no pixels, just navigation)
    //   - a compositor (receives Redraw events and composites them into a frame)
    //
    // EngineConfig lets you tune limits like max_zones; Default is fine to start.
    let backend = NullBackend::new().expect("null backend");
    let mut engine = GosubEngine::new(
        Some(EngineConfig::default()),
        Arc::new(backend),
        Arc::new(RwLock::new(DefaultCompositor::default())),
    );

    // start() launches the engine's internal Tokio tasks.
    let join_handle = engine.start().expect("cannot start engine");

    // Subscribe before creating any tabs so we don't miss early events.
    let mut events = engine.subscribe_events();

    // ── Step 2: Create a zone ────────────────────────────────────────────────────
    //
    // A Zone is an isolated browsing profile: it owns cookies, local/session
    // storage, and a set of tabs. Think of it like a browser profile or a
    // private window. Multiple zones can coexist within one engine instance.
    let services = ZoneServices {
        storage: Arc::new(StorageService::new(
            Arc::new(InMemoryLocalStore::new()),
            Arc::new(InMemorySessionStore::new()),
        )),
        cookie_store: None,
        cookie_jar: Some(DefaultCookieJar::new().into()),
        partition_policy: PartitionPolicy::None,
    };
    let mut zone = engine.create_zone(ZoneConfig::default(), services, None)?;

    // ── Step 3: Open a tab ───────────────────────────────────────────────────────
    //
    // A Tab is a single browsing context (like a browser tab). You control it
    // through the returned TabHandle by sending TabCommands.
    let tab = zone
        .create_tab(
            TabDefaults {
                viewport: Some(Viewport::new(0, 0, 1280, 800)),
                ..Default::default()
            },
            None,
        )
        .await
        .expect("cannot create tab");

    // ── Step 4: Navigate ─────────────────────────────────────────────────────────
    //
    // TabCommand::Navigate kicks off the async fetch + parse pipeline. The engine
    // emits EngineEvent::Navigation events as the load progresses.
    println!("Navigating to {url}");
    tab.send(TabCommand::Navigate { url }).await?;

    // ── Step 5: Event loop ───────────────────────────────────────────────────────
    //
    // The engine is event-driven: your application reacts to EngineEvent values
    // received from the channel returned by subscribe_events().
    // We stop once navigation finishes (or fails), or when Ctrl-C is pressed.
    loop {
        tokio::select! {
            Ok(ev) = events.recv() => {
                if handle_event(ev, &tab).await {
                    break;
                }
            }
            _ = tokio::signal::ctrl_c() => {
                println!("Interrupted.");
                break;
            }
        }
    }

    // ── Step 6: Shutdown ─────────────────────────────────────────────────────────
    //
    // Always shut the engine down cleanly — this drains in-flight tasks and
    // flushes any pending state before the process exits.
    engine.shutdown().await?;

    if let Some(handle) = join_handle {
        let _ = handle.await;
    }

    println!("Done.");
    Ok(())
}

/// Handles one engine event. Returns `true` when the event loop should stop.
async fn handle_event(ev: EngineEvent, tab: &TabHandle) -> bool {
    match ev {
        EngineEvent::Navigation { event, .. } => match event {
            NavigationEvent::Started { url, .. } => {
                println!("  [nav] started:   {url}");
                false
            }
            NavigationEvent::Committed { url, .. } => {
                println!("  [nav] committed: {url}");
                false
            }
            NavigationEvent::Progress {
                received_bytes,
                expected_length,
                ..
            } => {
                let total = expected_length
                    .map(|n| format!("{} KB", n / 1024))
                    .unwrap_or_else(|| "?".into());
                println!("  [nav] progress:  {} KB / {total}", received_bytes / 1024);
                false
            }
            NavigationEvent::Finished { url, .. } => {
                println!("  [nav] finished:  {url}");
                true // navigation complete — stop the loop
            }
            NavigationEvent::Failed { url, error, .. } => {
                println!("  [nav] FAILED:    {url}  ({error})");
                true
            }
            NavigationEvent::DecisionRequired {
                nav_id, decision_token, ..
            } => {
                // The engine fetched response headers and needs the UA to decide:
                // render the page, or download the file? We always render here.
                let _ = tab
                    .cmd_tx
                    .send(TabCommand::SubmitDecision {
                        nav_id,
                        decision_token,
                        action: Action::Render,
                    })
                    .await;
                false
            }
            _ => false,
        },

        EngineEvent::Redraw { .. } => {
            // With a real backend you would composite a frame into your window here.
            false
        }

        _ => false,
    }
}
