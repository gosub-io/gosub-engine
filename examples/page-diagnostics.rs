// Example code: panicking on bad input is the desired behavior, as in any test code.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Load a page and print what the engine could not use from it: the HTML parser's errors and
//! the CSS the page's own stylesheets said that was dropped.
//!
//! `cargo run --example page-diagnostics -- https://example.com`

use gosub_engine::cookies::DefaultCookieJar;
use gosub_engine::diagnostics::PageDiagnostics;
use gosub_engine::events::{EngineEvent, NavigationEvent, TabCommand};
use gosub_engine::storage::{InMemoryLocalStore, InMemorySessionStore, PartitionPolicy, StorageService};
use gosub_engine::zone::ZoneServices;
use gosub_engine::{DefaultRenderConfig, EngineConfig, GosubEngine};
use gosub_render_pipeline::render::backends::null::NullBackend;
use gosub_render_pipeline::render::DefaultCompositor;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

fn main() {
    gosub_engine::child_process::dispatch_with::<DefaultRenderConfig>();
    let url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "https://example.com".to_string());
    let report = tokio::runtime::Runtime::new()
        .expect("tokio runtime")
        .block_on(load(&url));
    match report {
        Some(report) => print(&url, &report),
        None => eprintln!("{url}: no diagnostics arrived (the load failed or timed out)"),
    }
}

async fn load(url: &str) -> Option<PageDiagnostics> {
    let mut engine: GosubEngine = GosubEngine::new(
        Some(EngineConfig::default()),
        Arc::new(NullBackend::new()),
        Arc::new(DefaultCompositor::default()),
    );
    let task = tokio::spawn(engine.start().expect("engine start"));
    let mut events = engine.subscribe_events();
    let services = ZoneServices {
        storage: Arc::new(StorageService::new(
            Arc::new(InMemoryLocalStore::new()),
            Arc::new(InMemorySessionStore::new()),
        )),
        cookie_store: None,
        cookie_jar: Some(DefaultCookieJar::new().into()),
        partition_policy: PartitionPolicy::None,
        places: None,
    };
    let mut zone = engine.zone_builder().services(services).create().expect("zone");
    let tab = zone.tab_builder().create().await.expect("tab");
    tab.send(TabCommand::SetViewport {
        x: 0,
        y: 0,
        width: 1280,
        height: 800,
    })
    .await
    .unwrap();
    tab.send(TabCommand::Navigate { url: url.to_string() }).await.unwrap();
    tab.send(TabCommand::ResumeDrawing { fps: 30 }).await.unwrap();

    let report = tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            match events.recv().await {
                Ok(EngineEvent::Navigation {
                    tab_id,
                    event: NavigationEvent::Diagnostics { diagnostics, .. },
                }) if tab_id == tab.tab_id => return Some(diagnostics),
                Ok(EngineEvent::Navigation {
                    tab_id,
                    event: NavigationEvent::Failed { error, .. },
                }) if tab_id == tab.tab_id => {
                    eprintln!("navigation failed: {error}");
                    return None;
                }
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(_) => return None,
            }
        }
    })
    .await
    .ok()
    .flatten();
    let _ = engine.shutdown().await;
    task.abort();
    report
}

fn print(url: &str, report: &PageDiagnostics) {
    println!("{url}\n");
    println!("HTML parse errors: {}", report.html_error_count);
    for error in report.html_errors.iter().take(15) {
        println!("  {:>5}:{:<4} {}", error.line, error.column, error.message);
    }
    if report.html_errors.len() > 15 {
        println!("  ... {} more", report.html_error_count - 15);
    }

    println!("\nDropped CSS: {}", report.css_dropped_count);
    // Grouped by what was dropped: a page usually drops the same few things many times.
    let mut grouped: BTreeMap<(String, String), (usize, u32, String)> = BTreeMap::new();
    for dropped in &report.css_dropped {
        let entry = grouped
            .entry((format!("{:?}", dropped.kind), dropped.subject.clone()))
            .or_insert((0, dropped.line, dropped.stylesheet.clone()));
        entry.0 += 1;
    }
    let mut rows: Vec<_> = grouped.into_iter().collect();
    rows.sort_by_key(|row| std::cmp::Reverse(row.1 .0));
    for ((kind, subject), (count, line, sheet)) in rows.iter().take(25) {
        let sheet = sheet.split('?').next().unwrap_or(sheet);
        let sheet = sheet.rsplit('/').next().unwrap_or(sheet);
        println!("  {count:>4}x  {kind:<18} {subject:<40} first at {sheet}:{line}");
    }
    if rows.len() > 25 {
        println!("  ... {} more kinds", rows.len() - 25);
    }
}
