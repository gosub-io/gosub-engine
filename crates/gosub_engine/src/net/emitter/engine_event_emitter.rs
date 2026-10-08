use crate::engine::events::{CancelReason, ResourceEvent};
use crate::engine::types::{EventChannel, RequestId, ResourceChannel};
use crate::engine::LoadError;
use crate::events::{EngineEvent, ResourceUpdate};
use crate::net::emitter::{report_url, NetObserver};
use crate::net::events::NetEvent;
use crate::net::req_ref_tracker::{RequestReference, REF_REGISTRY};
use crate::net::types::{Initiator, ResourceKind};
use crate::net::BlockReason;
use crate::tab::TabId;

/// Converts `NetEvent`s into `EngineEvent`s sent back to the UA over `event_tx`.
pub struct EngineEventEmitter {
    /// The tab ID to route the event to
    tab_id: TabId,
    /// The request ID to correlate the event with
    req_id: RequestId,
    /// The request reference to correlate the event with
    reference: RequestReference,
    /// Per-resource events go to the resource stream.
    resource_tx: ResourceChannel,
    /// Throttled navigation and download progress goes to the control bus.
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
        // The net layer normally never sees tab IDs; needed here to route
        // events back to the right tab (from the resource_request_map).
        tab_id: TabId,
        req_id: RequestId,
        reference: RequestReference,
        resource_tx: ResourceChannel,
        event_tx: EventChannel,
        kind: ResourceKind,
        initiator: Initiator,
    ) -> Self {
        Self {
            tab_id,
            req_id,
            reference,
            resource_tx,
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
    fn report_failure(&self, url: String, error: LoadError) {
        use std::sync::atomic::Ordering;
        if self.failure_reported.swap(true, Ordering::Relaxed) {
            return;
        }
        REF_REGISTRY.forget_request(self.req_id);
        self.emit(ResourceEvent::Failed {
            request_id: self.req_id,
            reference: self.reference,
            url,
            error,
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
        let _ = self.resource_tx.send(ResourceUpdate {
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
                    url: report_url(&url).to_string(),
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
                    url: report_url(&url).to_string(),
                    body,
                    truncated,
                });
            }
            NetEvent::Started { url } => {
                self.emit(ResourceEvent::Started {
                    request_id: self.req_id,
                    reference: self.reference,
                    url: report_url(&url).to_string(),
                    kind: self.kind,
                    initiator: self.initiator,
                });
            }
            NetEvent::Redirected { from, to, status } => {
                self.emit(ResourceEvent::Redirected {
                    request_id: self.req_id,
                    reference: self.reference,
                    from: report_url(&from).to_string(),
                    to: report_url(&to).to_string(),
                    status,
                });
            }
            NetEvent::ResponseHeaders { url, status, headers } => {
                self.emit(ResourceEvent::Headers {
                    request_id: self.req_id,
                    reference: self.reference,
                    url: report_url(&url).to_string(),
                    status,
                    content_length: headers
                        .get(http::header::CONTENT_LENGTH)
                        .and_then(|v| v.to_str().ok())
                        .and_then(|s| s.parse::<u64>().ok()),
                    content_type: headers
                        .get(http::header::CONTENT_TYPE)
                        .and_then(|v| v.to_str().ok())
                        .map(|s| s.to_string()),
                    // `Set-Cookie` is redacted like a request's `Cookie`; see `header_value`.
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
                    url: report_url(&url).into_owned(),
                    received_bytes,
                    elapsed: Some(elapsed),
                });
            }
            NetEvent::Blocked { url, reason } => self.report_failure(
                report_url(&url).to_string(),
                LoadError::Blocked {
                    reason: BlockReason::from_net(reason),
                },
            ),
            NetEvent::TlsFailed { url, error } => self.report_failure(
                report_url(&url).to_string(),
                LoadError::Tls {
                    message: format!(
                        "TLS handshake with {} failed: {:?} ({})",
                        error.host, error.kind, error.message
                    ),
                },
            ),
            NetEvent::Failed { url, error } => {
                self.report_failure(report_url(&url).to_string(), classify(&error));
            }
            // A preflight is an internal hop of a CORS request, not a resource.
            NetEvent::CorsPreflight { url } => {
                log::trace!("CORS preflight for {url}");
            }
            NetEvent::Cancelled { url, reason } => {
                REF_REGISTRY.forget_request(self.req_id);
                self.emit(ResourceEvent::Cancelled {
                    request_id: self.req_id,
                    reference: self.reference,
                    url: report_url(&url).to_string(),
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
fn classify(error: &anyhow::Error) -> LoadError {
    LoadError::from(error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::types::NetError;
    use gosub_sonar::net::types::BlockReason;
    use gosub_sonar::{TransportError, TransportErrorKind};
    use std::sync::Arc;

    /// The whole point of `classify` is that the cause survives the trip through
    /// `anyhow`, including the extra context a caller may have attached on the way.
    #[test]
    fn a_typed_cause_survives_the_anyhow_wrapper() {
        let err = anyhow::Error::from(NetError::Blocked {
            reason: BlockReason::MixedContent,
            url: url::Url::parse("http://example.com/x.css").unwrap(),
        })
        .context("loading stylesheet");

        assert!(matches!(classify(&err), LoadError::Blocked { .. }));
    }

    #[test]
    fn a_timeout_is_not_reported_as_a_transfer_failure() {
        let err = anyhow::Error::from(NetError::Timeout("no response in 30s".into()));
        assert!(matches!(classify(&err), LoadError::Timeout { .. }));
    }

    #[test]
    fn a_broken_transfer_is_distinct_from_a_refusal() {
        let err = anyhow::Error::from(NetError::Read(Arc::new(anyhow::anyhow!(
            "connection reset while reading body"
        ))));
        assert!(matches!(classify(&err), LoadError::Transfer { .. }));
    }

    /// Build an emitter wired to a channel the test can read back.
    fn emitter() -> (EngineEventEmitter, tokio::sync::broadcast::Receiver<ResourceUpdate>) {
        let (resource_tx, rx) = tokio::sync::broadcast::channel(16);
        let (event_tx, _) = tokio::sync::broadcast::channel(16);
        let emitter = EngineEventEmitter::new(
            TabId::new(),
            RequestId::new(),
            RequestReference::Document(1),
            resource_tx,
            event_tx,
            ResourceKind::Stylesheet,
            Initiator::Parser,
        );
        (emitter, rx)
    }

    /// Every `ResourceEvent::Failed` the receiver saw, as `(kind, message)`.
    fn failures(rx: &mut tokio::sync::broadcast::Receiver<ResourceUpdate>) -> Vec<(LoadError, String)> {
        let mut out = Vec::new();
        while let Ok(ResourceUpdate { event, .. }) = rx.try_recv() {
            if let ResourceEvent::Failed { error, .. } = event {
                let message = error.to_string();
                out.push((error, message));
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
        assert!(matches!(seen[0].0, LoadError::Blocked { .. }));
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
        assert!(matches!(seen[0].0, LoadError::Blocked { .. }));
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
        assert!(matches!(classify(&connect), LoadError::Connect { .. }));

        let mid_body = anyhow::Error::from(NetError::Transport(TransportError {
            kind: TransportErrorKind::Body,
            message: "error reading a body from connection".into(),
        }));
        assert!(matches!(classify(&mid_body), LoadError::Transfer { .. }));
    }

    /// `TransportErrorKind` is non-exhaustive, so the catch-all arm gets whatever sonar
    /// learns to tell apart next. It must not be reported as a kind the engine does know.
    #[test]
    fn an_unmapped_transport_kind_claims_nothing() {
        let err = anyhow::Error::from(NetError::Transport(TransportError {
            kind: TransportErrorKind::Builder,
            message: "invalid header value".into(),
        }));
        assert!(matches!(classify(&err), LoadError::Other { .. }));
    }

    /// An error from somewhere other than the network stack says nothing about the
    /// cause, and claiming one would be worse than admitting we do not know.
    #[test]
    fn an_unrecognised_error_claims_nothing() {
        assert!(matches!(
            classify(&anyhow::anyhow!("something went wrong")),
            LoadError::Other { .. }
        ));
    }
}
