//! A navigation to XHTML renders the page, and one to XML shows it as text. Neither is offered
//! as a download.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // a test that cannot set up must fail loudly

use std::sync::Arc;
use std::time::Duration;

use gosub_engine::cookies::DefaultCookieJar;
use gosub_engine::events::{EngineEvent, NavigationEvent};
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

#[derive(Debug, PartialEq)]
enum Ending {
    /// The navigation finished with this page title.
    Page(String),
    Download,
    Failed,
}

/// Navigate a fresh tab to each URL and say how each navigation ended.
async fn navigate_all(urls: &[&str]) -> Vec<Ending> {
    let mut engine: GosubEngine = GosubEngine::new(
        Some(EngineConfig::default()),
        Arc::new(NullBackend::new()),
        Arc::new(DefaultCompositor::default()),
    );
    let task = tokio::spawn(engine.start().expect("engine start"));
    let mut events = engine.subscribe_events();
    let mut zone = engine.zone_builder().services(services()).create().expect("zone");

    let mut endings = Vec::new();
    for url in urls {
        let tab = zone.tab_builder().create().await.expect("tab");
        tab.navigate(*url).await.unwrap();
        let ending = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                match events.recv().await {
                    Ok(EngineEvent::Navigation {
                        tab_id,
                        event: NavigationEvent::Finished { .. },
                    }) if tab_id == tab.tab_id => return Ending::Page(tab.title()),
                    Ok(EngineEvent::Navigation {
                        tab_id,
                        event: NavigationEvent::Failed { .. },
                    }) if tab_id == tab.tab_id => return Ending::Failed,
                    Ok(EngineEvent::DownloadRequested { tab_id, .. }) if tab_id == tab.tab_id => {
                        return Ending::Download
                    }
                    Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(_) => panic!("event bus closed"),
                }
            }
        })
        .await
        .expect("the navigation ends");
        endings.push(ending);
    }

    let _ = engine.shutdown().await;
    task.abort();
    endings
}

#[tokio::test]
async fn an_xhtml_navigation_renders_the_page() {
    let endings = navigate_all(&[
        "data:application/xhtml+xml,<html><head><title>Page</title></head><body>x</body></html>",
        "data:application/xhtml+xml;charset=utf-8,<html><head><title>Page</title></head></html>",
    ])
    .await;
    assert_eq!(endings, [Ending::Page("Page".into()), Ending::Page("Page".into())]);
}

/// What a tab without a page title is called (`zone.rs`).
const NO_TITLE: &str = "New Tab";

/// The `<title>` in the XML stays text: no title means the body was not parsed as HTML.
#[tokio::test]
async fn an_xml_navigation_shows_the_text() {
    let endings = navigate_all(&[
        "data:application/xml,<r><title>T</title></r>",
        "data:text/xml,<r><title>T</title></r>",
        "data:application/rss+xml,<rss><title>T</title></rss>",
    ])
    .await;
    assert_eq!(
        endings,
        [
            Ending::Page(NO_TITLE.into()),
            Ending::Page(NO_TITLE.into()),
            Ending::Page(NO_TITLE.into())
        ]
    );
}
