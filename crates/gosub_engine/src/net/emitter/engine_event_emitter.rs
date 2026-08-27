use crate::engine::events::{CancelReason, ResourceEvent};
use crate::engine::types::{EventChannel, RequestId};
use crate::events::EngineEvent;
use crate::net::emitter::NetObserver;
use crate::net::events::NetEvent;
use crate::net::req_ref_tracker::{RequestReference, REF_REGISTRY};
use crate::net::types::{Initiator, ResourceKind};
use crate::tab::TabId;
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
        }
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
    fn on_event(&self, ev: NetEvent) {
        match ev {
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
                        .get(reqwest::header::CONTENT_LENGTH)
                        .and_then(|v| v.to_str().ok())
                        .and_then(|s| s.parse::<u64>().ok()),
                    content_type: headers
                        .get(reqwest::header::CONTENT_TYPE)
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
            NetEvent::Blocked { url, reason } => {
                REF_REGISTRY.forget_request(self.req_id);
                self.emit(ResourceEvent::Failed {
                    request_id: self.req_id,
                    reference: self.reference,
                    url: url.to_string(),
                    error: Arc::new(anyhow::anyhow!("blocked: {reason:?}")),
                });
            }
            NetEvent::Failed { url, error } => {
                REF_REGISTRY.forget_request(self.req_id);
                self.emit(ResourceEvent::Failed {
                    request_id: self.req_id,
                    reference: self.reference,
                    url: url.to_string(),
                    error: error.into(),
                });
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
        }
    }
}
