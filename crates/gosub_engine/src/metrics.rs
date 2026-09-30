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

    /// An `/events` client that leaves while no events flow ends its stream,
    /// and with it the subscription that keeps telemetry emission on.
    #[tokio::test]
    async fn an_idle_events_client_that_hangs_up_ends_its_stream() {
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
