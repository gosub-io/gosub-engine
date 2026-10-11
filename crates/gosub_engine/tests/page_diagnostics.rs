//! A navigation reports what the engine could not use from the page: the HTML parser's errors
//! and the CSS the page's own stylesheets said that was dropped.

#![allow(clippy::unwrap_used, clippy::expect_used)] // a test that cannot set up must fail loudly

use std::sync::Arc;
use std::time::Duration;

use gosub_engine::cookies::DefaultCookieJar;
use gosub_engine::diagnostics::{DroppedCssKind, PageDiagnostics};
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

/// No doctype and a stray end tag for the parser; an unknown property, a value its grammar
/// rejects and an at-rule the engine does not implement for the cascade. Two elements use the
/// rule with the bad value.
const PAGE: &str = "<html><head><style>\na { dsiplay: block }\nb { color: 10px }\n@keyframes spin { }\n</style></head>\n<body><a>x</a><b>y</b><b>z</b></i></body></html>";

async fn diagnostics_for(html: &str) -> PageDiagnostics {
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
        html: html.to_string(),
        base_url: "https://example.test/page".to_string(),
    })
    .await
    .unwrap();
    tab.send(TabCommand::ResumeDrawing { fps: 60 }).await.unwrap();

    let found = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match events.recv().await {
                Ok(EngineEvent::Navigation {
                    tab_id,
                    event: NavigationEvent::Diagnostics { diagnostics, .. },
                }) if tab_id == tab.tab_id => return Some(diagnostics),
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(_) => return None,
            }
        }
    })
    .await;
    let _ = engine.shutdown().await;
    task.abort();
    found.ok().flatten().expect("a Diagnostics event for the navigation")
}

#[tokio::test]
async fn a_navigation_reports_parse_errors_and_dropped_css() {
    let report = diagnostics_for(PAGE).await;
    assert_eq!(
        report.html_error_count, 2,
        "the doctype and the stray </i>: {report:#?}"
    );
    let first = &report.html_errors[0];
    assert_eq!(
        (first.message.as_str(), first.line, first.column),
        ("expected-doctype-but-got-start-tag", 1, 1)
    );
    assert_eq!(report.html_errors[1].line, 6, "the </i> is on the last line");

    let entries: Vec<(DroppedCssKind, &str, u32)> = report
        .css_dropped
        .iter()
        .map(|dropped| (dropped.kind, dropped.subject.as_str(), dropped.line))
        .collect();
    assert!(
        entries.contains(&(DroppedCssKind::UnknownProperty, "dsiplay", 2)),
        "{entries:?}"
    );
    assert!(
        entries.contains(&(DroppedCssKind::UnsupportedAtRule, "@keyframes", 4)),
        "{entries:?}"
    );
    assert_eq!(
        entries
            .iter()
            .filter(|entry| **entry == (DroppedCssKind::InvalidValue, "color", 3))
            .count(),
        1,
        "once per rule, not per element: {entries:?}"
    );
    assert_eq!(report.css_dropped_count, report.css_dropped.len());
    assert!(
        report
            .css_dropped
            .iter()
            .all(|dropped| dropped.stylesheet.contains("example.test")),
        "only the page's own sheets: {entries:?}"
    );
}
