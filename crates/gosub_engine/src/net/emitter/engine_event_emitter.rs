use crate::engine::events::{CancelReason, FailureKind, ResourceEvent};
use crate::engine::types::{EventChannel, RequestId};
use crate::events::EngineEvent;
use crate::net::emitter::NetObserver;
use crate::net::events::NetEvent;
use crate::net::req_ref_tracker::{RequestReference, REF_REGISTRY};
use crate::net::types::{Initiator, NetError, ResourceKind};
use crate::tab::TabId;
use gosub_sonar::{TransportError, TransportErrorKind};
use std::sync::Arc;

/// Converts NetEvents into EngineEvents and send them over to the event_tx channel back to the UA
pub struct EngineEventEmitter {
    /// The tab ID to route the event to
    tab_id: TabId,
    /// The request ID to correlate the event with
    req_id: RequestId,
    /// The request reference to correlate the event with
    reference: RequestReference,
    /// The channel to send the events to
    event_tx: EventChannel,
    /// The resource kind (e.g., Document, Script, Image, etc.)
    kind: ResourceKind,
    /// The initiator of the request
    initiator: Initiator,
    /// Bytes at the last forwarded progress event (progress arrives per read chunk from
    /// the transport, which is too chatty for the event bus).
    last_progress: std::sync::atomic::AtomicU64,
    /// Whether this request has already been reported as failed. See [`Self::report_failure`].
    failure_reported: std::sync::atomic::AtomicBool,
}

impl EngineEventEmitter {
    #[must_use]
    pub fn new(
        // Normally we don't expose high-level tab IDs to the net layer, but we need it here to
        // route events back to the right tab. We retrieve this IDs from the resource_request_map
        tab_id: TabId,
        req_id: RequestId,
        reference: RequestReference,
        event_tx: EventChannel,
        kind: ResourceKind,
        initiator: Initiator,
    ) -> Self {
        Self {
            tab_id,
            req_id,
            reference,
            event_tx,
            kind,
            initiator,
            last_progress: std::sync::atomic::AtomicU64::new(0),
            failure_reported: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Report a failure once, keeping the first cause that arrives.
    ///
    /// The network stack names a specific cause first (`Blocked`, `TlsFailed`) and follows
    /// it with a terminal `Failed` -- except when it does not: a request rejected by policy
    /// *before it is sent* never reaches the code that emits the terminal event, so `Blocked`
    /// is all there is. Reporting on both events would emit two failures for one request;
    /// reporting only on the terminal one drops every pre-flight rejection on the floor.
    /// First one wins settles both, and the first is always the more specific.
    fn report_failure(&self, url: String, kind: FailureKind, error: anyhow::Error) {
        use std::sync::atomic::Ordering;
        if self.failure_reported.swap(true, Ordering::Relaxed) {
            return;
        }
        REF_REGISTRY.forget_request(self.req_id);
        self.emit(ResourceEvent::Failed {
            request_id: self.req_id,
            reference: self.reference,
            url,
            kind,
            error: Arc::new(error),
        });
    }

    /// Forward at most one progress event per `STEP` bytes received (always forwarding
    /// the final one that reaches `expected`).
    fn should_report_progress(&self, received: u64, expected: Option<u64>) -> bool {
        use std::sync::atomic::Ordering;
        const STEP: u64 = 64 * 1024;
        let last = self.last_progress.load(Ordering::Relaxed);
        if received.saturating_sub(last) >= STEP || Some(received) == expected {
            self.last_progress.store(received, Ordering::Relaxed);
            true
        } else {
            false
        }
    }

    /// Emit a resource event
    fn emit(&self, ev: ResourceEvent) {
        let _ = self.event_tx.send(EngineEvent::Resource {
            tab_id: self.tab_id,
            event: ev,
        });
    }
}

impl NetObserver for EngineEventEmitter {
    /// Policy for what is worth capturing lives in one place; see
    /// [`crate::net::emitter::should_capture_body`].
    fn body_capture_limit(&self, headers: &http::HeaderMap, content_length: Option<u64>) -> Option<usize> {
        crate::net::emitter::should_capture_body(headers, content_length)
    }

    fn on_event(&self, ev: NetEvent) {
        match ev {
            NetEvent::DnsResolved { host, elapsed, .. } => {
                self.emit(ResourceEvent::DnsResolved {
                    request_id: self.req_id,
                    reference: self.reference,
                    host,
                    elapsed_us: elapsed.as_micros() as u64,
                });
            }
            NetEvent::Connected { elapsed } => {
                self.emit(ResourceEvent::Connected {
                    request_id: self.req_id,
                    reference: self.reference,
                    elapsed_us: elapsed.as_micros() as u64,
                });
            }
            NetEvent::RequestSent { url, method, headers } => {
                self.emit(ResourceEvent::RequestSent {
                    request_id: self.req_id,
                    reference: self.reference,
                    url: url.to_string(),
                    method: method.to_string(),
                    // Names always, values only where they are safe to pass on: this event
                    // is an API, and it goes wherever the embedder puts it.
                    headers: headers
                        .iter()
                        .map(|(k, v)| {
                            let name = k.to_string();
                            let value = crate::net::emitter::header_value(&name, v.to_str().unwrap_or(""));
                            (name, value)
                        })
                        .collect(),
                });
            }
            NetEvent::BodyPreview { url, body, truncated } => {
                self.emit(ResourceEvent::BodyPreview {
                    request_id: self.req_id,
                    reference: self.reference,
                    url: url.to_string(),
                    body,
                    truncated,
                });
            }
            NetEvent::Started { url } => {
                self.emit(ResourceEvent::Started {
                    request_id: self.req_id,
                    reference: self.reference,
                    url: url.to_string(),
                    kind: self.kind,
                    initiator: self.initiator,
                });
            }
            NetEvent::Redirected { from, to, status } => {
                self.emit(ResourceEvent::Redirected {
                    request_id: self.req_id,
                    reference: self.reference,
                    from: from.to_string(),
                    to: to.to_string(),
                    status,
                });
            }
            NetEvent::ResponseHeaders { url, status, headers } => {
                self.emit(ResourceEvent::Headers {
                    request_id: self.req_id,
                    reference: self.reference,
                    url: url.to_string(),
                    status,
                    content_length: headers
                        .get(http::header::CONTENT_LENGTH)
                        .and_then(|v| v.to_str().ok())
                        .and_then(|s| s.parse::<u64>().ok()),
                    content_type: headers
                        .get(http::header::CONTENT_TYPE)
                        .and_then(|v| v.to_str().ok())
                        .map(|s| s.to_string()),
                    headers: headers
                        .iter()
                        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
                        .collect(),
                });
            }
            NetEvent::Progress {
                received_bytes,
                expected_length,
                elapsed,
            } => {
                if !self.should_report_progress(received_bytes, expected_length) {
                    return;
                }
                match self.reference {
                    // Document fetch of a navigation: the shell's load-progress signal.
                    RequestReference::Navigation(nav_id) => {
                        let _ = self.event_tx.send(EngineEvent::Navigation {
                            tab_id: self.tab_id,
                            event: crate::engine::events::NavigationEvent::Progress {
                                nav_id,
                                received_bytes,
                                expected_length,
                                elapsed,
                            },
                        });
                    }
                    // A download: granular per-chunk progress for the shell's downloads UI.
                    RequestReference::Download(id) => {
                        let _ = self.event_tx.send(EngineEvent::DownloadProgress {
                            tab_id: self.tab_id,
                            id: crate::engine::events::DownloadId(id),
                            received_bytes,
                            total_bytes: expected_length,
                        });
                        return; // not a page resource
                    }
                    _ => {}
                }
                self.emit(ResourceEvent::Progress {
                    request_id: self.req_id,
                    reference: self.reference,
                    received_bytes,
                    expected_length,
                    elapsed,
                });
            }
            NetEvent::Finished {
                url,
                received_bytes,
                elapsed,
            } => {
                REF_REGISTRY.forget_request(self.req_id);
                self.emit(ResourceEvent::Finished {
                    request_id: self.req_id,
                    reference: self.reference,
                    url,
                    received_bytes,
                    elapsed: Some(elapsed),
                });
            }
            NetEvent::Blocked { url, reason } => self.report_failure(
                url.to_string(),
                FailureKind::Blocked,
                anyhow::anyhow!("blocked: {reason}"),
            ),
            NetEvent::TlsFailed { url, error } => self.report_failure(
                url.to_string(),
                FailureKind::Tls,
                anyhow::anyhow!(
                    "TLS handshake with {} failed: {:?} ({})",
                    error.host,
                    error.kind,
                    error.message
                ),
            ),
            NetEvent::Failed { url, error } => {
                self.report_failure(url.to_string(), classify(&error), error);
            }
            NetEvent::Cancelled { url, reason } => {
                REF_REGISTRY.forget_request(self.req_id);
                self.emit(ResourceEvent::Cancelled {
                    request_id: self.req_id,
                    reference: self.reference,
                    url: url.to_string(),
                    reason: CancelReason::Custom(reason.to_string()),
                });
            }

            NetEvent::Io { .. } => {
                // Do nothing
            }
            NetEvent::Warning { .. } => {
                // Do nothing
            }
            // Timing-only events (DnsResolved, Connected) and anything sonar adds
            // later carry nothing the shell needs; the timing emitter reads them.
            _ => {}
        }
    }
}

/// Read the network stack's own typed error rather than its message.
///
/// The error arrives wrapped in `anyhow`, but the `NetError` underneath is intact and
/// already says what went wrong. Matching on it keeps this honest, where matching on the
/// text of a message would quietly rot the first time one is reworded.
fn classify(error: &anyhow::Error) -> FailureKind {
    let Some(net) = error.downcast_ref::<NetError>() else {
        return FailureKind::Other;
    };
    match net {
        NetError::Blocked { .. } => FailureKind::Blocked,
        NetError::Tls(_) => FailureKind::Tls,
        NetError::Timeout(_) => FailureKind::Timeout,
        NetError::Redirect(_) => FailureKind::Redirect,
        NetError::Cancelled(_) => FailureKind::Cancelled,
        NetError::Io(_) => FailureKind::Transfer,
        // Sonar has already separated these. A `send()` that never got a connection and a
        // body that stopped mid-stream are both transport failures, and reporting the first
        // as a broken transfer sends you looking at the server when the problem is the
        // address.
        NetError::Transport(e) => from_transport(e),
        NetError::Read(_) => FailureKind::Transfer,
        NetError::Other(_) => FailureKind::Other,
    }
}

/// Map a sonar transport failure onto the kind the shell reports.
fn from_transport(error: &TransportError) -> FailureKind {
    match error.kind {
        TransportErrorKind::Connect => FailureKind::Connect,
        TransportErrorKind::Timeout => FailureKind::Timeout,
        TransportErrorKind::Redirect => FailureKind::Redirect,
        TransportErrorKind::Body | TransportErrorKind::Decode => FailureKind::Transfer,
        // `Request` and `Builder` mean nothing was sent, and whatever sonar learns to tell
        // apart later lands here first. Neither says anything about the network.
        _ => FailureKind::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gosub_sonar::net::types::BlockReason;

    /// The whole point of `classify` is that the cause survives the trip through
    /// `anyhow`, including the extra context a caller may have attached on the way.
    #[test]
    fn a_typed_cause_survives_the_anyhow_wrapper() {
        let err = anyhow::Error::from(NetError::Blocked {
            reason: BlockReason::MixedContent,
            url: url::Url::parse("http://example.com/x.css").unwrap(),
        })
        .context("loading stylesheet");

        assert_eq!(classify(&err), FailureKind::Blocked);
    }

    #[test]
    fn a_timeout_is_not_reported_as_a_transfer_failure() {
        let err = anyhow::Error::from(NetError::Timeout("no response in 30s".into()));
        assert_eq!(classify(&err), FailureKind::Timeout);
    }

    #[test]
    fn a_broken_transfer_is_distinct_from_a_refusal() {
        let err = anyhow::Error::from(NetError::Read(Arc::new(anyhow::anyhow!(
            "connection reset while reading body"
        ))));
        assert_eq!(classify(&err), FailureKind::Transfer);
    }

    /// Build an emitter wired to a channel the test can read back.
    fn emitter() -> (EngineEventEmitter, tokio::sync::broadcast::Receiver<EngineEvent>) {
        let (tx, rx) = tokio::sync::broadcast::channel(16);
        let emitter = EngineEventEmitter::new(
            TabId::new(),
            RequestId::new(),
            RequestReference::Document(1),
            tx,
            ResourceKind::Stylesheet,
            Initiator::Parser,
        );
        (emitter, rx)
    }

    /// Every `ResourceEvent::Failed` the receiver saw, as `(kind, message)`.
    fn failures(rx: &mut tokio::sync::broadcast::Receiver<EngineEvent>) -> Vec<(FailureKind, String)> {
        let mut out = Vec::new();
        while let Ok(EngineEvent::Resource { event, .. }) = rx.try_recv() {
            if let ResourceEvent::Failed { kind, error, .. } = event {
                out.push((kind, error.to_string()));
            }
        }
        out
    }

    /// A request refused before it is sent gets a `Blocked` and nothing else -- the code
    /// that emits the terminal `Failed` is downstream of the rejection and never runs. A
    /// shell that only listened for the terminal event would show no row at all for it,
    /// which is how three stylesheets on a page became two in the network panel.
    #[test]
    fn a_request_refused_before_it_is_sent_is_still_reported() {
        let (emitter, mut rx) = emitter();
        emitter.on_event(NetEvent::Blocked {
            url: url::Url::parse("http://example.com/x.css").unwrap(),
            reason: gosub_sonar::net::types::BlockReason::MixedContent,
        });

        let seen = failures(&mut rx);
        assert_eq!(seen.len(), 1, "expected exactly one failure, got {seen:?}");
        assert_eq!(seen[0].0, FailureKind::Blocked);
    }

    /// And when the terminal event *does* follow, the request has still failed once. The
    /// first cause is kept because it is the specific one.
    #[test]
    fn a_cause_followed_by_the_terminal_event_is_reported_once() {
        let (emitter, mut rx) = emitter();
        let url = url::Url::parse("http://example.com/x.css").unwrap();
        emitter.on_event(NetEvent::Blocked {
            url: url.clone(),
            reason: gosub_sonar::net::types::BlockReason::MixedContent,
        });
        emitter.on_event(NetEvent::Failed {
            url,
            error: anyhow::anyhow!("net.get_with_redirects request failed"),
        });

        let seen = failures(&mut rx);
        assert_eq!(seen.len(), 1, "expected exactly one failure, got {seen:?}");
        assert_eq!(seen[0].0, FailureKind::Blocked);
    }

    /// The case that made this worth doing: a host nothing is listening on and a body that
    /// stopped mid-stream are both transport failures. Reported as a broken transfer the
    /// first sends you looking at the server; reported as a connection failure it sends you
    /// at the address, which is where the problem is. Sonar draws the line and tests it
    /// against a real socket; this checks the engine keeps it.
    #[test]
    fn a_host_that_never_connects_is_not_reported_as_a_broken_transfer() {
        let connect = anyhow::Error::from(NetError::Transport(TransportError {
            kind: TransportErrorKind::Connect,
            message: "net.get_with_redirects request failed: connection refused".into(),
        }));
        assert_eq!(classify(&connect), FailureKind::Connect);

        let mid_body = anyhow::Error::from(NetError::Transport(TransportError {
            kind: TransportErrorKind::Body,
            message: "error reading a body from connection".into(),
        }));
        assert_eq!(classify(&mid_body), FailureKind::Transfer);
    }

    /// `TransportErrorKind` is non-exhaustive, so the catch-all arm gets whatever sonar
    /// learns to tell apart next. It must not be reported as a kind the engine does know.
    #[test]
    fn an_unmapped_transport_kind_claims_nothing() {
        let err = anyhow::Error::from(NetError::Transport(TransportError {
            kind: TransportErrorKind::Builder,
            message: "invalid header value".into(),
        }));
        assert_eq!(classify(&err), FailureKind::Other);
    }

    /// An error from somewhere other than the network stack says nothing about the
    /// cause, and claiming one would be worse than admitting we do not know.
    #[test]
    fn an_unrecognised_error_claims_nothing() {
        assert_eq!(classify(&anyhow::anyhow!("something went wrong")), FailureKind::Other);
    }
}
