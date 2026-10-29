//! Per-document security state: what a tab's security indicator reads.
//!
//! The main document's hops are folded into a record while its request is fetched. The tab
//! takes that record when the navigation ends ([`SecuritySlot::commit`],
//! [`SecuritySlot::fail`]), and the document's subresources count into it from then on.
//!
//! The slot lives in the tab's [`TabSink`](crate::tab::sink::TabSink) and is registered here
//! by tab, so the fetch side, which only knows the tab id, writes to the same one.
//!
//! The outcome of a failed navigation comes from its [`LoadError`], not from the fetch
//! events: across the network process `TlsFailed` and `Blocked` arrive as a plain `Failed`.

use crate::engine::events::EngineEvent;
use crate::engine::types::{EventChannel, NavigationId};
use crate::engine::LoadError;
use crate::net::detail::{CacheOutcome, TlsFailureKind, UpgradeReason};
use crate::net::emitter::NetObserver;
use crate::net::events::NetEvent;
use crate::net::protocols::{transport, Transport};
use crate::net::BlockReason;
use crate::tab::TabId;
use crate::util::spawn_named;
use parking_lot::{Mutex, RwLock};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};
use tokio::runtime::Handle;
use url::Url;

/// Security state of what a tab shows: the committed document, or the error page of a
/// navigation that failed.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SecurityInfo {
    /// The document's URL, or for a failure the hop that failed, which can differ from the
    /// address bar.
    pub url: Url,
    /// How it came over the network. `None` for what did not (`file:`, `data:`, `about:`,
    /// internal pages).
    pub transport: Option<Transport>,
    /// What happened to the final hop.
    pub outcome: SecurityOutcome,
    /// Why the final hop was rewritten to `https`, if it was.
    pub upgrade: Option<UpgradeReason>,
    /// Where the response came from.
    pub source: ResponseSource,
    /// The document's subresources so far.
    pub subresources: SubresourceSummary,
}

/// What happened to a navigation's final hop.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SecurityOutcome {
    /// The document loaded.
    Loaded,
    /// The TLS handshake with `host` failed.
    TlsFailed {
        /// What went wrong, classified.
        kind: TlsFailureKind,
        /// The host the handshake was with.
        host: String,
        /// The SHA-256 of the certificate the server presented, when it presented one.
        fingerprint: Option<[u8; 32]>,
    },
    /// Refused by policy before it was sent.
    Blocked {
        /// What refused it.
        reason: BlockReason,
    },
    /// Failed another way: no connection, a timeout, a broken transfer. The error itself is
    /// on `NavigationEvent::Failed`.
    Failed,
}

/// Where a document's response came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum ResponseSource {
    /// Fetched, or not cached at all.
    #[default]
    Network,
    /// The HTTP cache served it; `revalidated` when the server confirmed it with a `304`.
    Cache {
        /// The server confirmed the stored response.
        revalidated: bool,
    },
}

/// A document's subresources, counted as they finish.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct SubresourceSummary {
    /// Loaded over an encrypted transport.
    pub over_tls: u32,
    /// Loaded in cleartext.
    pub cleartext: u32,
    /// Failed their TLS handshake.
    pub failed_tls: u32,
    /// Distinct hosts among all of the above.
    pub hosts: u32,
}

/// Subresource updates are announced at most this often.
const ANNOUNCE_INTERVAL: Duration = Duration::from_millis(250);

/// Every tab's slot, by tab.
pub(crate) static SECURITY: LazyLock<SecurityRegistry> = LazyLock::new(SecurityRegistry::default);

#[derive(Default)]
pub(crate) struct SecurityRegistry {
    tabs: RwLock<HashMap<TabId, Arc<SecuritySlot>>>,
}

impl SecurityRegistry {
    pub(crate) fn register(&self, tab_id: TabId, slot: Arc<SecuritySlot>) {
        self.tabs.write().insert(tab_id, slot);
    }

    pub(crate) fn remove(&self, tab_id: TabId) {
        self.tabs.write().remove(&tab_id);
    }

    fn get(&self, tab_id: TabId) -> Option<Arc<SecuritySlot>> {
        self.tabs.read().get(&tab_id).cloned()
    }
}

/// A tab's security state, shared by the tab and the requests it makes.
#[derive(Default)]
pub struct SecuritySlot {
    state: Mutex<SlotState>,
}

#[derive(Default)]
struct SlotState {
    /// The navigation in flight and what its document's hops have shown so far.
    pending: Option<(NavigationId, Hop)>,
    current: Option<Arc<SecurityInfo>>,
    /// Hosts the current document's subresources came from.
    hosts: HashSet<String>,
    /// When a subresource update was last announced, and whether one is waiting.
    announced: Option<Instant>,
    announce_due: bool,
}

/// The current hop of a navigation's document request.
#[derive(Default)]
struct Hop {
    url: Option<Url>,
    upgrade: Option<UpgradeReason>,
    source: ResponseSource,
}

impl SecuritySlot {
    pub(crate) fn get(&self) -> Option<Arc<SecurityInfo>> {
        self.state.lock().current.clone()
    }

    /// Navigation `nav` committed a document at `url`: from now on the tab shows it.
    pub(crate) fn commit(&self, nav: NavigationId, url: &Url) -> Arc<SecurityInfo> {
        let mut state = self.state.lock();
        let hop = take_hop(&mut state, nav);
        let info = Arc::new(SecurityInfo {
            url: url.clone(),
            transport: transport(url),
            outcome: SecurityOutcome::Loaded,
            upgrade: hop.upgrade,
            source: hop.source,
            subresources: SubresourceSummary::default(),
        });
        state.hosts.clear();
        state.current = Some(Arc::clone(&info));
        info
    }

    /// Navigation `nav` to `url` failed with `error`: the tab shows its error page. `None`
    /// for a cancelled one, which leaves the tab where it was.
    pub(crate) fn fail(&self, nav: NavigationId, url: &Url, error: &LoadError) -> Option<Arc<SecurityInfo>> {
        let mut state = self.state.lock();
        let hop = take_hop(&mut state, nav);
        let outcome = match error {
            LoadError::Cancelled { .. } => return None,
            LoadError::Tls {
                kind,
                host,
                fingerprint,
                ..
            } => SecurityOutcome::TlsFailed {
                kind: *kind,
                host: host.clone(),
                fingerprint: *fingerprint,
            },
            LoadError::Blocked { reason } => SecurityOutcome::Blocked { reason: *reason },
            _ => SecurityOutcome::Failed,
        };
        let url = hop.url.unwrap_or_else(|| url.clone());
        let info = Arc::new(SecurityInfo {
            transport: transport(&url),
            url,
            outcome,
            upgrade: hop.upgrade,
            source: hop.source,
            subresources: SubresourceSummary::default(),
        });
        state.hosts.clear();
        state.current = Some(Arc::clone(&info));
        Some(info)
    }

    /// Navigation `nav` ended without changing what the tab shows (a download).
    pub(crate) fn abandon(&self, nav: NavigationId) {
        take_hop(&mut self.state.lock(), nav);
    }

    /// Fold one of navigation `nav`'s document events into its hop.
    fn record_hop(&self, nav: NavigationId, ev: &NetEvent) {
        let mut state = self.state.lock();
        // A newer navigation's events replace whatever an abandoned one left.
        if state.pending.as_ref().is_none_or(|(id, _)| *id != nav) {
            state.pending = Some((nav, Hop::default()));
        }
        let Some((_, hop)) = state.pending.as_mut() else {
            return;
        };
        match ev {
            NetEvent::Accepted { url, .. } => hop.url = Some(url.clone()),
            NetEvent::Redirected { to, .. } => {
                *hop = Hop {
                    url: Some(to.clone()),
                    ..Hop::default()
                }
            }
            NetEvent::SecurityUpgraded { to, reason, .. } => {
                hop.url = Some(to.clone());
                hop.upgrade = Some(UpgradeReason::from_net(*reason));
            }
            NetEvent::Cache { outcome, .. } => match CacheOutcome::from_net(*outcome) {
                CacheOutcome::Hit | CacheOutcome::Stale => hop.source = ResponseSource::Cache { revalidated: false },
                CacheOutcome::Validated => hop.source = ResponseSource::Cache { revalidated: true },
                _ => {}
            },
            _ => {}
        }
    }

    /// Count a subresource's end into the current document. `true` when the summary changed.
    fn count_subresource(&self, ev: &NetEvent) -> bool {
        let (url, tls_failed) = match ev {
            NetEvent::Finished { url, .. } => (url, false),
            NetEvent::TlsFailed { url, .. } => (url, true),
            _ => return false,
        };
        let mut state = self.state.lock();
        let Some(current) = state.current.clone() else {
            return false;
        };
        let mut summary = current.subresources;
        match (tls_failed, transport(url)) {
            (true, _) => summary.failed_tls += 1,
            (false, Some(Transport::Encrypted)) => summary.over_tls += 1,
            (false, Some(Transport::Cleartext)) => summary.cleartext += 1,
            (false, None) => return false,
        }
        if let Some(host) = url.host_str() {
            state.hosts.insert(host.to_string());
            summary.hosts = state.hosts.len() as u32;
        }
        state.current = Some(Arc::new(SecurityInfo {
            subresources: summary,
            ..(*current).clone()
        }));
        true
    }

    /// Tell the embedder the summary changed, at most once per [`ANNOUNCE_INTERVAL`]; a change
    /// inside the interval is announced when it ends. Without a `runtime` to wait on, every
    /// change is announced at once.
    fn announce(self: &Arc<Self>, tab_id: TabId, event_tx: &EventChannel, runtime: Option<&Handle>) {
        let wait = {
            let mut state = self.state.lock();
            if state.announce_due {
                return;
            }
            match state.announced.map(|at| at.elapsed()) {
                Some(since) if since < ANNOUNCE_INTERVAL && runtime.is_some() => {
                    state.announce_due = true;
                    ANNOUNCE_INTERVAL - since
                }
                _ => {
                    state.announced = Some(Instant::now());
                    Duration::ZERO
                }
            }
        };
        let Some(runtime) = runtime.filter(|_| !wait.is_zero()) else {
            let _ = event_tx.send(EngineEvent::SecurityChanged { tab_id });
            return;
        };
        let slot = Arc::clone(self);
        let event_tx = event_tx.clone();
        let _entered = runtime.enter();
        spawn_named("security-announce", async move {
            tokio::time::sleep(wait).await;
            {
                let mut state = slot.state.lock();
                state.announce_due = false;
                state.announced = Some(Instant::now());
            }
            let _ = event_tx.send(EngineEvent::SecurityChanged { tab_id });
        });
    }
}

/// Take navigation `nav`'s hop, or an empty one when it had none (nothing was fetched).
fn take_hop(state: &mut SlotState, nav: NavigationId) -> Hop {
    match state.pending.take() {
        Some((id, hop)) if id == nav => hop,
        other => {
            state.pending = other;
            Hop::default()
        }
    }
}

/// What a request is to its tab's security state.
pub(crate) enum Role {
    /// The document request of a navigation.
    Document(NavigationId),
    /// Anything the committed document loads.
    Subresource,
}

/// [`NetObserver`] decorator that feeds a request's events into its tab's [`SecuritySlot`]
/// and forwards them on unchanged.
pub(crate) struct SecurityRecorder {
    inner: Arc<dyn NetObserver + Send + Sync>,
    slot: Arc<SecuritySlot>,
    role: Role,
    tab_id: TabId,
    event_tx: EventChannel,
    /// The runtime the request was made on. Events can arrive off it, on the network
    /// process's reader thread, and a delayed announcement still needs a timer.
    runtime: Option<Handle>,
}

impl SecurityRecorder {
    /// Wrap `inner` for a request of `tab_id`'s; `inner` as it is when the tab is gone.
    pub(crate) fn wrap(
        inner: Arc<dyn NetObserver + Send + Sync>,
        tab_id: TabId,
        role: Role,
        event_tx: &EventChannel,
    ) -> Arc<dyn NetObserver + Send + Sync> {
        let Some(slot) = SECURITY.get(tab_id) else {
            return inner;
        };
        Arc::new(Self {
            inner,
            slot,
            role,
            tab_id,
            event_tx: event_tx.clone(),
            runtime: Handle::try_current().ok(),
        })
    }
}

impl NetObserver for SecurityRecorder {
    fn on_event(&self, ev: NetEvent) {
        match self.role {
            Role::Document(nav) => self.slot.record_hop(nav, &ev),
            Role::Subresource => {
                if self.slot.count_subresource(&ev) {
                    self.slot.announce(self.tab_id, &self.event_tx, self.runtime.as_ref());
                }
            }
        }
        self.inner.on_event(ev);
    }

    fn body_capture_limit(&self, headers: &http::HeaderMap, content_length: Option<u64>) -> Option<usize> {
        self.inner.body_capture_limit(headers, content_length)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::emitter::null_emitter::NullEmitter;
    use gosub_sonar::net::cache::CacheOutcome as NetCacheOutcome;
    use gosub_sonar::net::tls::{TlsError, TlsErrorKind};
    use gosub_sonar::net::types::UpgradeReason as NetUpgradeReason;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    /// A tab with a registered slot, and a recorder for one of its requests.
    fn tab(role: Role) -> (TabId, Arc<SecuritySlot>, Arc<dyn NetObserver + Send + Sync>) {
        let tab_id = TabId::new();
        let slot = Arc::new(SecuritySlot::default());
        SECURITY.register(tab_id, Arc::clone(&slot));
        let (event_tx, _) = tokio::sync::broadcast::channel(16);
        let recorder = SecurityRecorder::wrap(Arc::new(NullEmitter), tab_id, role, &event_tx);
        (tab_id, slot, recorder)
    }

    fn accepted(s: &str) -> NetEvent {
        NetEvent::Accepted {
            url: url(s),
            joined: false,
        }
    }

    fn finished(s: &str) -> NetEvent {
        NetEvent::Finished {
            received_bytes: 0,
            elapsed: Duration::ZERO,
            url: url(s),
        }
    }

    fn tls_error(host: &str) -> LoadError {
        LoadError::Tls {
            kind: TlsFailureKind::UnknownIssuer,
            host: host.into(),
            fingerprint: Some([7; 32]),
            message: "self-signed".into(),
        }
    }

    #[test]
    fn a_page_over_http_is_cleartext() {
        let nav = NavigationId::new();
        let (tab_id, slot, rec) = tab(Role::Document(nav));
        rec.on_event(accepted("http://site.test/"));
        let info = slot.commit(nav, &url("http://site.test/"));
        assert_eq!(info.transport, Some(Transport::Cleartext));
        assert_eq!(info.outcome, SecurityOutcome::Loaded);
        assert_eq!(info.source, ResponseSource::Network);
        assert_eq!(info.upgrade, None);
        assert_eq!(slot.get(), Some(info));
        SECURITY.remove(tab_id);
    }

    #[test]
    fn a_page_over_https_is_encrypted() {
        let nav = NavigationId::new();
        let (tab_id, slot, rec) = tab(Role::Document(nav));
        rec.on_event(accepted("https://site.test/"));
        let info = slot.commit(nav, &url("https://site.test/"));
        assert_eq!(info.transport, Some(Transport::Encrypted));
        assert_eq!(info.outcome, SecurityOutcome::Loaded);
        SECURITY.remove(tab_id);
    }

    /// Nothing fetched (an internal page): no transport, and nothing from an earlier hop.
    #[test]
    fn a_local_page_has_no_transport() {
        let (tab_id, slot, _) = tab(Role::Subresource);
        let info = slot.commit(NavigationId::new(), &url("about:blank"));
        assert_eq!(info.transport, None);
        assert_eq!(info.outcome, SecurityOutcome::Loaded);
        SECURITY.remove(tab_id);
    }

    #[test]
    fn a_tls_failure_on_the_final_hop() {
        let nav = NavigationId::new();
        let (tab_id, slot, rec) = tab(Role::Document(nav));
        rec.on_event(accepted("https://bad.test/"));
        let info = slot
            .fail(nav, &url("https://bad.test/"), &tls_error("bad.test"))
            .expect("a failure is shown");
        assert_eq!(info.url, url("https://bad.test/"));
        assert_eq!(info.transport, Some(Transport::Encrypted));
        assert_eq!(
            info.outcome,
            SecurityOutcome::TlsFailed {
                kind: TlsFailureKind::UnknownIssuer,
                host: "bad.test".into(),
                fingerprint: Some([7; 32]),
            }
        );
        SECURITY.remove(tab_id);
    }

    /// The failure names the hop that failed, not the URL the user asked for.
    #[test]
    fn a_tls_failure_on_a_redirect_hop_names_the_target() {
        let nav = NavigationId::new();
        let (tab_id, slot, rec) = tab(Role::Document(nav));
        rec.on_event(accepted("http://site.test/"));
        rec.on_event(NetEvent::Redirected {
            from: url("http://site.test/"),
            to: url("https://bad.test/landing"),
            status: 302,
        });
        rec.on_event(NetEvent::TlsFailed {
            url: url("https://bad.test/landing"),
            error: TlsError {
                kind: TlsErrorKind::UnknownIssuer,
                host: "bad.test".into(),
                message: "self-signed".into(),
                certificate: None,
                fingerprint: None,
            },
        });
        let info = slot
            .fail(nav, &url("http://site.test/"), &tls_error("bad.test"))
            .expect("a failure is shown");
        assert_eq!(info.url, url("https://bad.test/landing"));
        assert_eq!(info.transport, Some(Transport::Encrypted));
        assert!(matches!(&info.outcome, SecurityOutcome::TlsFailed { host, .. } if host == "bad.test"));
        SECURITY.remove(tab_id);
    }

    #[test]
    fn a_blocked_navigation_is_not_a_tls_failure() {
        let nav = NavigationId::new();
        let (tab_id, slot, _) = tab(Role::Document(nav));
        let error = LoadError::Blocked {
            reason: BlockReason::MixedContent,
        };
        let info = slot.fail(nav, &url("http://site.test/"), &error).unwrap();
        assert_eq!(
            info.outcome,
            SecurityOutcome::Blocked {
                reason: BlockReason::MixedContent
            }
        );
        let error = LoadError::Connect {
            message: "refused".into(),
        };
        let info = slot.fail(nav, &url("http://site.test/"), &error).unwrap();
        assert_eq!(info.outcome, SecurityOutcome::Failed);
        SECURITY.remove(tab_id);
    }

    /// A cancelled navigation leaves the tab on what it showed.
    #[test]
    fn a_cancelled_navigation_changes_nothing() {
        let nav = NavigationId::new();
        let (tab_id, slot, _) = tab(Role::Document(nav));
        let shown = slot.commit(NavigationId::new(), &url("https://site.test/"));
        let next = NavigationId::new();
        let error = LoadError::Cancelled { message: "new".into() };
        assert_eq!(slot.fail(next, &url("http://other.test/"), &error), None);
        assert_eq!(slot.get(), Some(shown));
        SECURITY.remove(tab_id);
    }

    #[test]
    fn a_cache_hit_and_a_revalidation() {
        let nav = NavigationId::new();
        let (tab_id, slot, rec) = tab(Role::Document(nav));
        rec.on_event(accepted("https://site.test/"));
        rec.on_event(NetEvent::Cache {
            url: url("https://site.test/"),
            outcome: NetCacheOutcome::Hit,
        });
        let info = slot.commit(nav, &url("https://site.test/"));
        assert_eq!(info.source, ResponseSource::Cache { revalidated: false });

        let nav = NavigationId::new();
        let rec = SecurityRecorder::wrap(
            Arc::new(NullEmitter),
            tab_id,
            Role::Document(nav),
            &tokio::sync::broadcast::channel(1).0,
        );
        rec.on_event(NetEvent::Cache {
            url: url("https://site.test/"),
            outcome: NetCacheOutcome::Validated,
        });
        let info = slot.commit(nav, &url("https://site.test/"));
        assert_eq!(info.source, ResponseSource::Cache { revalidated: true });
        SECURITY.remove(tab_id);
    }

    #[test]
    fn an_hsts_upgrade_is_kept_until_a_redirect() {
        let nav = NavigationId::new();
        let (tab_id, slot, rec) = tab(Role::Document(nav));
        rec.on_event(accepted("http://site.test/"));
        rec.on_event(NetEvent::SecurityUpgraded {
            from: url("http://site.test/"),
            to: url("https://site.test/"),
            reason: NetUpgradeReason::Hsts,
        });
        let info = slot.commit(nav, &url("https://site.test/"));
        assert_eq!(info.upgrade, Some(UpgradeReason::Hsts));
        assert_eq!(info.transport, Some(Transport::Encrypted));

        // The upgraded hop redirected on: the final hop was not upgraded.
        let nav = NavigationId::new();
        let rec = SecurityRecorder::wrap(
            Arc::new(NullEmitter),
            tab_id,
            Role::Document(nav),
            &tokio::sync::broadcast::channel(1).0,
        );
        rec.on_event(NetEvent::SecurityUpgraded {
            from: url("http://site.test/"),
            to: url("https://site.test/"),
            reason: NetUpgradeReason::Hsts,
        });
        rec.on_event(NetEvent::Redirected {
            from: url("https://site.test/"),
            to: url("https://www.site.test/"),
            status: 301,
        });
        let info = slot.commit(nav, &url("https://www.site.test/"));
        assert_eq!(info.upgrade, None);
        SECURITY.remove(tab_id);
    }

    /// An older navigation's leftovers never reach a newer one.
    #[test]
    fn a_hop_belongs_to_its_own_navigation() {
        let old = NavigationId::new();
        let (tab_id, slot, rec) = tab(Role::Document(old));
        rec.on_event(NetEvent::SecurityUpgraded {
            from: url("http://site.test/"),
            to: url("https://site.test/"),
            reason: NetUpgradeReason::Hsts,
        });
        let info = slot.commit(NavigationId::new(), &url("http://other.test/"));
        assert_eq!(info.upgrade, None);
        SECURITY.remove(tab_id);
    }

    #[test]
    fn subresources_count_into_the_committed_document() {
        let (tab_id, slot, sub) = tab(Role::Subresource);
        // Nothing committed yet: nothing to count into.
        sub.on_event(finished("https://cdn.test/a.css"));
        assert_eq!(slot.get(), None);

        slot.commit(NavigationId::new(), &url("https://site.test/"));
        sub.on_event(finished("https://cdn.test/a.css"));
        sub.on_event(finished("https://cdn.test/b.js"));
        sub.on_event(finished("http://img.test/c.png"));
        sub.on_event(NetEvent::TlsFailed {
            url: url("https://bad.test/d.js"),
            error: TlsError {
                kind: TlsErrorKind::Expired,
                host: "bad.test".into(),
                message: "expired".into(),
                certificate: None,
                fingerprint: None,
            },
        });
        sub.on_event(finished("data:text/plain,x"));
        let summary = slot.get().unwrap().subresources;
        assert_eq!(
            (summary.over_tls, summary.cleartext, summary.failed_tls, summary.hosts),
            (2, 1, 1, 3)
        );

        // A new document starts from nothing.
        let info = slot.commit(NavigationId::new(), &url("https://site.test/next"));
        assert_eq!(info.subresources, SubresourceSummary::default());
        SECURITY.remove(tab_id);
    }

    /// The first change is announced at once, the rest of a burst once when the interval
    /// ends.
    #[tokio::test]
    async fn subresource_changes_are_announced_throttled() {
        let tab_id = TabId::new();
        let slot = Arc::new(SecuritySlot::default());
        SECURITY.register(tab_id, Arc::clone(&slot));
        let (event_tx, mut events) = tokio::sync::broadcast::channel(16);
        let sub = SecurityRecorder::wrap(Arc::new(NullEmitter), tab_id, Role::Subresource, &event_tx);
        slot.commit(NavigationId::new(), &url("https://site.test/"));

        for _ in 0..5 {
            sub.on_event(finished("https://cdn.test/a.css"));
        }
        assert!(matches!(events.try_recv(), Ok(EngineEvent::SecurityChanged { .. })));
        assert!(events.try_recv().is_err(), "the burst was announced at once");

        tokio::time::sleep(ANNOUNCE_INTERVAL * 2).await;
        assert!(matches!(events.try_recv(), Ok(EngineEvent::SecurityChanged { .. })));
        assert!(events.try_recv().is_err());
        assert_eq!(slot.get().unwrap().subresources.over_tls, 5);
        SECURITY.remove(tab_id);
    }

    /// The network process's reader thread is not on the runtime; the burst is still
    /// announced when the interval ends.
    #[tokio::test]
    async fn changes_from_a_plain_thread_are_announced_throttled() {
        let tab_id = TabId::new();
        let slot = Arc::new(SecuritySlot::default());
        SECURITY.register(tab_id, Arc::clone(&slot));
        let (event_tx, mut events) = tokio::sync::broadcast::channel(16);
        let sub = SecurityRecorder::wrap(Arc::new(NullEmitter), tab_id, Role::Subresource, &event_tx);
        slot.commit(NavigationId::new(), &url("https://site.test/"));

        std::thread::spawn(move || {
            for _ in 0..5 {
                sub.on_event(finished("https://cdn.test/a.css"));
            }
        })
        .join()
        .unwrap();
        assert!(matches!(events.try_recv(), Ok(EngineEvent::SecurityChanged { .. })));
        assert!(events.try_recv().is_err());

        tokio::time::sleep(ANNOUNCE_INTERVAL * 2).await;
        assert!(matches!(events.try_recv(), Ok(EngineEvent::SecurityChanged { .. })));
        assert!(events.try_recv().is_err());
        SECURITY.remove(tab_id);
    }

    /// With no runtime at all, nothing waits and nothing is swallowed.
    #[test]
    fn without_a_runtime_every_change_is_announced() {
        let (tab_id, slot, _) = tab(Role::Subresource);
        let (event_tx, mut events) = tokio::sync::broadcast::channel(16);
        let sub = SecurityRecorder::wrap(Arc::new(NullEmitter), tab_id, Role::Subresource, &event_tx);
        slot.commit(NavigationId::new(), &url("https://site.test/"));

        for _ in 0..3 {
            sub.on_event(finished("https://cdn.test/a.css"));
        }
        for _ in 0..3 {
            assert!(matches!(events.try_recv(), Ok(EngineEvent::SecurityChanged { .. })));
        }
        SECURITY.remove(tab_id);
    }

    /// A request of a tab that is gone is reported as before, and recorded nowhere.
    #[test]
    fn a_closed_tab_gets_no_recorder() {
        let (event_tx, _) = tokio::sync::broadcast::channel(1);
        let inner: Arc<dyn NetObserver + Send + Sync> = Arc::new(NullEmitter);
        let wrapped = SecurityRecorder::wrap(Arc::clone(&inner), TabId::new(), Role::Subresource, &event_tx);
        assert!(Arc::ptr_eq(&inner, &wrapped));
    }
}
