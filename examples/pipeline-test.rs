/// End-to-end pipeline smoke test.
///
/// Spins up a tiny local HTTP server that serves a known HTML page (with a
/// title, a stylesheet, a script, and an image), navigates the engine to it,
/// and then checks that:
///   1. Navigation started and finished without error.
///   2. All three sub-resources (CSS / JS / image) were discovered and fetched.
///
/// Exit code 0 = all checks passed, 1 = one or more checks failed.
use std::sync::Arc;
use std::time::Duration;

use gosub_engine::events::{EngineEvent, NavigationEvent, ResourceEvent, TabCommand};
use gosub_engine::net::types::FetchResultMeta;
use gosub_engine::net::DecisionToken;
use gosub_engine::tab::{TabDefaults, TabHandle};
use gosub_engine::{
    cookies::DefaultCookieJar,
    render::{DefaultCompositor, Viewport},
    storage::{InMemoryLocalStore, InMemorySessionStore, PartitionPolicy, StorageService},
    zone::{ZoneConfig, ZoneServices},
    Action, EngineConfig, EngineError, GosubEngine, NavigationId,
};
use parking_lot::RwLock;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;

// ── Test page content ────────────────────────────────────────────────────────

const INDEX_HTML: &str = r#"<!DOCTYPE html>
<html>
  <head>
    <title>Pipeline Test</title>
    <link rel="stylesheet" href="/style.css">
  </head>
  <body>
    <h1>Pipeline is working</h1>
    <script src="/app.js"></script>
    <img src="/pixel.gif">
  </body>
</html>"#;

// Minimal 1×1 transparent GIF89a
const PIXEL_GIF: &[u8] = &[
    0x47, 0x49, 0x46, 0x38, 0x39, 0x61, // GIF89a
    0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, // 1×1, no GCT
    0x2c, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, // image descriptor
    0x02, 0x02, 0x4c, 0x01, 0x00, // image data
    0x3b, // trailer
];

// ── Minimal HTTP server ───────────────────────────────────────────────────────

async fn serve(mut stream: TcpStream) {
    let mut buf = vec![0u8; 4096];
    let n = stream.read(&mut buf).await.unwrap_or(0);
    let req = String::from_utf8_lossy(&buf[..n]);
    let path = req
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .unwrap_or("/");

    let (ct, body): (&str, &[u8]) = match path {
        "/" | "/index.html" => ("text/html; charset=utf-8", INDEX_HTML.as_bytes()),
        "/style.css" => ("text/css; charset=utf-8", b"body { color: #333; }"),
        "/app.js" => ("application/javascript; charset=utf-8", b"// pipeline test"),
        "/pixel.gif" => ("image/gif", PIXEL_GIF),
        _ => ("text/plain", b"not found"),
    };

    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {ct}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(header.as_bytes()).await;
    let _ = stream.write_all(body).await;
}

async fn run_server(listener: TcpListener, stop: CancellationToken) {
    loop {
        tokio::select! {
            _ = stop.cancelled() => break,
            Ok((stream, _)) = listener.accept() => { tokio::spawn(serve(stream)); }
        }
    }
}

// ── Decision handler ─────────────────────────────────────────────────────────

async fn decide(tab: TabHandle, nav_id: NavigationId, meta: FetchResultMeta, token: DecisionToken) {
    let ct = meta
        .headers
        .get(http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let action = if ct.starts_with("text/html") || ct.starts_with("text/") {
        Action::Render
    } else {
        Action::Render // render everything for testing purposes
    };

    let _ = tab
        .cmd_tx
        .send(TabCommand::SubmitDecision {
            nav_id,
            decision_token: token,
            action,
        })
        .await;
}

// ── Test harness ─────────────────────────────────────────────────────────────

#[derive(Default)]
struct Collected {
    nav_started: bool,
    nav_finished: bool,
    nav_error: Option<String>,
    resources_started: Vec<String>,
}

fn check(label: &str, ok: bool) -> bool {
    let mark = if ok { "PASS" } else { "FAIL" };
    println!("  [{mark}]  {label}");
    ok
}

// ── Main ─────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<(), EngineError> {
    // Suppress all log/trace noise — we print our own output.
    let _ = tracing_subscriber::fmt().with_env_filter("error").try_init();

    // Bind on a random port.
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("local_addr").port();
    let base_url = format!("http://127.0.0.1:{port}/");
    let stop_server = CancellationToken::new();
    tokio::spawn(run_server(listener, stop_server.clone()));

    println!("gosub engine — pipeline smoke test");
    println!("Server: {base_url}");
    println!();

    // Start engine.
    let backend = gosub_engine::render::backends::null::NullBackend::new().expect("null backend");
    let mut engine = GosubEngine::new(
        Some(EngineConfig::builder().max_zones(1).build().expect("cfg")),
        Arc::new(backend),
        Arc::new(RwLock::new(DefaultCompositor::default())),
    );
    let engine_join = engine.start().expect("start");
    let mut events = engine.subscribe_events();

    let zone_services = ZoneServices {
        storage: Arc::new(StorageService::new(
            Arc::new(InMemoryLocalStore::new()),
            Arc::new(InMemorySessionStore::new()),
        )),
        cookie_store: None,
        cookie_jar: Some(DefaultCookieJar::new().into()),
        partition_policy: PartitionPolicy::None,
    };
    let mut zone = engine.create_zone(
        ZoneConfig::builder().max_tabs(1).build().expect("zone cfg"),
        zone_services,
        None,
    )?;
    let tab = zone
        .create_tab(
            TabDefaults {
                url: None,
                title: None,
                viewport: Some(Viewport::new(0, 0, 800, 600)),
            },
            None,
        )
        .await
        .expect("create tab");

    tab.navigate(base_url).await.expect("navigate");

    // Collect events until navigation finishes or 15 s timeout.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    let mut c = Collected::default();

    loop {
        tokio::select! {
            Ok(ev) = events.recv() => {
                match ev {
                    EngineEvent::Navigation { event, .. } => match event {
                        NavigationEvent::Started { .. } => {
                            c.nav_started = true;
                        }
                        NavigationEvent::Finished { .. } => {
                            c.nav_finished = true;
                            break;
                        }
                        NavigationEvent::Failed { error, .. } => {
                            c.nav_error = Some(error.to_string());
                            break;
                        }
                        NavigationEvent::Cancelled { reason, .. } => {
                            c.nav_error = Some(format!("cancelled: {reason:?}"));
                            break;
                        }
                        NavigationEvent::DecisionRequired { nav_id, meta, decision_token } => {
                            decide(tab.clone(), nav_id, meta, decision_token).await;
                        }
                        _ => {}
                    },
                    EngineEvent::Resource { event: ResourceEvent::Started { url, .. }, .. } => {
                        c.resources_started.push(url);
                    }
                    _ => {}
                }
            }
            _ = tokio::time::sleep_until(deadline) => {
                c.nav_error = Some("timed out after 15s".into());
                break;
            }
        }
    }

    // Drain any remaining resource events that arrived around the same time.
    tokio::time::sleep(Duration::from_millis(500)).await;
    while let Ok(ev) = events.try_recv() {
        if let EngineEvent::Resource {
            event: ResourceEvent::Started { url, .. },
            ..
        } = ev
        {
            c.resources_started.push(url);
        }
    }

    stop_server.cancel();
    tokio::time::sleep(Duration::from_millis(100)).await;
    engine.shutdown().await?;
    if let Some(h) = engine_join {
        let _ = h.await;
    }

    // ── Report ────────────────────────────────────────────────────────────────
    println!("Checks:");
    let mut all_pass = true;

    all_pass &= check("Navigation started", c.nav_started);
    all_pass &= check(
        "Navigation finished without error",
        c.nav_finished && c.nav_error.is_none(),
    );
    if let Some(err) = &c.nav_error {
        println!("         → {err}");
    }

    let css = c.resources_started.iter().any(|u| u.contains("/style.css"));
    let js = c.resources_started.iter().any(|u| u.contains("/app.js"));
    let img = c.resources_started.iter().any(|u| u.contains("/pixel.gif"));

    all_pass &= check("CSS sub-resource discovered  (/style.css)", css);
    all_pass &= check("JS  sub-resource discovered  (/app.js)", js);
    all_pass &= check("Img sub-resource discovered  (/pixel.gif)", img);

    println!();
    if all_pass {
        println!("All checks passed.");
    } else {
        println!("One or more checks FAILED.");
        println!("Resources seen: {:#?}", c.resources_started);
        std::process::exit(1);
    }

    Ok(())
}
