//! [`NetObserver`] decorator that files fetch timings into the shared timing table.
//!
//! Fetching happens in `gosub-sonar`, so the engine has no clock on it. What it does
//! have is sonar's observer hook, and the events already carry the numbers we want:
//! `Finished` reports its own `elapsed`, and the arrival of `ResponseHeaders` is time
//! to first byte. This wraps whatever observer the engine would otherwise have used,
//! reads those two events in passing, and hands the event on untouched.
//!
//! It is a decorator rather than a fan-out on purpose. [`NetObserver::on_event`] takes
//! the event **by value** and `NetEvent` is not `Clone` (its `Failed` variant carries an
//! `anyhow::Error`, which cannot be), so an observer cannot hand the same event to two
//! others without an upstream change to sonar. Inspecting by reference and then passing
//! ownership along needs no such change.

use std::sync::Arc;
use std::time::Instant;

use crate::net::emitter::NetObserver;
use crate::net::events::NetEvent;
use crate::net::types::ResourceKind;

/// Wraps another [`NetObserver`], recording `net.fetch.*` and `net.ttfb` on the way past.
pub struct TimingEmitter {
    /// The observer this one decorates. Every event is forwarded to it unchanged.
    inner: Arc<dyn NetObserver + Send + Sync>,
    /// Resource kind, which selects the `net.fetch.*` namespace.
    kind: ResourceKind,
    /// Navigation these samples belong to, carried explicitly because events arrive on
    /// the fetch stack's own tasks where the thread-local scope does not apply.
    scope: Option<gosub_shared::timing::ScopeId>,
    /// When this request was first seen, so `ResponseHeaders` can be turned into a
    /// duration. `Started` normally sets it; requests served from cache may never emit
    /// `Started`, in which case TTFB is skipped rather than guessed.
    started: parking_lot::Mutex<Option<Instant>>,
}

impl TimingEmitter {
    /// Wrap `inner` so this request's fetch timings are recorded as they pass through.
    ///
    /// `scope` attributes them to a navigation; `None` leaves them unattributed rather
    /// than filed against whichever navigation happens to be current.
    #[must_use]
    pub fn wrap(
        inner: Arc<dyn NetObserver + Send + Sync>,
        kind: ResourceKind,
        scope: Option<gosub_shared::timing::ScopeId>,
    ) -> Self {
        Self {
            inner,
            kind,
            scope,
            started: parking_lot::Mutex::new(None),
        }
    }

    /// File `duration` under `namespace`, scoped when this request belongs to a navigation.
    fn record(&self, namespace: &str, duration: std::time::Duration, url: &url::Url) {
        let us = duration.as_micros() as u64;
        match self.scope {
            Some(scope) => gosub_shared::timing::record_in(scope, namespace, us, Some(url.to_string())),
            None => gosub_shared::timing::record(namespace, us, Some(url.to_string())),
        }
    }

    /// The `net.fetch.*` namespace for this request's resource kind.
    ///
    /// Kinds that aren't part of the page-load story share `net.fetch.other` rather than
    /// each getting a namespace nothing reads.
    fn namespace(&self) -> &'static str {
        match self.kind {
            ResourceKind::Document => "net.fetch.html",
            ResourceKind::Stylesheet => "net.fetch.css",
            ResourceKind::Script { .. } => "net.fetch.js",
            ResourceKind::Image => "net.fetch.image",
            ResourceKind::Font => "net.fetch.font",
            _ => "net.fetch.other",
        }
    }
}

impl NetObserver for TimingEmitter {
    fn on_event(&self, ev: NetEvent) {
        // Read what we need by reference; ownership of `ev` still moves on below.
        match &ev {
            NetEvent::Started { .. } => {
                *self.started.lock() = Some(Instant::now());
            }
            NetEvent::ResponseHeaders { url, .. } => {
                if let Some(started) = *self.started.lock() {
                    self.record("net.ttfb", started.elapsed(), url);
                }
            }
            NetEvent::Finished { elapsed, url, .. } => {
                // sonar measured this itself - use its number rather than re-deriving one
                // from our own clock, which would include event-queue latency.
                self.record(self.namespace(), *elapsed, url);
            }
            _ => {}
        }

        self.inner.on_event(ev);
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use gosub_shared::timing::snapshot_stats;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;
    use url::Url;

    /// Counts what it is handed, so we can assert the decorator forwards everything.
    struct CountingObserver(AtomicUsize);
    impl NetObserver for CountingObserver {
        fn on_event(&self, _ev: NetEvent) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn url() -> Url {
        Url::parse("https://example.org/a.css").expect("static url parses")
    }

    #[test]
    fn records_fetch_duration_and_forwards_every_event() {
        // The timing table is process-global and tests run in parallel: assert a relative
        // increase, and never reset_stats() here - that would wipe whatever a concurrent
        // test is recording.
        let stat_of = |ns: &str| {
            snapshot_stats()
                .iter()
                .find(|s| s.namespace == ns)
                .map(|s| (s.count, s.total_us))
        };
        let before = stat_of("net.fetch.css").unwrap_or((0, 0));

        let inner = Arc::new(CountingObserver(AtomicUsize::new(0)));
        let em = TimingEmitter::wrap(inner.clone(), ResourceKind::Stylesheet, None);

        em.on_event(NetEvent::Started { url: url() });
        em.on_event(NetEvent::Finished {
            received_bytes: 1234,
            elapsed: Duration::from_millis(40),
            url: url(),
        });

        // Both events reached the wrapped observer.
        assert_eq!(inner.0.load(Ordering::SeqCst), 2);

        let (count, total_us) = stat_of("net.fetch.css").expect("net.fetch.css recorded");
        assert_eq!(count, before.0 + 1, "exactly one sample added");
        // sonar's own elapsed is used verbatim, not re-measured from our clock.
        assert_eq!(total_us - before.1, 40_000);
    }

    /// A request that belongs to a navigation records into that navigation's scope, not
    /// the global bucket - the events arrive on the fetch stack's tasks, where there is no
    /// thread-local scope to fall back on.
    #[test]
    fn records_into_the_navigation_scope() {
        let scope = gosub_shared::timing::ScopeId(uuid::Uuid::new_v4());
        let inner = Arc::new(CountingObserver(AtomicUsize::new(0)));
        let em = TimingEmitter::wrap(inner, ResourceKind::Document, Some(scope));

        em.on_event(NetEvent::Finished {
            received_bytes: 10,
            elapsed: Duration::from_millis(25),
            url: url(),
        });

        let scoped = gosub_shared::timing::snapshot_stats_for(scope);
        let fetch = scoped
            .iter()
            .find(|s| s.namespace == "net.fetch.html")
            .expect("recorded in the navigation scope");
        assert_eq!(fetch.count, 1);
        assert_eq!(fetch.total_us, 25_000);
    }

    #[test]
    fn namespace_follows_resource_kind() {
        let inner = Arc::new(CountingObserver(AtomicUsize::new(0)));
        let ns = |k| TimingEmitter::wrap(inner.clone(), k, None).namespace();

        assert_eq!(ns(ResourceKind::Document), "net.fetch.html");
        assert_eq!(ns(ResourceKind::Stylesheet), "net.fetch.css");
        assert_eq!(ns(ResourceKind::Script { blocking: true }), "net.fetch.js");
        assert_eq!(ns(ResourceKind::Image), "net.fetch.image");
        assert_eq!(ns(ResourceKind::Font), "net.fetch.font");
        assert_eq!(ns(ResourceKind::Xhr), "net.fetch.other");
    }
}
