//! The broker's side of the cookie vault: spawn it, talk to it, persist what
//! it reports, and stand in for it as a [`CookieJar`] for everything in the
//! engine that expects one.

use crate::cookie_vault::protocol::{CookieScope, FromVault, SameSite, Tag, ToVault, VAULT_ROLE};
use crate::engine::cookies::{CookieJar, CookieJarHandle, CookieStoreHandle, DefaultCookieJar, SameSiteContext};
use crate::engine::zone::ZoneId;
use gosub_ipc::{Endpoint, EndpointTx};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;
use url::Url;

/// How long a query waits for the vault. Generous: the vault does no I/O, so
/// anything near this means it is gone.
const REPLY_TIMEOUT: Duration = Duration::from_secs(10);
/// How long to wait for the child to identify itself.
const READY_TIMEOUT: Duration = Duration::from_secs(10);
/// How long shutdown waits before killing a lingering vault.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(2);

/// A reply the reader thread routed to a waiting query.
enum Reply {
    Cookies(Option<String>),
    All(Vec<(String, String)>),
}

/// A running vault and the broker's link to it.
pub struct CookieVault {
    tx: Mutex<EndpointTx>,
    pending: Arc<Mutex<HashMap<Tag, mpsc::SyncSender<Reply>>>>,
    next_tag: AtomicU64,
    /// The zones' persisting stores, for the snapshots the vault sends back.
    stores: Arc<Mutex<HashMap<String, (ZoneId, CookieStoreHandle)>>>,
    child: Mutex<Option<gosub_sandbox::spawn::Child>>,
    /// Whether a network process was given its own line to this vault.
    net_linked: bool,
}

impl std::fmt::Debug for CookieVault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CookieVault").finish_non_exhaustive()
    }
}

/// The network process's end of its direct line to the vault, to hand over at
/// its spawn.
pub struct NetVaultLink(pub gosub_ipc::channel::Channel);

impl CookieVault {
    /// Re-exec this binary as the vault. With `with_net_link`, a second channel
    /// is created whose far end the network process is to inherit, so cookie
    /// values on requests never pass through this process.
    pub fn spawn(with_net_link: bool) -> anyhow::Result<(Self, Option<NetVaultLink>)> {
        if crate::child_process::is_child_process() {
            anyhow::bail!(
                "this process was started as an engine child role but is running embedder startup, \
                 which means gosub_engine::child_process::dispatch() was not called at the top of \
                 main(); refusing to spawn further processes"
            );
        }

        let exe = std::env::current_exe()?;
        let (ours, theirs) = gosub_ipc::channel::Channel::pair()?;
        let net_pair = if with_net_link {
            Some(gosub_ipc::channel::Channel::pair()?)
        } else {
            None
        };

        // The vault's end of the network line rides along as an extra inherited
        // fd, named in argv before the primary link (which `spawn` appends).
        let net_spec = net_pair.as_ref().map(|(vault_end, _)| vault_end.to_argv());
        let mut args: Vec<&str> = vec![crate::child_process::ROLE_FLAG, VAULT_ROLE];
        if let Some(spec) = net_spec.as_deref() {
            args.push(spec);
        }
        let extra_fds: Vec<i32> = net_pair.iter().map(|(vault_end, _)| vault_end.raw()).collect();

        let child = gosub_sandbox::spawn::spawn(
            &exe,
            &args,
            theirs,
            // No PID namespace: the vault serves its two links on two threads,
            // and a process that unshared its PID namespace cannot create one.
            gosub_sandbox::NamespaceIsolation::NoPidNamespace,
            gosub_sandbox::spawn::ContainerProfile {
                name: "gosub-vault",
                internet: false,
                fs_grant: None,
                data_limit: None,
                extra_fds: &extra_fds,
            },
        )?;
        if let Err(e) = gosub_sandbox::confine_spawned_child(&child) {
            log::warn!("could not confine the vault process: {e}");
        }
        let net_link = net_pair.map(|(vault_end, net_end)| {
            drop(vault_end); // the child holds its copy
            NetVaultLink(net_end)
        });

        let endpoint = Endpoint::from_channel(ours)?;
        let (mut tx, mut rx) = endpoint.split();
        let _ = tx.set_write_timeout(Some(REPLY_TIMEOUT));

        let pending: Arc<Mutex<HashMap<Tag, mpsc::SyncSender<Reply>>>> = Arc::new(Mutex::new(HashMap::new()));
        let stores: Arc<Mutex<HashMap<String, (ZoneId, CookieStoreHandle)>>> = Arc::new(Mutex::new(HashMap::new()));
        let (ready_tx, ready_rx) = mpsc::sync_channel::<()>(1);

        let waiters = Arc::clone(&pending);
        let persist = Arc::clone(&stores);
        std::thread::Builder::new()
            .name("cookie-vault-reader".into())
            .spawn(move || {
                while let Ok(msg) = rx.recv::<FromVault>() {
                    match msg {
                        FromVault::Pong => {
                            let _ = ready_tx.send(());
                        }
                        FromVault::Cookies { tag, header } => {
                            if let Some(waiter) = waiters.lock().remove(&tag) {
                                let _ = waiter.send(Reply::Cookies(header));
                            }
                        }
                        FromVault::All { tag, cookies } => {
                            if let Some(waiter) = waiters.lock().remove(&tag) {
                                let _ = waiter.send(Reply::All(cookies));
                            }
                        }
                        FromVault::Snapshot { zone, jar } => persist_snapshot(&persist, &zone, jar),
                    }
                }
                waiters.lock().clear();
            })?;

        let vault = Self {
            tx: Mutex::new(tx),
            pending,
            next_tag: AtomicU64::new(1),
            stores,
            child: Mutex::new(Some(child)),
            net_linked: net_link.is_some(),
        };
        vault.tx.lock().send(&ToVault::Ping)?;
        if ready_rx.recv_timeout(READY_TIMEOUT).is_err() {
            vault.kill();
            anyhow::bail!("the spawned process did not answer as a cookie vault within {READY_TIMEOUT:?}");
        }
        Ok((vault, net_link))
    }

    /// Whether the network process talks to this vault directly.
    pub fn net_linked(&self) -> bool {
        self.net_linked
    }

    /// Start holding `zone`'s cookies. Seeded from `store`'s persisted state
    /// when there is one, which also receives every later snapshot.
    pub fn open_zone(&self, zone: ZoneId, store: Option<CookieStoreHandle>) {
        let key = zone.to_string();
        let snapshot = store.as_ref().and_then(|store| persisted_snapshot(store, zone));
        if let Some(store) = store {
            self.stores.lock().insert(key.clone(), (zone, store));
        }
        let _ = self.tx.lock().send(&ToVault::OpenZone { zone: key, snapshot });
    }

    pub fn close_zone(&self, zone: ZoneId) {
        let key = zone.to_string();
        self.stores.lock().remove(&key);
        let _ = self.tx.lock().send(&ToVault::CloseZone { zone: key });
    }

    fn ask(&self, build: impl FnOnce(Tag) -> ToVault) -> Option<Reply> {
        let tag = self.next_tag.fetch_add(1, Ordering::Relaxed);
        let (reply_tx, reply_rx) = mpsc::sync_channel::<Reply>(1);
        self.pending.lock().insert(tag, reply_tx);
        if self.tx.lock().send(&build(tag)).is_err() {
            self.pending.lock().remove(&tag);
            return None;
        }
        match reply_rx.recv_timeout(REPLY_TIMEOUT) {
            Ok(reply) => Some(reply),
            Err(_) => {
                self.pending.lock().remove(&tag);
                log::warn!("the cookie vault did not answer");
                None
            }
        }
    }

    /// The `Cookie` header for a request.
    pub fn get(&self, scope: CookieScope, url: &Url, visible_only: bool) -> Option<String> {
        match self.ask(|tag| ToVault::Get {
            tag,
            scope,
            url: url.to_string(),
            visible_only,
        })? {
            Reply::Cookies(header) => header,
            Reply::All(_) => None,
        }
    }

    pub fn store(&self, zone: &str, url: &Url, top_level: Option<&Url>, set_cookie: Vec<String>) {
        if set_cookie.is_empty() {
            return;
        }
        let _ = self.tx.lock().send(&ToVault::Store {
            zone: zone.to_string(),
            url: url.to_string(),
            top_level: top_level.map(|u| u.to_string()),
            set_cookie,
        });
    }

    fn get_all(&self, zone: &str) -> Vec<(String, String)> {
        match self.ask(|tag| ToVault::GetAll {
            tag,
            zone: zone.to_string(),
        }) {
            Some(Reply::All(cookies)) => cookies,
            _ => Vec::new(),
        }
    }

    fn tell(&self, msg: ToVault) {
        let _ = self.tx.lock().send(&msg);
    }

    /// Ask the vault to exit, then make sure it did.
    pub fn shutdown(&self) {
        let _ = self.tx.lock().send(&ToVault::Shutdown);
        let Some(mut child) = self.child.lock().take() else {
            return;
        };
        let deadline = std::time::Instant::now() + SHUTDOWN_GRACE;
        while std::time::Instant::now() < deadline {
            match child.try_wait() {
                Ok(true) => return,
                Ok(false) => std::thread::sleep(Duration::from_millis(20)),
                Err(_) => break,
            }
        }
        let _ = child.kill();
        let _ = child.wait();
    }

    fn kill(&self) {
        if let Some(mut child) = self.child.lock().take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for CookieVault {
    fn drop(&mut self) {
        self.kill();
    }
}

/// What the store has for `zone`: the persisting jar's current state.
fn persisted_snapshot(store: &CookieStoreHandle, zone: ZoneId) -> Option<DefaultCookieJar> {
    let jar = store.jar_for(zone)?;
    let guard = jar.read();
    if let Some(persisting) = guard
        .as_any()
        .downcast_ref::<crate::engine::cookies::PersistentCookieJar>()
    {
        let inner = persisting.inner.read();
        return inner.as_any().downcast_ref::<DefaultCookieJar>().cloned();
    }
    guard.as_any().downcast_ref::<DefaultCookieJar>().cloned()
}

/// A snapshot from the vault: written through the zone's store, and mirrored
/// into the store's own cached jar so its `release_zone`/`persist_all` paths
/// (which snapshot that cache) cannot resurrect stale state.
fn persist_snapshot(stores: &Mutex<HashMap<String, (ZoneId, CookieStoreHandle)>>, zone: &str, jar: DefaultCookieJar) {
    let Some((zone_id, store)) = stores.lock().get(zone).cloned() else {
        return;
    };
    if let Some(cached) = store.jar_for(zone_id) {
        let mut guard = cached.write();
        if let Some(persisting) = guard
            .as_any_mut()
            .downcast_mut::<crate::engine::cookies::PersistentCookieJar>()
        {
            let mut inner = persisting.inner.write();
            if let Some(inner) = inner.as_any_mut().downcast_mut::<DefaultCookieJar>() {
                *inner = jar.clone();
            }
        } else if let Some(inner) = guard.as_any_mut().downcast_mut::<DefaultCookieJar>() {
            *inner = jar.clone();
        }
    }
    store.persist_zone_from_snapshot(zone_id, &jar);
}

/// A [`CookieJar`] whose cookies live in the vault: every method is a message
/// across. This is what a vaulted zone's tabs hold as their jar, so the rest
/// of the engine (and the embedder API) is unchanged.
pub struct VaultCookieJar {
    vault: Arc<CookieVault>,
    zone: String,
}

impl VaultCookieJar {
    pub fn new(vault: Arc<CookieVault>, zone: ZoneId) -> Self {
        Self {
            vault,
            zone: zone.to_string(),
        }
    }

    /// Who this jar answers for, for a request that carries its scope instead.
    pub fn zone(&self) -> &str {
        &self.zone
    }

    pub fn handle(self) -> CookieJarHandle {
        self.into()
    }
}

impl std::fmt::Debug for VaultCookieJar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VaultCookieJar").field("zone", &self.zone).finish()
    }
}

impl CookieJar for VaultCookieJar {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn store_response_cookies(&mut self, url: &Url, headers: &http::HeaderMap, top_level: Option<&Url>) {
        let set_cookie: Vec<String> = headers
            .get_all(http::header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok().map(str::to_string))
            .collect();
        self.vault.store(&self.zone, url, top_level, set_cookie);
    }

    fn get_request_cookies(&self, url: &Url, top_level: Option<&Url>, samesite: SameSiteContext) -> Option<String> {
        let scope = CookieScope {
            zone: self.zone.clone(),
            top_level: top_level.map(|u| u.to_string()),
            samesite: SameSite::from(samesite),
        };
        self.vault.get(scope, url, false)
    }

    fn clear(&mut self) {
        self.vault.tell(ToVault::Clear {
            zone: self.zone.clone(),
        });
    }

    fn get_all_cookies(&self) -> Vec<(Url, String)> {
        self.vault
            .get_all(&self.zone)
            .into_iter()
            .filter_map(|(url, cookie)| Url::parse(&url).ok().map(|url| (url, cookie)))
            .collect()
    }

    fn remove_cookie(&mut self, url: &Url, cookie_name: &str) {
        self.vault.tell(ToVault::Remove {
            zone: self.zone.clone(),
            url: url.to_string(),
            name: cookie_name.to_string(),
        });
    }

    fn remove_cookies_for_url(&mut self, url: &Url) {
        self.vault.tell(ToVault::RemoveForUrl {
            zone: self.zone.clone(),
            url: url.to_string(),
        });
    }

    fn purge_expired(&mut self) {
        self.vault.tell(ToVault::PurgeExpired {
            zone: self.zone.clone(),
        });
    }
}
