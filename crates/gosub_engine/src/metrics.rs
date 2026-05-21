//! Lightweight HTTP metrics server.
//!
//! Call [`start`] once at engine startup to expose timing data over HTTP.
//!
//! # Endpoints
//!
//! | Method | Path              | Description                            |
//! |--------|-------------------|----------------------------------------|
//! | GET    | `/metrics`        | JSON snapshot of all timing namespaces |
//! | GET    | `/metrics/reset`  | Clear all timing counters              |
//! | GET    | `/health`         | Liveness probe (`{"status":"ok"}`)     |

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

    let (code, phrase, body) = if first_line.starts_with("GET /metrics/reset") {
        gosub_shared::timing::reset_stats();
        (200u16, "OK", r#"{"status":"reset"}"#.to_string())
    } else if first_line.starts_with("GET /metrics") || first_line.starts_with("HEAD /metrics") {
        (200, "OK", build_metrics_json())
    } else if first_line.starts_with("GET /health") {
        (200, "OK", r#"{"status":"ok"}"#.to_string())
    } else {
        (404, "Not Found", r#"{"error":"not found"}"#.to_string())
    };

    let response = format!(
        "HTTP/1.1 {code} {phrase}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
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
