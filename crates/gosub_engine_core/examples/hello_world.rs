//! hello_world — minimal gosub-engine-core example
//!
//! Navigates to a URL (default: https://example.com), prints engine events
//! until navigation finishes, then shuts down.  Uses the NullBackend so no
//! display is required.
//!
//! Run:
//!   cargo run --example hello_world -- https://gosub.io

use std::sync::{Arc, RwLock};

use gosub_css3::system::Css3System;
use gosub_engine_core::cookies::DefaultCookieJar;
use gosub_engine_core::events::{EngineEvent, NavigationEvent, TabCommand};
use gosub_engine_core::render::backends::null::NullBackend;
use gosub_engine_core::render::DefaultCompositor;
use gosub_engine_core::storage::{InMemoryLocalStore, InMemorySessionStore, PartitionPolicy, StorageService};
use gosub_engine_core::zone::{ZoneConfig, ZoneServices};
use gosub_engine_core::{EngineConfig, GosubEngine};
use gosub_html5::document::builder::DocumentBuilderImpl;
use gosub_html5::document::document_impl::DocumentImpl;
use gosub_html5::document::fragment::DocumentFragmentImpl;
use gosub_interface::config::{HasCssSystem, HasDocument};

// ---------------------------------------------------------------------------
// Config: wires together the HTML5 / CSS3 concrete implementations.
// HasHtmlParser is not needed here — the engine uses async HTML parsing
// internally which only depends on HasDocument.
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
struct Config;

impl HasCssSystem for Config {
    type CssSystem = Css3System;
}

impl HasDocument for Config {
    type Document = DocumentImpl<Self>;
    type DocumentFragment = DocumentFragmentImpl<Self>;
    type DocumentBuilder = DocumentBuilderImpl;
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "https://example.com".to_string());

    println!("gosub hello_world — navigating to {url}");

    // 1. Backend (null — no display) + compositor
    let backend = Arc::new(NullBackend::new()?);
    let compositor = Arc::new(RwLock::new(DefaultCompositor::default()));

    // 2. Engine
    let mut engine = GosubEngine::new(Some(EngineConfig::default()), backend, compositor);
    engine.start()?;

    // 3. Zone services (ephemeral in-memory storage + cookie jar)
    let services = ZoneServices {
        storage: Arc::new(StorageService::new(
            Arc::new(InMemoryLocalStore::new()),
            Arc::new(InMemorySessionStore::new()),
        )),
        cookie_store: None,
        cookie_jar: Some(DefaultCookieJar::new().into()),
        partition_policy: PartitionPolicy::None,
    };

    // 4. Subscribe before creating zones/tabs so we don't miss ZoneCreated /
    //    TabCreated events (broadcast sends fail with zero receivers).
    let mut events = engine.subscribe_events();

    // 5. Zone → Tab
    let mut zone = engine.create_zone(ZoneConfig::default(), services, None)?;
    let tab = zone.create_tab::<Config>(Default::default(), None).await?;

    // 6. Navigate
    tab.send(TabCommand::Navigate { url }).await?;
    tab.send(TabCommand::SetViewport {
        x: 0,
        y: 0,
        width: 1280,
        height: 800,
    })
    .await?;
    loop {
        match events.recv().await {
            Ok(EngineEvent::Navigation { tab_id, event }) => {
                println!("[{tab_id:?}] {event:?}");
                match event {
                    NavigationEvent::Finished { .. } | NavigationEvent::Failed { .. } => break,
                    _ => {}
                }
            }
            Ok(EngineEvent::Resource { tab_id, event }) => {
                println!("[{tab_id:?}] resource: {event:?}");
            }
            Ok(EngineEvent::EngineShutdown { .. }) | Err(_) => break,
            _ => {}
        }
    }

    engine.shutdown().await?;
    println!("Done.");
    Ok(())
}
