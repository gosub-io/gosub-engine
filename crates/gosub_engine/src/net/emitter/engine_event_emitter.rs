use crate::engine::events::{CancelReason, ResourceEvent};
use crate::engine::types::{EventChannel, RequestId, ResourceChannel};
use crate::engine::LoadError;
use crate::events::{EngineEvent, ResourceUpdate};
use crate::net::detail::{AuthChallenge, AuthTarget, CacheOutcome, RequestChange, UpgradeReason};
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
    /// Whether the progress event that completes the body has been forwarded:
    /// it passes the step once, not every time it is repeated.
    completion_reported: std::sync::atomic::AtomicBool,
    /// Whether the request has already been reported as failed. See
    /// [`Self::report_failure`].
    failure_reported: std::sync::atomic::AtomicBool,
    /// Whether the request's row has been opened (`ResourceEvent::Started`). See
    /// [`Self::open`].
    opened: std::sync::atomic::AtomicBool,
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
            completion_reported: std::sync::atomic::AtomicBool::new(false),
            failure_reported: std::sync::atomic::AtomicBool::new(false),
            opened: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Open the request's row, once. The fetcher opens every request it takes with
    /// `Accepted`, before any check or wait: a request refused before it was sent, or
    /// cancelled while it waited, never gets a `Started`, and still has its row and its
    /// end. `Started` comes once per attempt, so it opens the row only for a request
    /// reported by something other than the fetcher that sent no `Accepted`.
    fn open(&self, url: &url::Url) {
        if self.opened.swap(true, std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        self.emit(ResourceEvent::Started {
            request_id: self.req_id,
            reference: self.reference,
            url: report_url(url).to_string(),
            kind: self.kind,
            initiator: self.initiator,
        });
    }

    /// Report the request's failure once, keeping the first cause that arrives.
    ///
    /// The network stack names a specific cause first (`Blocked`, `TlsFailed`) and follows
    /// it with the terminal `Failed`. Reporting on both would emit two failures for one
    /// request; first one wins, and the first is always the more specific. A retried request
    /// ends once, after its last attempt: the attempts before it end in `Retrying`.
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

    /// Forward at most one progress event per `STEP` bytes received, and the one that
    /// reaches `expected` once: repeated, it would pass every time and fill the bus.
    fn should_report_progress(&self, received: u64, expected: Option<u64>) -> bool {
        use std::sync::atomic::Ordering;
        const STEP: u64 = 64 * 1024;
        if Some(received) == expected {
            if self.completion_reported.swap(true, Ordering::Relaxed) {
                return false;
            }
            self.last_progress.store(received, Ordering::Relaxed);
            return true;
        }
        let last = self.last_progress.load(Ordering::Relaxed);
        if received.saturating_sub(last) >= STEP {
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
            // The request's row opens on the first of these; see `open`.
            NetEvent::Accepted { url, .. } | NetEvent::Started { url } => self.open(&url),
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
            NetEvent::TlsFailed { url, error } => {
                self.report_failure(report_url(&url).to_string(), LoadError::from_tls(&error))
            }
            NetEvent::Failed { url, error } => {
                self.report_failure(report_url(&url).to_string(), classify(&error));
            }
            // A preflight is an internal hop of a CORS request, not a resource of its own, so
            // it is reported on the request that needed it: its round-trip is time the
            // embedder would otherwise see as an unexplained gap before the response.
            NetEvent::CorsPreflight { url } => {
                self.emit(ResourceEvent::CorsPreflight {
                    request_id: self.req_id,
                    reference: self.reference,
                    url: report_url(&url).to_string(),
                });
            }
            NetEvent::CorsPreflightDone { url, elapsed } => {
                self.emit(ResourceEvent::CorsPreflightDone {
                    request_id: self.req_id,
                    reference: self.reference,
                    url: report_url(&url).to_string(),
                    elapsed,
                });
            }
            NetEvent::Cache { url, outcome } => {
                self.emit(ResourceEvent::Cache {
                    request_id: self.req_id,
                    reference: self.reference,
                    url: report_url(&url).to_string(),
                    outcome: CacheOutcome::from_net(outcome),
                });
            }
            NetEvent::AuthRequired {
                url,
                target,
                challenges,
                retried,
            } => {
                self.emit(ResourceEvent::AuthRequired {
                    request_id: self.req_id,
                    reference: self.reference,
                    url: report_url(&url).to_string(),
                    target: AuthTarget::from_net(target),
                    challenges: challenges.iter().map(AuthChallenge::from_net).collect(),
                    retried,
                });
            }
            NetEvent::SecurityUpgraded { from, to, reason } => {
                self.emit(ResourceEvent::SecurityUpgraded {
                    request_id: self.req_id,
                    reference: self.reference,
                    from: report_url(&from).to_string(),
                    to: report_url(&to).to_string(),
                    reason: UpgradeReason::from_net(reason),
                });
            }
            NetEvent::RequestModified { url, change } => {
                self.emit(ResourceEvent::RequestModified {
                    request_id: self.req_id,
                    reference: self.reference,
                    url: report_url(&url).to_string(),
                    // A referrer is a URL like any other here, and can carry credentials too.
                    change: RequestChange::from_net(&change, |u| report_url(u).to_string()),
                });
            }
            // Only announces the next attempt: the attempt before it ended here, and the
            // request goes on.
            NetEvent::Retrying {
                url,
                attempt,
                delay,
                reason,
            } => {
                self.emit(ResourceEvent::Retrying {
                    request_id: self.req_id,
                    reference: self.reference,
                    url: report_url(&url).to_string(),
                    attempt,
                    delay,
                    reason,
                });
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
            // `NetEvent` is non-exhaustive: a case sonar adds later is dropped here until the
            // engine has a mirror for it.
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

    /// The progress event that completes a body is forwarded once, however
    /// often it is repeated: it is exempt from the byte step, so repeated it
    /// would otherwise fill the control bus.
    #[test]
    fn the_completing_progress_is_forwarded_once() {
        let (emitter, _rx) = emitter();
        assert!(emitter.should_report_progress(100, Some(100)));
        assert!(!emitter.should_report_progress(100, Some(100)));
        assert!(!emitter.should_report_progress(100, Some(100)));
        // The step still applies past it.
        assert!(emitter.should_report_progress(100 + 64 * 1024, None));
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

    /// The names of the events the receiver saw, in order.
    fn names(rx: &mut tokio::sync::broadcast::Receiver<ResourceUpdate>) -> Vec<&'static str> {
        drain(rx)
            .into_iter()
            .map(|ev| match ev {
                ResourceEvent::Started { .. } => "started",
                ResourceEvent::Failed { .. } => "failed",
                ResourceEvent::Finished { .. } => "finished",
                ResourceEvent::Retrying { .. } => "retrying",
                _ => "other",
            })
            .collect()
    }

    /// A retried request has one row and one end: every attempt reports its `Started`,
    /// `Retrying` comes between them, and only the last attempt ends the request.
    #[test]
    fn a_retried_request_has_one_row_and_one_end() {
        let (emitter, mut rx) = emitter();
        let target = url("https://does-not-resolve.test/");
        emitter.on_event(NetEvent::Accepted {
            url: target.clone(),
            joined: false,
        });
        for attempt in 1..=2 {
            emitter.on_event(NetEvent::Started { url: target.clone() });
            emitter.on_event(NetEvent::Retrying {
                url: target.clone(),
                attempt,
                delay: std::time::Duration::ZERO,
                reason: "dns error".into(),
            });
        }
        emitter.on_event(NetEvent::Started { url: target.clone() });
        emitter.on_event(NetEvent::Failed {
            url: target,
            error: anyhow::anyhow!("dns error: failed to lookup address information"),
        });
        assert_eq!(names(&mut rx), ["started", "retrying", "retrying", "failed"]);
    }

    /// A request refused before it was sent never gets a `Started`, and still has its row
    /// and its end, with the cause the refusal named.
    #[test]
    fn a_request_refused_before_it_was_sent_has_its_row() {
        let (emitter, mut rx) = emitter();
        let target = url("http://insecure.test/a.css");
        emitter.on_event(NetEvent::Accepted {
            url: target.clone(),
            joined: false,
        });
        emitter.on_event(NetEvent::Blocked {
            url: target.clone(),
            reason: BlockReason::MixedContent,
        });
        emitter.on_event(NetEvent::Failed {
            url: target.clone(),
            error: anyhow::anyhow!(crate::net::types::NetError::Blocked {
                reason: BlockReason::MixedContent,
                url: target,
            }),
        });
        let events = drain(&mut rx);
        assert!(matches!(events[0], ResourceEvent::Started { .. }), "{events:?}");
        assert_eq!(events.len(), 2, "{events:?}");
        assert!(
            matches!(
                &events[1],
                ResourceEvent::Failed {
                    error: LoadError::Blocked { .. },
                    ..
                }
            ),
            "{events:?}"
        );
    }

    /// A request that joined another's fetch opens its row on `Accepted`: it has no
    /// `Started` of its own. A loader that reports no `Accepted` opens it on `Started`.
    #[test]
    fn the_row_opens_on_accepted_or_else_on_started() {
        let target = url("https://example.test/a.css");
        let (joined, mut rx) = emitter();
        joined.on_event(NetEvent::Accepted {
            url: target.clone(),
            joined: true,
        });
        joined.on_event(NetEvent::Finished {
            url: target.clone(),
            received_bytes: 1,
            elapsed: std::time::Duration::ZERO,
        });
        assert_eq!(names(&mut rx), ["started", "finished"]);

        let (loaded, mut rx) = emitter();
        loaded.on_event(NetEvent::Started { url: target.clone() });
        loaded.on_event(NetEvent::Finished {
            url: target,
            received_bytes: 1,
            elapsed: std::time::Duration::ZERO,
        });
        assert_eq!(names(&mut rx), ["started", "finished"]);
    }

    /// Every event the receiver saw, in order.
    fn drain(rx: &mut tokio::sync::broadcast::Receiver<ResourceUpdate>) -> Vec<ResourceEvent> {
        let mut out = Vec::new();
        while let Ok(ResourceUpdate { event, .. }) = rx.try_recv() {
            out.push(event);
        }
        out
    }

    /// Feed one event to a fresh emitter and return the one event it forwarded, with the
    /// request id it should carry.
    fn forward(ev: NetEvent) -> (ResourceEvent, RequestId) {
        let (emitter, mut rx) = emitter();
        emitter.on_event(ev);
        let mut seen = drain(&mut rx);
        assert_eq!(seen.len(), 1, "expected exactly one event, got {seen:?}");
        (seen.remove(0), emitter.req_id)
    }

    fn url(s: &str) -> url::Url {
        url::Url::parse(s).unwrap()
    }

    #[test]
    fn a_cache_hit_is_reported() {
        let (ev, id) = forward(NetEvent::Cache {
            url: url("https://example.test/a.css"),
            outcome: gosub_sonar::net::cache::CacheOutcome::Hit,
        });
        let ResourceEvent::Cache {
            request_id,
            reference,
            url,
            outcome,
        } = ev
        else {
            panic!("expected Cache, got {ev:?}");
        };
        assert_eq!(request_id, id);
        assert_eq!(reference, RequestReference::Document(1));
        assert_eq!(url, "https://example.test/a.css");
        assert_eq!(outcome, CacheOutcome::Hit);
    }

    /// The realm is what a login prompt shows, and `retried` is what tells an embedder whether
    /// the stack answered the challenge or the `401` is the final response.
    #[test]
    fn an_auth_challenge_is_reported_with_its_scheme_and_realm() {
        use gosub_sonar::net::auth as sonar_auth;
        let challenged = url("https://example.test/private/");
        let (ev, id) = forward(NetEvent::AuthRequired {
            url: challenged.clone(),
            target: sonar_auth::AuthTarget::Proxy,
            challenges: vec![sonar_auth::AuthChallenge {
                url: challenged,
                target: sonar_auth::AuthTarget::Proxy,
                scheme: sonar_auth::AuthScheme::Digest,
                realm: Some("staff".into()),
                params: vec![("nonce".into(), "abc".into())],
                token68: None,
                attempt: 0,
                proxy: None,
            }],
            retried: false,
        });
        let ResourceEvent::AuthRequired {
            request_id,
            url,
            target,
            challenges,
            retried,
            ..
        } = ev
        else {
            panic!("expected AuthRequired, got {ev:?}");
        };
        assert_eq!(request_id, id);
        assert_eq!(url, "https://example.test/private/");
        assert_eq!(target, AuthTarget::Proxy);
        assert_eq!(
            challenges,
            [AuthChallenge {
                scheme: "Digest".into(),
                realm: Some("staff".into()),
            }]
        );
        assert!(!retried);
    }

    #[test]
    fn an_https_upgrade_is_reported_with_both_urls_and_the_reason() {
        let (ev, id) = forward(NetEvent::SecurityUpgraded {
            from: url("http://example.test/"),
            to: url("https://example.test/"),
            reason: gosub_sonar::net::types::UpgradeReason::Hsts,
        });
        let ResourceEvent::SecurityUpgraded {
            request_id,
            from,
            to,
            reason,
            ..
        } = ev
        else {
            panic!("expected SecurityUpgraded, got {ev:?}");
        };
        assert_eq!(request_id, id);
        assert_eq!(from, "http://example.test/");
        assert_eq!(to, "https://example.test/");
        assert_eq!(reason, UpgradeReason::Hsts);
    }

    #[test]
    fn a_policy_change_to_a_request_is_reported() {
        let (ev, id) = forward(NetEvent::RequestModified {
            url: url("https://other.test/"),
            change: gosub_sonar::net::types::RequestChange::CredentialsWithheld,
        });
        let ResourceEvent::RequestModified {
            request_id,
            url,
            change,
            ..
        } = ev
        else {
            panic!("expected RequestModified, got {ev:?}");
        };
        assert_eq!(request_id, id);
        assert_eq!(url, "https://other.test/");
        assert_eq!(change, RequestChange::CredentialsWithheld);
    }

    #[test]
    fn a_retry_is_announced_with_its_number_delay_and_reason() {
        let (ev, id) = forward(NetEvent::Retrying {
            url: url("https://example.test/"),
            attempt: 2,
            delay: std::time::Duration::from_millis(250),
            reason: "connection refused".into(),
        });
        let ResourceEvent::Retrying {
            request_id,
            url,
            attempt,
            delay,
            reason,
            ..
        } = ev
        else {
            panic!("expected Retrying, got {ev:?}");
        };
        assert_eq!(request_id, id);
        assert_eq!(url, "https://example.test/");
        assert_eq!(attempt, 2);
        assert_eq!(delay, std::time::Duration::from_millis(250));
        assert_eq!(reason, "connection refused");
    }

    #[test]
    fn a_cors_preflight_is_reported_on_the_request_that_needed_it() {
        let (ev, id) = forward(NetEvent::CorsPreflight {
            url: url("https://api.test/data"),
        });
        let ResourceEvent::CorsPreflight { request_id, url, .. } = ev else {
            panic!("expected CorsPreflight, got {ev:?}");
        };
        assert_eq!(request_id, id);
        assert_eq!(url, "https://api.test/data");
    }

    #[test]
    fn a_finished_cors_preflight_is_reported_with_its_round_trip() {
        let (ev, id) = forward(NetEvent::CorsPreflightDone {
            url: url("https://api.test/data"),
            elapsed: std::time::Duration::from_millis(42),
        });
        let ResourceEvent::CorsPreflightDone {
            request_id,
            url,
            elapsed,
            ..
        } = ev
        else {
            panic!("expected CorsPreflightDone, got {ev:?}");
        };
        assert_eq!(request_id, id);
        assert_eq!(url, "https://api.test/data");
        assert_eq!(elapsed, std::time::Duration::from_millis(42));
    }

    /// The URLs inside these events go wherever the embedder puts them, like every other URL
    /// it is given, so credentials are dropped from them too. A referrer is easy to miss: it
    /// sits inside the change, not on the event.
    ///
    /// Relies on the sensitive switch being at its default (off), and does not flip it.
    #[test]
    fn urls_inside_upgrades_and_referrer_changes_carry_no_credentials() {
        let (ev, _) = forward(NetEvent::SecurityUpgraded {
            from: url("http://alice:hunter2@example.test/"),
            to: url("https://alice:hunter2@example.test/"),
            reason: gosub_sonar::net::types::UpgradeReason::MixedContent,
        });
        let ResourceEvent::SecurityUpgraded { from, to, .. } = ev else {
            panic!("expected SecurityUpgraded, got {ev:?}");
        };
        assert_eq!(from, "http://example.test/");
        assert_eq!(to, "https://example.test/");

        let (ev, _) = forward(NetEvent::RequestModified {
            url: url("https://bob:pw@other.test/"),
            change: gosub_sonar::net::types::RequestChange::ReferrerReduced {
                from: url("https://alice:hunter2@example.test/page?q=1"),
                to: Some(url("https://alice:hunter2@example.test/")),
                policy: gosub_sonar::net::referrer::ReferrerPolicy::StrictOriginWhenCrossOrigin,
            },
        });
        let ResourceEvent::RequestModified { url, change, .. } = ev else {
            panic!("expected RequestModified, got {ev:?}");
        };
        assert_eq!(url, "https://other.test/");
        let RequestChange::ReferrerReduced { from, to, .. } = change else {
            panic!("expected ReferrerReduced, got {change:?}");
        };
        assert_eq!(from, "https://example.test/page?q=1");
        assert_eq!(to.as_deref(), Some("https://example.test/"));
    }

    /// A TLS failure stays typed on its way out: an embedder offering "continue anyway" needs
    /// the kind to know whether it can, and the host and fingerprint to remember the answer.
    #[test]
    fn a_tls_failure_keeps_its_kind_host_and_fingerprint() {
        let (emitter, mut rx) = emitter();
        emitter.on_event(NetEvent::TlsFailed {
            url: url("https://self-signed.test/"),
            error: gosub_sonar::net::tls::TlsError {
                kind: gosub_sonar::net::tls::TlsErrorKind::UnknownIssuer,
                host: "self-signed.test".into(),
                message: "invalid peer certificate: UnknownIssuer".into(),
                certificate: Some(vec![1, 2, 3]),
                fingerprint: Some([7; 32]),
            },
        });

        let seen = failures(&mut rx);
        assert_eq!(seen.len(), 1, "expected exactly one failure, got {seen:?}");
        let LoadError::Tls {
            kind,
            host,
            fingerprint,
            message,
        } = &seen[0].0
        else {
            panic!("expected LoadError::Tls, got {:?}", seen[0].0);
        };
        assert_eq!(*kind, crate::net::TlsFailureKind::UnknownIssuer);
        assert_eq!(host, "self-signed.test");
        assert_eq!(*fingerprint, Some([7; 32]));
        assert_eq!(message, "invalid peer certificate: UnknownIssuer");
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
