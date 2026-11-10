//! An in-process navigation reports its page title with `TitleChanged`, and only when the title
//! changed. The tab handle answers the same title.

#![allow(clippy::unwrap_used, clippy::expect_used)] // a test that cannot set up must fail loudly

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

#[tokio::test]
async fn an_in_process_page_reports_a_new_title() {
    let mut engine: GosubEngine = GosubEngine::new(
        Some(EngineConfig::default()),
        Arc::new(NullBackend::new()),
        Arc::new(DefaultCompositor::default()),
    );
    let task = tokio::spawn(engine.start().expect("engine start"));
    let mut events = engine.subscribe_events();
    let mut zone = engine.zone_builder().services(services()).create().expect("zone");
    let tab = zone.tab_builder().create().await.expect("tab");

    // The titles reported up to and including the navigation's Finished, and what the handle
    // answered when each arrived.
    let mut load = async |html: &str| -> Vec<(String, String)> {
        tab.send(TabCommand::LoadHtml {
            html: html.to_string(),
            base_url: "https://example.test/page".to_string(),
        })
        .await
        .unwrap();
        let mut seen = Vec::new();
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                match events.recv().await {
                    Ok(EngineEvent::TitleChanged { tab_id, title }) if tab_id == tab.tab_id => {
                        seen.push((title, tab.title()));
                    }
                    Ok(EngineEvent::Navigation {
                        tab_id,
                        event: NavigationEvent::Finished { .. },
                    }) if tab_id == tab.tab_id => return,
                    Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(_) => panic!("event bus closed"),
                }
            }
        })
        .await
        .expect("the navigation finishes");
        seen
    };

    let first = load("<title>First</title><p>a").await;
    let same = load("<title>First</title><p>b").await;
    let second = load("<title>Second</title><p>c").await;

    let _ = engine.shutdown().await;
    task.abort();

    assert_eq!(first, [("First".to_string(), "First".to_string())]);
    assert_eq!(same, [], "the title did not change");
    assert_eq!(second, [("Second".to_string(), "Second".to_string())]);
}
