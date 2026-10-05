//! End-to-end checks for the embedder-facing contract: the frame wakeup fires, and the
//! tab's read-side state is readable without replaying the event stream.

use std::sync::Arc;
use std::time::Duration;

use gosub_engine::cookies::DefaultCookieJar;
use gosub_engine::events::{EngineEvent, NavigationEvent, TabCommand};
use gosub_engine::storage::{InMemoryLocalStore, InMemorySessionStore, PartitionPolicy, StorageService};
use gosub_engine::zone::ZoneServices;
use gosub_engine::{EngineConfig, GosubEngine};
use gosub_render_pipeline::render::backends::null::NullBackend;
use gosub_render_pipeline::render::DefaultCompositor;

fn services() -> ZoneServices {
    ZoneServices {
        storage: Arc::new(StorageService::new(
            Arc::new(InMemoryLocalStore::new()),
            Arc::new(InMemorySessionStore::new()),
        )),
        cookie_store: None,
        cookie_jar: Some(DefaultCookieJar::new().into()),
        partition_policy: PartitionPolicy::None,
        places: None,
    }
}

const PAGE_ONE: &str = "<html><head><title>First</title></head><body><p>one</p></body></html>";
const PAGE_TWO: &str = "<html><head><title>Second</title></head><body><p>two</p></body></html>";

#[tokio::test]
async fn redraw_wakeup_fires_and_tab_state_is_readable() {
    let mut engine: GosubEngine = GosubEngine::new(
        Some(EngineConfig::default()),
        Arc::new(NullBackend::new()),
        Arc::new(DefaultCompositor::default()),
    );
    let task = tokio::spawn(engine.start().expect("engine start"));
    let mut events = engine.subscribe_events();

    let mut zone = engine.zone_builder().services(services()).create().expect("zone");
    let tab = zone.tab_builder().create().await.expect("tab");

    tab.send(TabCommand::SetViewport {
        x: 0,
        y: 0,
        width: 800,
        height: 600,
    })
    .await
    .unwrap();
    tab.send(TabCommand::LoadHtml {
        html: PAGE_ONE.to_string(),
        base_url: "https://example.test/one".to_string(),
    })
    .await
    .unwrap();
    tab.send(TabCommand::ResumeDrawing { fps: 60 }).await.unwrap();

    // The wakeup must fire on its own: nothing else tells a shell a frame is ready. A
    // frame may be drawn before the document commits (the tab paints as soon as drawing
    // resumes), so the read-side state is checked once the load has finished too.
    let redraw = tokio::time::timeout(Duration::from_secs(10), async {
        let (mut drawn, mut finished) = (false, false);
        loop {
            match events.recv().await {
                Ok(EngineEvent::Redraw { tab_id }) if tab_id == tab.tab_id => drawn = true,
                Ok(EngineEvent::Navigation {
                    tab_id,
                    event: NavigationEvent::Finished { .. },
                }) if tab_id == tab.tab_id => finished = true,
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(_) => return false,
            }
            if drawn && finished {
                return true;
            }
        }
    })
    .await;
    assert!(
        matches!(redraw, Ok(true)),
        "no Redraw and Finished for the tab: {redraw:?}"
    );

    assert_eq!(
        tab.url().map(|u| u.to_string()).as_deref(),
        Some("https://example.test/one")
    );
    assert_eq!(tab.title(), "First");
    assert!(!tab.can_go_back(), "a single entry has nothing behind it");
    assert!(!tab.can_go_forward());

    // A second document pushes a history entry, which back/forward must reflect.
    tab.send(TabCommand::LoadHtml {
        html: PAGE_TWO.to_string(),
        base_url: "https://example.test/two".to_string(),
    })
    .await
    .unwrap();

    let moved = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if tab.url().map(|u| u.to_string()).as_deref() == Some("https://example.test/two") {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    assert!(matches!(moved, Ok(true)), "second document never committed");

    assert_eq!(tab.title(), "Second");
    assert!(tab.can_go_back(), "two entries means back is available");
    assert!(!tab.can_go_forward());

    let _ = engine.shutdown().await;
    task.abort();
}
