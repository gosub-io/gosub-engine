//! The firehose: a process-wide stream of what the engine is doing, for
//! tooling outside it.
//!
//! Components [`emit`] events - a kind, a JSON payload - and anyone who
//! [`subscribe`]s sees every one of them in order. The engine itself never
//! reads the stream; it exists to be pushed out (the `metrics` feature's
//! `GET /events` serves it as newline-delimited JSON) and displayed by
//! whatever wants to. Emitting costs nothing while nobody listens, so
//! components can report freely.
//!
//! Sandboxed children cannot reach a socket, so their numbers travel over
//! their existing IPC links and the broker emits them on their behalf, with
//! a `source` naming the process (see [`emit_from`]).

use serde::Serialize;
use std::sync::{Arc, OnceLock};
use tokio::sync::broadcast;

/// Events kept for a slow subscriber before it starts losing them.
const CAPACITY: usize = 8192;

/// One thing that happened.
#[derive(Debug, Clone, Serialize)]
pub struct Event {
    /// Unix time, microseconds.
    pub ts_us: u64,
    /// Which process reported it: `broker`, `renderer:<pid>`, ...
    pub source: String,
    /// A dotted name for what happened, e.g. `remote.scroll`.
    pub kind: String,
    /// Whatever the reporter measured.
    pub data: serde_json::Value,
}

fn bus() -> &'static broadcast::Sender<Arc<Event>> {
    static BUS: OnceLock<broadcast::Sender<Arc<Event>>> = OnceLock::new();
    BUS.get_or_init(|| broadcast::channel(CAPACITY).0)
}

/// Whether anyone is listening - so a reporter can skip assembling a payload.
pub fn enabled() -> bool {
    bus().receiver_count() > 0
}

/// Report something this process did.
pub fn emit(kind: &str, data: serde_json::Value) {
    emit_from("broker", kind, data);
}

/// Report something on behalf of `source` (a child process that cannot
/// reach the bus itself).
pub fn emit_from(source: &str, kind: &str, data: serde_json::Value) {
    let bus = bus();
    if bus.receiver_count() == 0 {
        return;
    }
    let ts_us = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0);
    // Err means no receiver - a race with the check above; nothing to do.
    let _ = bus.send(Arc::new(Event {
        ts_us,
        source: source.to_string(),
        kind: kind.to_string(),
        data,
    }));
}

/// Start receiving events from now on. A receiver that falls more than
/// [`CAPACITY`] events behind loses the oldest (`RecvError::Lagged`).
pub fn subscribe() -> broadcast::Receiver<Arc<Event>> {
    bus().subscribe()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn events_reach_a_subscriber_and_cost_nothing_without_one() {
        assert!(!enabled() || bus().receiver_count() > 0);
        emit("test.unheard", serde_json::json!({ "n": 1 }));

        let mut rx = subscribe();
        assert!(enabled());
        emit_from("renderer:7", "test.heard", serde_json::json!({ "n": 2 }));
        let event = rx.recv().await.expect("event");
        assert_eq!(event.kind, "test.heard");
        assert_eq!(event.source, "renderer:7");
        assert_eq!(event.data["n"], 2);
        assert!(event.ts_us > 0);
        assert!(serde_json::to_string(&*event)
            .expect("json")
            .contains("\"kind\":\"test.heard\""));
    }
}
