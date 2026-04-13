use crate::engine::events::{CancelReason, ResourceEvent};
use crate::engine::types::EventChannel;
use crate::events::{EngineEvent, NavigationEvent};
use crate::tab::TabId;
use gosub_net::emitter::NetObserver;
use gosub_net::events::NetEvent;
use gosub_net::net_types::{FetchResultMeta, Initiator, ResourceKind};
use gosub_net::req_ref_tracker::RequestReference;
use gosub_net::types::RequestId;
use http::StatusCode;

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
    //// The initiator of the request
    initiator: Initiator,
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
        }
    }

    /// Emit a navigation event
    fn emit_navigation_event(&self, ev: NavigationEvent) {
        let _ = self.event_tx.send(EngineEvent::Navigation {
            tab_id: self.tab_id,
            event: ev,
        });
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
                self.emit(ResourceEvent::Finished {
                    request_id: self.req_id,
                    reference: self.reference,
                    url,
                    received_bytes,
                    elapsed: Some(elapsed),
                });
            }
            NetEvent::Failed { url, error } => {
                self.emit(ResourceEvent::Failed {
                    request_id: self.req_id,
                    reference: self.reference,
                    url: url.to_string(),
                    error: error.into(),
                });
            }
            NetEvent::Cancelled { url, reason } => {
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
            NetEvent::DecisionRequired {
                url,
                status,
                headers,
                content_length,
                content_type,
                peek_buf,
                token,
            } => {
                let RequestReference::Navigation(nav_id) = self.reference else {
                    // Only navigation requests can trigger decision required events
                    log::warn!(
                        "Received DecisionRequired event for non-navigation request: {:?}",
                        self.reference
                    );
                    return;
                };

                let has_body = !peek_buf.is_empty();

                self.emit_navigation_event(NavigationEvent::DecisionRequired {
                    nav_id,
                    decision_token: token,
                    meta: FetchResultMeta {
                        final_url: url.clone(),
                        status,
                        status_text: StatusCode::from_u16(status)
                            .map(|s| s.canonical_reason().unwrap_or("").to_string())
                            .unwrap_or_default(),
                        headers: headers.clone(),
                        content_length,
                        content_type,
                        has_body,
                    },
                });
            }
        }
    }
}
