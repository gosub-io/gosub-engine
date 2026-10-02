//! Lightweight HTTP metrics server.
//!
//! Call [`start`] once at engine startup to expose timing data over HTTP.
//!
//! # Endpoints
//!
//! | Method | Path              | Description                                          |
//! |--------|-------------------|------------------------------------------------------|
//! | GET    | `/`               | The telemetry viewer (open it in a browser)          |
//! | GET    | `/metrics`        | JSON snapshot of all timing namespaces               |
//! | POST   | `/metrics/reset`  | Clear all timing counters                            |
//! | GET    | `/events`         | The telemetry firehose, streamed as NDJSON           |
//! | GET    | `/renderers`      | Resident renderer processes and their tabs           |
//! | GET    | `/health`         | Liveness probe (`{"status":"ok"}`)                   |
//!
//! The viewer is served from here so its requests to `/events` and
//! `/renderers` are same-origin: opened as a file, the browser would block
//! them, and a CORS header would let any page in the browser read telemetry.

use crate::engine::EngineContext;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// The telemetry viewer page, served at `/`. A single static page, no build step.
const VIEWER: &str = include_str!("metrics/viewer.html");

/// Spawn the metrics HTTP server on `127.0.0.1:{port}` in a background Tokio task.
///
/// The function returns immediately; the server runs until the process exits.
pub fn start(port: u16, context: Arc<EngineContext>) {
    tokio::spawn(async move {
        if let Err(e) = serve(port, context).await {
            log::error!("[metrics] server stopped: {e}");
        }
    });
    log::info!("[metrics] server starting on http://127.0.0.1:{port}/metrics");
}

async fn serve(port: u16, context: Arc<EngineContext>) -> std::io::Result<()> {
    let listener = TcpListener::bind(format!("127.0.0.1:{port}")).await?;
    log::info!("[metrics] listening on http://127.0.0.1:{port} (telemetry viewer at /)");
    loop {
        let (stream, _addr) = listener.accept().await?;
        tokio::spawn(handle(stream, Arc::clone(&context)));
    }
}

/// The value of header `name` in `req`, if present.
fn header<'a>(req: &'a str, name: &str) -> Option<&'a str> {
    req.lines()
        .skip(1)
        .take_while(|l| !l.is_empty())
        .find_map(|l| l.split_once(':').filter(|(k, _)| k.eq_ignore_ascii_case(name)))
        .map(|(_, v)| v.trim())
}

/// Whether `host` (`host` or `host:port`, IPv6 in brackets) is a loopback name.
fn is_loopback_host(host: &str) -> bool {
    let name = if let Some(rest) = host.strip_prefix('[') {
        rest.split_once(']').map(|(n, _)| n).unwrap_or(rest)
    } else {
        host.rsplit_once(':').map_or(host, |(n, _)| n)
    };
    ["127.0.0.1", "localhost", "::1"]
        .iter()
        .any(|local| name.eq_ignore_ascii_case(local))
}

/// Whether the request's `Host` names this server as a local address. The
/// listener is on 127.0.0.1, which keeps the network out but not a web page:
/// a page on `attacker.example` whose DNS answer is switched to 127.0.0.1
/// after it loaded reaches this port same-origin and reads the firehose -
/// every URL the browser fetches. The `Host` it sends is its own name;
/// refusing anything but the loopback names closes that read. It does not
/// close a cross-site *write*: a form on any page may POST here, and the
/// browser then sends this server's own name as `Host` - see
/// [`origin_is_local`] for the mutation.
fn host_is_local(req: &str) -> bool {
    header(req, "host").is_some_and(is_loopback_host)
}

/// Whether a mutation may run: its `Origin`, when there is one, must be this
/// server's own (the viewer at `/`). A browser sends `Origin` on every POST,
/// a cross-site form's being the other page's, or `null`; a request without
/// one did not come from a browser (`examples/metrics_cli.rs`, curl).
fn origin_is_local(req: &str) -> bool {
    match header(req, "origin") {
        None => true,
        Some(origin) => origin
            .strip_prefix("http://")
            .is_some_and(|rest| is_loopback_host(rest.trim_end_matches('/'))),
    }
}

async fn handle(mut stream: TcpStream, context: Arc<EngineContext>) {
    let mut buf = vec![0u8; 2048];
    let n = stream.read(&mut buf).await.unwrap_or(0);
    let req = std::str::from_utf8(&buf[..n]).unwrap_or("");
    let first_line = req.lines().next().unwrap_or("");

    if !host_is_local(req) {
        let body = r#"{"error":"this server answers only to its loopback name"}"#;
        let response = format!(
            "HTTP/1.1 403 Forbidden\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = stream.write_all(response.as_bytes()).await;
        return;
    }

    if first_line.starts_with("GET /events") {
        stream_events(stream).await;
        return;
    }

    const JSON: &str = "application/json";
    // A mutation on a GET is a `<img>` tag away, so POST only - and a POST
    // is a cross-site form away, so its `Origin` must be ours.
    let (code, phrase, content_type, body) = if first_line.starts_with("POST /metrics/reset") {
        if origin_is_local(req) {
            gosub_shared::timing::reset_stats();
            (200u16, "OK", JSON, r#"{"status":"reset"}"#.to_string())
        } else {
            (
                403,
                "Forbidden",
                JSON,
                r#"{"error":"a reset must come from this server's own page"}"#.to_string(),
            )
        }
    } else if first_line.starts_with("GET / ") || first_line.starts_with("HEAD / ") {
        (200, "OK", "text/html; charset=utf-8", VIEWER.to_string())
    } else if first_line.starts_with("GET /metrics") || first_line.starts_with("HEAD /metrics") {
        (200, "OK", JSON, build_metrics_json())
    } else if first_line.starts_with("GET /renderers") {
        (200, "OK", JSON, build_renderers_json(&context))
    } else if first_line.starts_with("GET /health") {
        (200, "OK", JSON, r#"{"status":"ok"}"#.to_string())
    } else {
        (404, "Not Found", JSON, r#"{"error":"not found"}"#.to_string())
    };

    // HEAD gets the same headers (including Content-Length) but no body.
    let payload = if first_line.starts_with("HEAD ") {
        ""
    } else {
        body.as_str()
    };
    let response = format!(
        "HTTP/1.1 {code} {phrase}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
}

/// The firehose, one JSON object per line, for as long as the client reads.
/// Subscribing is what switches emission on, so the stream starts with the
/// first event after the request. A client that reads too slowly is told
/// what it missed rather than silently skipped past.
///
/// A client that hangs up while nothing is happening must still be noticed,
/// or an idle stream keeps its subscription, and with it emission, alive
/// forever. A read EOF does not say so (the client may only have closed its
/// sending side and still be reading), so an idle stream writes an empty line
/// every [`HEARTBEAT`] - NDJSON readers skip it - and a write that fails is
/// the hang-up.
async fn stream_events(stream: TcpStream) {
    stream_events_with(stream, HEARTBEAT).await
}

/// How often an idle `/events` stream checks, by writing, that its client is there.
const HEARTBEAT: std::time::Duration = std::time::Duration::from_secs(15);

async fn stream_events_with(mut stream: TcpStream, heartbeat: std::time::Duration) {
    use tokio::sync::broadcast::error::RecvError;

    let mut events = crate::telemetry::subscribe();
    let head =
        "HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n";
    if stream.write_all(head.as_bytes()).await.is_err() {
        return;
    }
    let (mut reader, mut writer) = stream.split();
    let mut discard = [0u8; 256];
    let mut reading = true;
    let mut beat = tokio::time::interval(heartbeat);
    beat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    beat.tick().await; // the first tick is immediate
    loop {
        let line = tokio::select! {
            received = events.recv() => match received {
                Ok(event) => serde_json::to_string(&*event).unwrap_or_default(),
                Err(RecvError::Lagged(dropped)) => {
                    format!(r#"{{"source":"broker","kind":"telemetry.lagged","data":{{"dropped":{dropped}}}}}"#)
                }
                Err(RecvError::Closed) => return,
            },
            read = reader.read(&mut discard), if reading => match read {
                // The client closed its sending side; it may still be reading.
                Ok(0) => {
                    reading = false;
                    continue;
                }
                // A broken socket: the client is gone.
                Err(_) => return,
                // Nothing a client sends after its request means anything here.
                Ok(_) => continue,
            },
            _ = beat.tick() => String::new(),
        };
        if writer.write_all(line.as_bytes()).await.is_err() || writer.write_all(b"\n").await.is_err() {
            return;
        }
    }
}

/// The resident renderer pool: one entry per process, with its tab count.
fn build_renderers_json(context: &EngineContext) -> String {
    use serde_json::json;

    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    let renderers: Vec<serde_json::Value> = context
        .renderer_pool
        .get()
        .map(|pool| {
            pool.snapshot()
                .into_iter()
                .map(|r| {
                    json!({
                        "pid": r.pid,
                        "zone": r.key.zone.to_string(),
                        "site": r.key.site,
                        "tabs": r.tabs,
                        "rss_kb": r.rss_kb,
                        "data_kb": r.data_kb,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    #[cfg(not(all(feature = "process-isolation", target_os = "linux")))]
    let renderers: Vec<serde_json::Value> = {
        let _ = context;
        Vec::new()
    };

    serde_json::to_string_pretty(&json!({ "renderers": renderers })).unwrap_or_else(|_| "{}".to_string())
}

fn build_metrics_json() -> String {
    use gosub_shared::timing::snapshot_stats;
    use serde_json::{json, Map, Value};

    let mut map = Map::new();
    for s in snapshot_stats() {
        map.insert(
            s.namespace.clone(),
            json!({
                "count":    s.count,
                "total_us": s.total_us,
                "min_us":   s.min_us,
                "max_us":   s.max_us,
                "avg_us":   s.avg_us,
                "p50_us":   s.p50_us,
                "p75_us":   s.p75_us,
                "p95_us":   s.p95_us,
                "p99_us":   s.p99_us,
            }),
        );
    }

    serde_json::to_string_pretty(&json!({ "namespaces": Value::Object(map) })).unwrap_or_else(|_| "{}".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// One `/events` stream with a short heartbeat, and a client already past
    /// the response head.
    /// The telemetry bus is process-wide: `telemetry::enabled()` is true while
    /// any test's stream holds a subscription, so the tests that assert on it
    /// run one at a time.
    static ONE_AT_A_TIME: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    async fn one_stream() -> (TcpStream, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            stream_events_with(stream, Duration::from_millis(100)).await;
        });
        let mut client = TcpStream::connect(addr).await.unwrap();
        let mut head = [0u8; 256];
        let _ = client.read(&mut head).await.unwrap();
        assert!(crate::telemetry::enabled(), "the stream subscribes while it runs");
        (client, server)
    }

    /// The viewer comes from the metrics server itself, so its requests to
    /// `/events` and `/renderers` are same-origin.
    #[tokio::test]
    async fn the_viewer_is_served_at_the_root() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (event_tx, _events) = tokio::sync::broadcast::channel(16);
            let context = Arc::new(EngineContext {
                event_tx,
                ..Default::default()
            });
            handle(stream, context).await;
        });
        let mut client = TcpStream::connect(addr).await.unwrap();
        client
            .write_all(b"GET / HTTP/1.1\r\nHost: 127.0.0.1:9090\r\n\r\n")
            .await
            .unwrap();
        let mut response = Vec::new();
        client.read_to_end(&mut response).await.unwrap();
        let response = String::from_utf8_lossy(&response);
        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "{response:.80}");
        assert!(response.contains("Content-Type: text/html"), "not served as HTML");
        assert!(response.contains("<title>Gosub telemetry</title>"), "not the viewer");
    }

    /// A request under any other name is a page that reached 127.0.0.1 by
    /// DNS rebinding (its `Host` is its own name), or a proxy: refused.
    #[tokio::test]
    async fn a_request_for_another_host_name_is_refused() {
        for (host, allowed) in [
            ("attacker.example:9090", false),
            ("attacker.example", false),
            ("127.0.0.1.attacker.example", false),
            ("127.0.0.1:9090", true),
            ("localhost:9090", true),
            ("LOCALHOST", true),
            ("[::1]:9090", true),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            tokio::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                let (event_tx, _events) = tokio::sync::broadcast::channel(16);
                let context = Arc::new(EngineContext {
                    event_tx,
                    ..Default::default()
                });
                handle(stream, context).await;
            });
            let mut client = TcpStream::connect(addr).await.unwrap();
            client
                .write_all(format!("GET /health HTTP/1.1\r\nHost: {host}\r\n\r\n").as_bytes())
                .await
                .unwrap();
            let mut response = Vec::new();
            client.read_to_end(&mut response).await.unwrap();
            let response = String::from_utf8_lossy(&response);
            let expected = if allowed {
                "HTTP/1.1 200 OK"
            } else {
                "HTTP/1.1 403 Forbidden"
            };
            assert!(response.starts_with(expected), "Host {host}: {response:.60}");
        }
        // No Host at all is not local either.
        assert!(!host_is_local("GET /health HTTP/1.1\r\n\r\n"));
    }

    /// A cross-site form POSTs with this server's name as `Host` and its own
    /// page as `Origin`: refused. The viewer's own origin, or no `Origin` at
    /// all (not a browser), may reset.
    #[tokio::test]
    async fn a_reset_from_another_origin_is_refused() {
        for (origin, allowed) in [
            (Some("https://attacker.example"), false),
            (Some("null"), false),
            (Some("http://127.0.0.1.attacker.example"), false),
            (Some("http://127.0.0.1:9090"), true),
            (Some("http://localhost:9090"), true),
            (None, true),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            tokio::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                let (event_tx, _events) = tokio::sync::broadcast::channel(16);
                let context = Arc::new(EngineContext {
                    event_tx,
                    ..Default::default()
                });
                handle(stream, context).await;
            });
            let mut client = TcpStream::connect(addr).await.unwrap();
            let origin_line = origin.map_or(String::new(), |o| format!("Origin: {o}\r\n"));
            client
                .write_all(
                    format!(
                        "POST /metrics/reset HTTP/1.1\r\nHost: 127.0.0.1:9090\r\n{origin_line}Content-Length: 0\r\n\r\n"
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            let mut response = Vec::new();
            client.read_to_end(&mut response).await.unwrap();
            let response = String::from_utf8_lossy(&response);
            let expected = if allowed {
                "HTTP/1.1 200 OK"
            } else {
                "HTTP/1.1 403 Forbidden"
            };
            assert!(response.starts_with(expected), "Origin {origin:?}: {response:.60}");
        }
    }

    /// An `/events` client that leaves while no events flow ends its stream,
    /// and with it the subscription that keeps telemetry emission on.
    #[tokio::test]
    async fn an_idle_events_client_that_hangs_up_ends_its_stream() {
        let _one = ONE_AT_A_TIME.lock().await;
        let (client, server) = one_stream().await;
        drop(client);

        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .expect("an idle stream whose client left must end")
            .unwrap();
        assert!(!crate::telemetry::enabled(), "its subscription went with it");
    }

    /// A client that closes only its sending side is still reading: its
    /// stream goes on, and the next event reaches it.
    #[tokio::test]
    async fn a_client_that_only_stops_sending_still_gets_events() {
        let _one = ONE_AT_A_TIME.lock().await;
        let (mut client, server) = one_stream().await;
        client.shutdown().await.unwrap();
        // Long enough for the server to read the EOF, and for heartbeats.
        tokio::time::sleep(Duration::from_millis(300)).await;
        crate::telemetry::emit("test.half_closed", serde_json::json!({}));

        let mut seen = Vec::new();
        let got = tokio::time::timeout(Duration::from_secs(2), async {
            let mut buf = [0u8; 4096];
            loop {
                let n = client.read(&mut buf).await.unwrap();
                if n == 0 {
                    return false;
                }
                seen.extend_from_slice(&buf[..n]);
                if String::from_utf8_lossy(&seen).contains("test.half_closed") {
                    return true;
                }
            }
        })
        .await;
        assert_eq!(
            got,
            Ok(true),
            "the event never arrived: {:?}",
            String::from_utf8_lossy(&seen)
        );
        server.abort();
    }
}
