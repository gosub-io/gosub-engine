//! Lightweight HTTP metrics server.
//!
//! Call [`start`] once at engine startup to expose timing data over HTTP.
//!
//! # Endpoints
//!
//! | Method | Path              | Description                                          |
//! |--------|-------------------|------------------------------------------------------|
//! | GET    | `/metrics`        | JSON snapshot of all timing namespaces               |
//! | GET    | `/metrics/reset`  | Clear all timing counters                            |
//! | GET    | `/events`         | The telemetry firehose, streamed as NDJSON           |
//! | GET    | `/renderers`      | Renderer processes (none yet; reserved for the viewer)  |
//! | GET    | `/health`         | Liveness probe (`{"status":"ok"}`)                   |

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// Spawn the metrics HTTP server on `127.0.0.1:{port}` in a background Tokio task.
///
/// The function returns immediately; the server runs until the process exits.
pub fn start(port: u16) {
    tokio::spawn(async move {
        if let Err(e) = serve(port).await {
            log::error!("[metrics] server stopped: {e}");
        }
    });
    log::info!("[metrics] server starting on http://127.0.0.1:{port}/metrics");
}

async fn serve(port: u16) -> std::io::Result<()> {
    let listener = TcpListener::bind(format!("127.0.0.1:{port}")).await?;
    log::info!("[metrics] listening on http://127.0.0.1:{port}");
    loop {
        let (stream, _addr) = listener.accept().await?;
        tokio::spawn(handle(stream));
    }
}

async fn handle(mut stream: TcpStream) {
    let mut buf = vec![0u8; 2048];
    let n = stream.read(&mut buf).await.unwrap_or(0);
    let req = std::str::from_utf8(&buf[..n]).unwrap_or("");
    let first_line = req.lines().next().unwrap_or("");

    if first_line.starts_with("GET /events") {
        stream_events(stream).await;
        return;
    }

    // A mutation on a GET is a `<img>` tag away; POST only.
    let (code, phrase, body) = if first_line.starts_with("POST /metrics/reset") {
        gosub_shared::timing::reset_stats();
        (200u16, "OK", r#"{"status":"reset"}"#.to_string())
    } else if first_line.starts_with("GET /metrics") || first_line.starts_with("HEAD /metrics") {
        (200, "OK", build_metrics_json())
    } else if first_line.starts_with("GET /renderers") {
        (200, "OK", r#"{"renderers":[]}"#.to_string())
    } else if first_line.starts_with("GET /health") {
        (200, "OK", r#"{"status":"ok"}"#.to_string())
    } else {
        (404, "Not Found", r#"{"error":"not found"}"#.to_string())
    };

    // HEAD gets the same headers (including Content-Length) but no body.
    let payload = if first_line.starts_with("HEAD ") {
        ""
    } else {
        body.as_str()
    };
    let response = format!(
        "HTTP/1.1 {code} {phrase}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
}

/// The firehose, one JSON object per line, for as long as the client reads.
/// Subscribing is what switches emission on, so the stream starts with the
/// first event after the request. A client that reads too slowly is told
/// what it missed rather than silently skipped past.
async fn stream_events(mut stream: TcpStream) {
    use tokio::sync::broadcast::error::RecvError;

    let mut events = crate::telemetry::subscribe();
    let head =
        "HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n";
    if stream.write_all(head.as_bytes()).await.is_err() {
        return;
    }
    loop {
        let line = match events.recv().await {
            Ok(event) => serde_json::to_string(&*event).unwrap_or_default(),
            Err(RecvError::Lagged(dropped)) => {
                format!(r#"{{"source":"broker","kind":"telemetry.lagged","data":{{"dropped":{dropped}}}}}"#)
            }
            Err(RecvError::Closed) => return,
        };
        if stream.write_all(line.as_bytes()).await.is_err() || stream.write_all(b"\n").await.is_err() {
            return;
        }
    }
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
