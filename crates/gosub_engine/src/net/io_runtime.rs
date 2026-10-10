use crate::cookies::SameSiteContext;
use crate::engine::events::IoCommand;
use crate::engine::types::IoChannel;
use crate::engine::EngineContext;
use crate::net::fetcher::{fetcher_config_from, reach_config, EngineNetContext, Fetcher};
use crate::net::req_ref_tracker::{RequestRefTracker, RequestReference, REF_REGISTRY};
use crate::net::ssrf::AddressSpace;
use crate::net::tab_identity::{TabIdentity, TabIdentityRegistry};
use crate::net::types::{FetchHandle, FetchRequest, FetchResult};
use crate::tab::TabId;
use crate::util::spawn_named;
use crate::zone::ZoneId;
use crate::EngineError;
use dashmap::DashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::instrument;

/// Handle to the I/O runtime thread and its submission channel.
pub struct IoHandle {
    /// Channel to submit I/O requests
    tx_submit: IoChannel,
    /// Cancelled to signal global IO thread shutdown
    shutdown_token: CancellationToken,
    /// Join handle for shutdown sync
    join_handle: JoinHandle<()>,
}

impl IoHandle {
    pub async fn shutdown_zone(&self, zone_id: ZoneId) -> anyhow::Result<()> {
        let (tx, rx) = oneshot::channel();
        self.tx_submit
            .send(IoCommand::ShutdownZone { zone_id, reply_tx: tx })
            .map_err(|e| anyhow::anyhow!("send ShutdownZone failed: {e}"))?;
        // wait until the zone's scheduler has actually stopped
        rx.await.map_err(|e| anyhow::anyhow!("ShutdownZone ack failed: {e}"))?;
        Ok(())
    }

    #[instrument(name = "io.shutdown", level = "debug", skip(self))]
    pub async fn shutdown(self) {
        let IoHandle {
            tx_submit,
            shutdown_token,
            join_handle,
        } = self;

        log::trace!("signal: global shutdown -> I/O thread");
        shutdown_token.cancel();

        // Subscribers hold clones of this sender, so the channel only fully
        // closes once they drop theirs; the cancellation token is the real signal.
        log::trace!("signal: dropping our submit channel handle");
        drop(tx_submit);

        log::trace!("await: I/O thread join");
        match join_handle.await {
            Ok(()) => {
                log::debug!("I/O thread has exited cleanly");
            }
            Err(e) if e.is_cancelled() => {
                log::warn!("I/O driver task was cancelled during shutdown");
            }
            Err(e) if e.is_panic() => {
                log::error!("I/O driver task panicked during shutdown: {e:?}");
            }
            Err(e) => {
                log::warn!("I/O driver join error: {e:?}");
            }
        }
    }

    /// Get a clone of the submission channel (hand to zones/tabs).
    pub(crate) fn subscribe(&self) -> IoChannel {
        self.tx_submit.clone()
    }

    /// The escape audit in the network process; `None` without one.
    #[cfg(feature = "process-isolation")]
    pub async fn audit_net(&self) -> Option<gosub_sandbox::audit::AuditReport> {
        let (tx, rx) = oneshot::channel();
        self.tx_submit.send(IoCommand::AuditNet { reply_tx: tx }).ok()?;
        rx.await.ok().flatten()
    }

    /// The network process's pid; `None` without one.
    #[cfg(feature = "process-isolation")]
    pub async fn net_pid(&self) -> Option<u32> {
        let (tx, rx) = oneshot::channel();
        self.tx_submit.send(IoCommand::NetPid { reply_tx: tx }).ok()?;
        rx.await.ok().flatten()
    }
}

pub struct ZoneEntry {
    fetchers: ZoneFetchers,
    shutdown: CancellationToken,
    join: JoinHandle<()>,
}

/// A zone's fetchers, one per address space a document can be in (see
/// `net::ssrf`). Each has its own connection pool, on purpose: a pooled
/// connection is a resolution already made.
#[derive(Clone)]
pub struct ZoneFetchers {
    /// Navigations, and subresources of documents on this machine: reaches
    /// anything.
    loopback: Arc<Fetcher>,
    /// Subresources of documents from the local network: not loopback.
    local: Arc<Fetcher>,
    /// Subresources of public documents: neither loopback nor the local network.
    public: Arc<Fetcher>,
}

impl ZoneFetchers {
    /// The fetcher for requests that may reach `reach` and nothing more private.
    pub fn for_reach(&self, reach: AddressSpace) -> Arc<Fetcher> {
        match reach {
            AddressSpace::Loopback => self.loopback.clone(),
            AddressSpace::Local => self.local.clone(),
            AddressSpace::Public => self.public.clone(),
        }
    }
}

/// Routes I/O requests to per-zone fetchers, spawning them on first use.
pub struct IoRouter {
    /// Map of zone ID to zone entries
    zones: DashMap<ZoneId, ZoneEntry>,
    /// Shared engine context for event broadcasting and request tracking
    engine_ctx: Arc<EngineContext>,
    /// Observer factory for requests the engine serves itself (the `file://` scheme),
    /// so they emit the same resource events a gosub-sonar fetch would.
    local_ctx: EngineNetContext,
    /// The network process, if `security.network_process` is on and it started.
    /// One for the whole engine: it holds no per-zone state, and the connection
    /// pooling that *is* per-zone lives inside it.
    #[cfg(feature = "process-isolation")]
    net_process: Option<Arc<crate::net::process::client::NetProcess>>,
}

impl IoRouter {
    pub fn new(engine_ctx: Arc<EngineContext>) -> Self {
        let local_ctx = EngineNetContext {
            resource_tx: engine_ctx.resource_tx.clone(),
            event_tx: engine_ctx.event_tx.clone(),
            request_reference_map: engine_ctx.request_reference_map.clone(),
            request_ref_tracker: Arc::new(RequestRefTracker::new()),
            tab_identities: Arc::clone(&engine_ctx.tab_identities),
            reach: AddressSpace::Loopback,
        };
        #[cfg(feature = "process-isolation")]
        let net_process = start_net_process(&engine_ctx);

        Self {
            zones: DashMap::new(),
            engine_ctx,
            local_ctx,
            #[cfg(feature = "process-isolation")]
            net_process,
        }
    }

    /// Serve a `file://` request from disk on its own task (never through gosub-sonar,
    /// which only speaks http(s)). Policy lives in [`crate::net::file_loader`].
    fn serve_file_request(&self, req: FetchRequest, reply_tx: oneshot::Sender<crate::net::types::FetchResult>) {
        use gosub_sonar::net::fetcher_context::FetcherContext;
        let enabled = self.engine_ctx.config_store.get_bool("net.file.enabled");
        let observer = self
            .local_ctx
            .observer_for(req.reference, req.req_id, req.kind, req.initiator);
        spawn_named("file-loader", async move {
            let result = crate::net::file_loader::serve(&req, enabled, observer).await;
            let _ = reply_tx.send(result);
        });
    }

    /// Serve a `data:` request from the URL itself, for the same reason as
    /// [`Self::serve_file_request`]: gosub-sonar only speaks http(s). Policy lives in
    /// [`crate::net::data_url`] - which is to say there is none, the bytes being in the URL.
    fn serve_data_url_request(
        &self,
        req: FetchRequest,
        cancel: CancellationToken,
        reply_tx: oneshot::Sender<crate::net::types::FetchResult>,
    ) {
        use gosub_sonar::net::fetcher_context::FetcherContext;
        let observer = self
            .local_ctx
            .observer_for(req.reference, req.req_id, req.kind, req.initiator);
        spawn_named("data-url", async move {
            // A navigation cancelled while its request sat in the I/O queue must not go on to
            // emit network events for it. Decoding never awaits, so there is no point inside
            // `serve` to cancel at: the check belongs here, ahead of the first event. Dropping
            // `reply_tx` is how the zone fetcher reports a cancelled fetch too.
            if cancel.is_cancelled() {
                return;
            }
            let result = crate::net::data_url::serve(&req, observer).await;
            let _ = reply_tx.send(result);
        });
    }

    /// Who each tab is, for resolving cookies without trusting the request.
    pub fn tab_identities(&self) -> &TabIdentityRegistry {
        &self.engine_ctx.tab_identities
    }

    /// The network process, when this engine is running one.
    #[cfg(feature = "process-isolation")]
    pub fn net_process(&self) -> Option<Arc<crate::net::process::client::NetProcess>> {
        self.net_process.clone()
    }

    #[cfg(not(feature = "process-isolation"))]
    pub fn net_process(&self) -> Option<std::convert::Infallible> {
        None
    }

    /// The zone's fetchers, spawned on first use.
    pub fn get_or_spawn_zone_fetchers(&self, zone_id: ZoneId) -> Result<ZoneFetchers, EngineError> {
        if let Some(entry) = self.zones.get(&zone_id) {
            return Ok(entry.fetchers.clone());
        }

        let zone_shutdown = CancellationToken::new();

        // Read the settings store now rather than at engine start, so `net.*` overrides made
        // after `start()` apply to every zone that fetches from then on.
        let cfg = fetcher_config_from(&self.engine_ctx.config_store);
        let fetcher = |reach: AddressSpace| {
            let context = Arc::new(EngineNetContext {
                resource_tx: self.engine_ctx.resource_tx.clone(),
                event_tx: self.engine_ctx.event_tx.clone(),
                request_reference_map: self.engine_ctx.request_reference_map.clone(),
                request_ref_tracker: Arc::new(RequestRefTracker::new()),
                tab_identities: Arc::clone(&self.engine_ctx.tab_identities),
                reach,
            });
            Fetcher::new(reach_config(&cfg, reach), context)
                .map(Arc::new)
                .map_err(|e| EngineError::NetworkError(e.to_string()))
        };
        let fetchers = ZoneFetchers {
            loopback: fetcher(AddressSpace::Loopback)?,
            local: fetcher(AddressSpace::Local)?,
            public: fetcher(AddressSpace::Public)?,
        };

        let running = fetchers.clone();
        let cancel = zone_shutdown.clone();
        let title = format!("I/O Fetcher Zone {}", zone_id);
        let join_handle = spawn_named(&title, async move {
            tokio::join!(
                running.loopback.run(cancel.clone()),
                running.local.run(cancel.clone()),
                running.public.run(cancel)
            );
        });

        self.zones.insert(
            zone_id,
            ZoneEntry {
                fetchers: fetchers.clone(),
                shutdown: zone_shutdown,
                join: join_handle,
            },
        );

        Ok(fetchers)
    }

    #[instrument(
        name = "zone.shutdown",
        level = "debug",
        skip(self),
        fields(zone_id = %zone_id)
    )]
    pub async fn shutdown_zone(&self, zone_id: ZoneId) -> bool {
        log::trace!("removing zone fetcher");
        let Some((_, entry)) = self.zones.remove(&zone_id) else {
            return false;
        };

        // Shutdown the fetcher
        log::trace!("signal: shutdown to zone fetcher");
        entry.shutdown.cancel();
        // Wait for it to finish
        log::trace!("await: zone fetcher join");
        let _ = entry.join.await;

        true
    }

    /// Shutdown the IO thread
    #[instrument(name = "io.shutdown", level = "debug", skip(self))]
    pub async fn shutdown_all(self) {
        let mut tasks = Vec::new();

        let keys: Vec<_> = self.zones.iter().map(|kv| *kv.key()).collect();
        for zone_id in keys {
            if let Some((_, entry)) = self.zones.remove(&zone_id) {
                entry.shutdown.cancel();
                tasks.push(entry.join);
            }
        }

        log::trace!("await: all zone fetcher joins");
        for j in tasks {
            let _ = j.await;
        }

        // Stop the network process now, not when the last in-flight task
        // drops its `Arc` (up to the reply timeout later), and not from `Drop`
        // on a runtime worker: `shutdown` waits for the child with blocking
        // sleeps. It takes the child, so the later `Drop` has nothing to do.
        #[cfg(feature = "process-isolation")]
        if let Some(net) = self.net_process.clone() {
            let _ = tokio::task::spawn_blocking(move || net.shutdown()).await;
        }
    }
}

/// Start the network process if the setting asks for one.
#[cfg(feature = "process-isolation")]
fn start_net_process(engine_ctx: &Arc<EngineContext>) -> Option<Arc<crate::net::process::client::NetProcess>> {
    if !engine_ctx.config_store.get_bool("security.network_process") {
        return None;
    }

    // The vault's line for this process, if the engine started a vault with one.
    #[cfg(target_os = "linux")]
    let vault_line = engine_ctx
        .net_vault_link
        .lock()
        .take()
        .map(|link| crate::net::process::client::VaultLine(link.0));
    #[cfg(not(target_os = "linux"))]
    let vault_line = None;
    match crate::net::process::client::NetProcess::spawn(vault_line) {
        Ok(net) => {
            log::info!("network stack running in a separate, sandboxed process");
            let net = Arc::new(net);
            // A respawned vault hands this process a new line through here,
            // and a respawned network process hands the vault its end. Weak
            // that way round: the vault already holds this process.
            #[cfg(target_os = "linux")]
            if let (Some(vault), true) = (engine_ctx.cookie_vault.get(), net.vault_linked()) {
                let relinked = Arc::clone(&net);
                vault.on_relink(Box::new(move |line| {
                    relinked.relink_vault(crate::net::process::client::VaultLine(line.0));
                }));
                let vault = Arc::downgrade(vault);
                net.on_relink(Box::new(move |line| {
                    if let Some(vault) = vault.upgrade() {
                        vault.adopt_net_line(crate::cookie_vault::client::NetVaultLink(line));
                    }
                }));
            }
            Some(net)
        }
        Err(e) => {
            log::error!(
                "security.network_process is on but the network process could not start ({e}); \
                 falling back to in-process networking. Does this embedder call \
                 gosub_engine::child_process::dispatch() at the top of main()?"
            );
            None
        }
    }
}

/// Hand a request to the network process and answer the caller when it replies.
/// The wait runs as a task, not a thread, and follows `cancel`: an abandoned
/// navigation frees its slot and tells the child to drop the request.
///
/// Under a vault `grant` the network process asks the vault for each hop's
/// cookies itself; the grant is revoked once the reply is in, and the hops it
/// was used at are checked against the redirects the request reported. A
/// network process that claimed hops it never reported is lying about where
/// its requests went, and is killed (the next request respawns it).
#[cfg(feature = "process-isolation")]
fn dispatch_to_net_process(
    net: Arc<crate::net::process::client::NetProcess>,
    req: FetchRequest,
    reach: AddressSpace,
    grant: Option<Grant>,
    cancel: tokio_util::sync::CancellationToken,
    reply_tx: oneshot::Sender<FetchResult>,
    observer: Option<Arc<dyn crate::net::emitter::NetObserver + Send + Sync>>,
) {
    use crate::net::process::client::net_error;
    use crate::net::process::protocol::FetchOutcome;

    // The body preview switches live in this process; the child is told the
    // outcome, never asked to read them.
    let body_preview = crate::net::emitter::capture_body_previews().then(crate::net::emitter::body_capture_limit);

    let url = req.url.to_string();
    let method = req.method.as_str().to_string();
    let streaming = req.streaming;
    // No in-process fetcher emits the terminal event that would drop this.
    let req_id = req.req_id;
    let context = crate::net::process::protocol::RequestContext::of(&req);
    let mut headers = crate::net::process::protocol::flatten_headers(&req.headers);

    // The body crosses the link as plain bytes. Its Content-Type is folded into
    // the headers here, mirroring what gosub-sonar would inject at send time.
    let body = match req.body.as_ref() {
        None => None,
        Some(body) => match body.as_bytes() {
            Some(bytes) => {
                if !req.headers.contains_key(http::header::CONTENT_TYPE) {
                    if let Some(ct) = &body.content_type {
                        headers.push((http::header::CONTENT_TYPE.as_str().to_string(), ct.as_bytes().to_vec()));
                    }
                }
                Some(bytes.to_vec())
            }
            // A streaming body cannot cross the link; refuse rather than send
            // the request without it.
            None => {
                let _ = reply_tx.send(FetchResult::Error(net_error(format!(
                    "cannot send a streaming request body to the network process ({url})"
                ))));
                return;
            }
        },
    };

    #[cfg(target_os = "linux")]
    let cookies = grant.as_ref().map(|grant| grant.scope.clone());
    #[cfg(not(target_os = "linux"))]
    let cookies: Option<crate::net::process::protocol::CookieScope> = grant.map(|grant| match grant {});

    spawn_named("net-process-request", async move {
        let out = crate::net::process::client::Outbound {
            url,
            method,
            headers,
            body,
            reach,
            streaming,
            cookies,
            body_preview,
            context,
        };
        // A process that died is respawned before the request goes out;
        // blocking, so off the runtime's workers.
        if !net.is_alive() {
            let respawning = Arc::clone(&net);
            let _ = tokio::task::spawn_blocking(move || respawning.ensure_alive()).await;
        }
        let reply = net.fetch(out, &cancel, observer).await;
        crate::net::req_ref_tracker::REF_REGISTRY.forget_request(req_id);
        #[cfg(target_os = "linux")]
        if let Some(grant) = grant {
            // Only a reply the child sent carries every redirect it reported;
            // one the broker gave up waiting for proves nothing either way.
            let answered = reply.redirects.is_some();
            let redirects = reply.redirects.clone().unwrap_or_default();
            let unexplained = tokio::task::spawn_blocking(move || grant.vault.revoke(&grant.scope, redirects))
                .await
                .unwrap_or_default();
            if answered && !unexplained.is_empty() {
                log::error!(
                    "the network process used a cookie ticket at {unexplained:?}, which no redirect it \
                     reported explains; killing it"
                );
                net.condemn();
            }
        }
        let _ = reply_tx.send(match reply.outcome {
            FetchOutcome::Error(e) => FetchResult::Error(net_error(e)),
            _ => match crate::net::process::client::outcome_to_result(reply) {
                Ok(result) => result,
                Err(e) => FetchResult::Error(e),
            },
        });
    });
}

/// Whose cookies a request is about; nothing where no network process exists.
#[cfg(feature = "process-isolation")]
type CookieScope = crate::net::process::protocol::CookieScope;
#[cfg(not(feature = "process-isolation"))]
type CookieScope = std::convert::Infallible;

#[cfg(not(feature = "process-isolation"))]
fn dispatch_to_net_process(
    _net: std::convert::Infallible,
    _req: FetchRequest,
    _reach: AddressSpace,
    _grant: Option<Grant>,
    _cancel: tokio_util::sync::CancellationToken,
    _reply_tx: oneshot::Sender<FetchResult>,
    _observer: Option<Arc<dyn crate::net::emitter::NetObserver + Send + Sync>>,
) {
}

/// The scope a request carries instead of a cookie header: only when the
/// network process has its own line to the vault *and* this tab's jar is a
/// vault jar (an embedder-supplied jar stays the broker's business).
#[cfg(all(feature = "process-isolation", target_os = "linux"))]
fn cookie_scope_for(router: &IoRouter, identity: Option<&TabIdentity>, req: &FetchRequest) -> Option<CookieScope> {
    let identity = identity?;
    if !router.net_process().is_some_and(|net| net.vault_linked()) {
        return None;
    }
    let jar = identity.cookie_jar.read();
    let vaulted = jar
        .as_any()
        .downcast_ref::<crate::cookie_vault::client::VaultCookieJar>()?;
    Some(CookieScope {
        ticket: uuid::Uuid::new_v4().as_u128(),
        url: req.url.to_string(),
        zone: vaulted.zone().to_string(),
        top_level: identity.top_level.as_ref().map(|u| u.to_string()),
        site: identity
            .same_site_document(REF_REGISTRY.from_net(req.reference))
            .map(|u| u.to_string()),
        samesite: first_hop_context(identity, req).into(),
        navigation: req.kind == gosub_sonar::net::types::ResourceKind::Primary,
    })
}

/// A grant the vault holds for one request: the scope the network process
/// asks under, and the vault to revoke it with once the reply is in (see
/// [`dispatch_to_net_process`]).
#[cfg(all(feature = "process-isolation", target_os = "linux"))]
struct Grant {
    vault: Arc<crate::cookie_vault::client::CookieVault>,
    scope: CookieScope,
}
#[cfg(not(all(feature = "process-isolation", target_os = "linux")))]
type Grant = std::convert::Infallible;

#[cfg(all(feature = "process-isolation", target_os = "linux"))]
async fn grant_scope(identity: Option<&TabIdentity>, scope: CookieScope) -> Option<Grant> {
    let vault = {
        let jar = identity?.cookie_jar.read();
        Arc::clone(
            jar.as_any()
                .downcast_ref::<crate::cookie_vault::client::VaultCookieJar>()?
                .vault(),
        )
    };
    let granting = Arc::clone(&vault);
    let granted_scope = scope.clone();
    let granted = tokio::task::spawn_blocking(move || granting.grant(&granted_scope)).await;
    if !matches!(granted, Ok(true)) {
        return None;
    }
    Some(Grant { vault, scope })
}

#[cfg(not(all(feature = "process-isolation", target_os = "linux")))]
async fn grant_scope(_identity: Option<&TabIdentity>, _scope: CookieScope) -> Option<Grant> {
    None
}

#[cfg(not(all(feature = "process-isolation", target_os = "linux")))]
fn cookie_scope_for(_router: &IoRouter, _identity: Option<&TabIdentity>, _req: &FetchRequest) -> Option<CookieScope> {
    None
}

/// Put the requesting tab's cookies on an outbound request.
/// A vault jar answers over IPC, so the lookup runs on a blocking thread.
/// The document a request's policies (private-network protection, opaque
/// response blocking) are decided from: the one the request was made for,
/// which engine code stamps as the referrer of every subresource fetch - the
/// in-process pipeline's for the page it is loading, the brokered loader's
/// for the page a renderer is showing. The tab's top-level URL is only the
/// fallback. It moves to the navigation target as soon as a navigation
/// starts, while the page still shown keeps asking through scroll, hover and
/// media passes until the new one commits; judged by the top-level URL, a
/// public page's late requests were classified as the private page's own.
fn policy_document(req: &FetchRequest, identity: Option<&TabIdentity>) -> Option<url::Url> {
    req.referrer
        .clone()
        .or_else(|| identity.and_then(|id| id.top_level.clone()))
}

async fn attach_request_cookies(req: &mut FetchRequest, identity: Option<&TabIdentity>) {
    req.headers.remove(http::header::COOKIE);

    let Some(identity) = identity else {
        return;
    };
    let context = first_hop_context(identity, req);
    let jar = identity.cookie_jar.clone();
    let url = req.url.clone();
    let top_level = identity.top_level.clone();
    let cookies =
        tokio::task::spawn_blocking(move || jar.read().get_request_cookies(&url, top_level.as_ref(), context)).await;
    let Ok(Some(cookies)) = cookies else {
        return;
    };
    if let Ok(value) = cookies.parse() {
        req.headers.insert(http::header::COOKIE, value);
    }
}

/// The `SameSite` context of a request's first hop, judged from the document
/// that caused it (see [`TabIdentity::same_site_document`]): a navigation a
/// cross-site page started is a cross-site navigation, `Lax` only when its
/// method is safe.
fn first_hop_context(identity: &TabIdentity, req: &FetchRequest) -> SameSiteContext {
    crate::engine::cookies::hop_context(
        identity.same_site_document(REF_REGISTRY.from_net(req.reference)),
        &req.url,
        &[],
        &req.method,
        req.kind == gosub_sonar::net::types::ResourceKind::Primary,
    )
}

/// Classify a request against the document that caused it; see
/// [`request_context`](crate::engine::cookies::request_context).
#[cfg(test)]
fn same_site_context(top_level: Option<&url::Url>, url: &url::Url) -> SameSiteContext {
    crate::engine::cookies::request_context(top_level, url, false)
}

/// Wrap a reply channel so `Set-Cookie` is recorded on this side before the
/// result reaches the requester, which receives it unchanged.
fn store_response_cookies_then_forward(
    identity: TabIdentity,
    reply_tx: oneshot::Sender<FetchResult>,
) -> oneshot::Sender<FetchResult> {
    let (inner_tx, inner_rx) = oneshot::channel::<FetchResult>();

    spawn_named("io-cookie-store", async move {
        // An error means the fetcher dropped the channel (cancelled or failed);
        // there is then no response whose cookies could be stored.
        let Ok(result) = inner_rx.await else {
            return;
        };
        // A vault jar stores over IPC (and may respawn the vault first), so
        // like the lookup in `attach_request_cookies` it runs off the runtime.
        if let Some((url, headers)) = result.meta().map(|m| (m.final_url.clone(), m.headers.clone())) {
            let stored = tokio::task::spawn_blocking(move || {
                identity
                    .cookie_jar
                    .write()
                    .store_response_cookies(&url, &headers, identity.top_level.as_ref());
            })
            .await;
            if let Err(e) = stored {
                log::warn!("storing a response's cookies failed: {e}");
            }
        }
        let _ = reply_tx.send(result);
    });

    inner_tx
}

/// Wrap a navigation's reply channel so the tab's identity records where the
/// response came from - the connection's peer, not a fresh lookup of the host,
/// which a rebinding DNS server could answer differently - before the requester
/// sees it. A failed navigation records nothing.
fn record_document_space_then_forward(
    tab_identities: Arc<crate::net::tab_identity::TabIdentityRegistry>,
    tab_id: crate::tab::TabId,
    navigation: crate::engine::types::NavigationId,
    reply_tx: oneshot::Sender<FetchResult>,
) -> oneshot::Sender<FetchResult> {
    let (inner_tx, inner_rx) = oneshot::channel::<FetchResult>();

    spawn_named("io-doc-space", async move {
        let Ok(result) = inner_rx.await else {
            return;
        };
        if let Some(meta) = result.meta() {
            let space = crate::net::ssrf::space_of_response(&meta.final_url, meta.peer_addr);
            tab_identities.record_navigation(tab_id, navigation, space);
        }
        let _ = reply_tx.send(result);
    });

    inner_tx
}

/// Wrap a reply channel so a cross-origin body that must not reach a page is
/// withheld here (see [`crate::net::orb`]); the requester sees an error instead.
fn block_opaque_responses_then_forward(
    document: url::Url,
    reply_tx: oneshot::Sender<FetchResult>,
) -> oneshot::Sender<FetchResult> {
    let (inner_tx, inner_rx) = oneshot::channel::<FetchResult>();

    spawn_named("io-orb", async move {
        let Ok(result) = inner_rx.await else {
            return;
        };
        let _ = reply_tx.send(apply_orb(&document, result));
    });

    inner_tx
}

/// The ORB verdict applied to one result: unchanged when allowed, an error
/// carrying the reason when not.
fn apply_orb(document: &url::Url, result: FetchResult) -> FetchResult {
    use crate::net::orb::{verdict, OrbVerdict};

    let (meta, peek): (&gosub_sonar::net::types::FetchResultMeta, &[u8]) = match &result {
        FetchResult::Buffered { meta, body } => (meta, body.as_ref()),
        FetchResult::Stream { meta, peek_buf, .. } => (meta, peek_buf.as_ref()),
        FetchResult::Error(_) => return result,
    };
    let same_origin = document.origin() == meta.final_url.origin();
    let content_type = meta
        .headers
        .get(http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok());
    let nosniff = meta
        .headers
        .get("x-content-type-options")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.trim().eq_ignore_ascii_case("nosniff"));
    match verdict(same_origin, content_type, nosniff, meta.status, peek) {
        OrbVerdict::Allow => result,
        OrbVerdict::Block(reason) => {
            log::info!("opaque response blocked for {document}: {} ({reason})", meta.final_url);
            let url = meta.final_url.clone();
            FetchResult::Error(gosub_sonar::net::types::NetError::Other(Arc::new(anyhow::anyhow!(
                "opaque response blocked ({reason}): {url}"
            ))))
        }
    }
}

/// Submit a fetch on behalf of `tab_id`.
pub(crate) async fn submit_to_io(
    zone_id: ZoneId,
    tab_id: Option<TabId>,
    req: FetchRequest,
    io_tx: IoChannel,
    parent_cancel: Option<CancellationToken>,
) -> anyhow::Result<(FetchHandle, oneshot::Receiver<FetchResult>)> {
    let (reply_tx, reply_rx) = oneshot::channel::<FetchResult>();

    let cancel = match parent_cancel {
        Some(parent) => parent.child_token(),
        None => CancellationToken::new(),
    };

    let handle = FetchHandle {
        req_id: req.req_id,
        cancel: cancel.clone(),
    };

    io_tx
        .send(IoCommand::Fetch {
            zone_id,
            tab_id,
            req,
            handle: handle.clone(),
            reply_tx,
        })
        .map_err(|_| anyhow::anyhow!("I/O thread has shut down"))?;

    Ok((handle, reply_rx))
}

/// Spawns the IO thread and runs a single fetcher on top. The fetcher config is read from
/// the settings store per zone, when it first fetches.
pub(crate) fn spawn_io_thread(engine_ctx: Arc<EngineContext>) -> IoHandle {
    let (tx_submit, mut rx_submit) = mpsc::unbounded_channel::<IoCommand>();
    let shutdown_token = CancellationToken::new();
    let cancel = shutdown_token.clone();

    let join_handle = spawn_named("I/O Thread", async move {
        let router = IoRouter::new(engine_ctx);

        loop {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => {
                    log::trace!("I/O thread received global shutdown signal");
                    break;
                }
                maybe_req = rx_submit.recv() => {
                    match maybe_req {
                        Some(IoCommand::Fetch { zone_id, tab_id, mut req, handle, reply_tx }) => {
                            // A request made on a tab's behalf through the broker (a
                            // renderer's subresource) names a document of the loader's
                            // own; which tab that is, is known here and nowhere below.
                            // Recorded now, so the request's observer finds the tab.
                            if let Some(tab) = tab_id {
                                if let Some(reference) = REF_REGISTRY.from_net(req.reference) {
                                    if matches!(reference, RequestReference::Document(_)) {
                                        router
                                            .engine_ctx
                                            .request_reference_map
                                            .write()
                                            .entry(reference)
                                            .or_insert(tab);
                                    }
                                }
                            }
                            // The engine serves file:// and data: itself; everything else goes
                            // to the zone's gosub-sonar fetcher, which only speaks http(s).
                            if crate::net::file_loader::handles(&req) {
                                router.serve_file_request(req, reply_tx);
                                continue;
                            }
                            if crate::net::data_url::handles(&req) {
                                router.serve_data_url_request(req, handle.cancel.clone(), reply_tx);
                                continue;
                            }

                            // A navigation a document started has its cookies judged from
                            // that document, which `navigation()` stamps as the request's
                            // referrer; the user's own carries none. Recorded before
                            // anything asks for the navigation's cookies.
                            if let (Some(tab), gosub_sonar::net::types::ResourceKind::Primary) = (tab_id, req.kind) {
                                if let Some(RequestReference::Navigation(nav)) = REF_REGISTRY.from_net(req.reference) {
                                    router
                                        .tab_identities()
                                        .set_navigation_initiator(tab, nav, req.referrer.clone());
                                }
                            }
                            // Cookies are attached here, never by the requester: see
                            // `net::tab_identity`. `identity` is None for a tab that has
                            // closed or never registered, which sends no cookies.
                            let identity = tab_id.and_then(|id| router.tab_identities().get(id));
                            // With a vault the network process talks to directly, the
                            // request carries whose cookies it wants and the network
                            // process asks for them per hop. In process, the fetcher asks
                            // the tab's jar per hop (`EngineNetContext::cookies_for`).
                            // Either way this side neither attaches nor stores any.
                            let cookie_scope = cookie_scope_for(&router, identity.as_ref(), &req);
                            let net = router.net_process();
                            // All of the zone's fetchers: which one serves the request
                            // is decided in the task, after the address-space lookup.
                            let fetchers = match &net {
                                Some(_) => None,
                                None => match router.get_or_spawn_zone_fetchers(zone_id) {
                                    Ok(fetchers) => Some(fetchers),
                                    Err(e) => {
                                        log::error!("Failed to create fetcher for zone {zone_id}: {e}");
                                        continue;
                                    }
                                },
                            };
                            let tab_identities = Arc::clone(&router.engine_ctx.tab_identities);
                            let reference = REF_REGISTRY.from_net(req.reference);
                            // The network process has no fetcher of ours to ask for an
                            // observer: built here, as the in-process fetcher would.
                            #[cfg(feature = "process-isolation")]
                            let observer = net.as_ref().map(|_| {
                                crate::net::fetcher::observer_for_request(
                                    &router.engine_ctx.resource_tx,
                                    &router.engine_ctx.event_tx,
                                    &router.engine_ctx.request_reference_map,
                                    req.reference,
                                    req.req_id,
                                    req.kind,
                                    req.initiator,
                                )
                            });

                            // The rest may block - a vault round trip for the cookies, a
                            // DNS lookup for the policy - so it runs off this loop, which
                            // must stay free for every other tab's requests.
                            spawn_named("io-fetch", async move {
                                let subresource = req.kind != gosub_sonar::net::types::ResourceKind::Primary;
                                let document = policy_document(&req, identity.as_ref());
                                // The vault must hold the grant before the network process
                                // can ask under it; a refused grant means no cookies at all.
                                let grant = match cookie_scope {
                                    Some(scope) => grant_scope(identity.as_ref(), scope).await,
                                    None => None,
                                };
                                // Only a network process without a vault grant cannot ask
                                // for cookies per hop: it gets them here, once, for the
                                // first URL, and a cookie a redirect hop sets reaches no jar
                                // (its final response's does, below).
                                let cookies_per_hop = grant.is_some() || net.is_none();
                                if cookies_per_hop {
                                    req.headers.remove(http::header::COOKIE);
                                } else {
                                    attach_request_cookies(&mut req, identity.as_ref()).await;
                                }

                                // Policy for what a page loads, decided from the document the
                                // request is for - never from anything a renderer sent. A
                                // subresource may not reach an address space more private than
                                // its document's, and its cross-origin bytes pass through ORB.
                                // The document is placed by where its response came from, which
                                // the tab's identity recorded when it arrived; see
                                // `TabIdentity::document_space`. A navigation reaches anything.
                                let reach = if subresource {
                                    identity
                                        .as_ref()
                                        .map_or(AddressSpace::Public, |id| id.document_space(reference))
                                } else {
                                    AddressSpace::Loopback
                                };

                                // A network process's reply is intercepted so `Set-Cookie` is
                                // stored on this side too; the requester still receives the
                                // untouched result. Under a vault grant the network process
                                // stored the cookies and stripped them, so this finds none -
                                // unless the vault did not take them, and then this is the
                                // only store they get. In process, the fetcher already handed
                                // every hop's to the jar.
                                let reply_tx = match identity {
                                    Some(id) if net.is_some() => store_response_cookies_then_forward(id, reply_tx),
                                    _ => reply_tx,
                                };
                                let reply_tx = match (subresource, document) {
                                    (true, Some(top)) => block_opaque_responses_then_forward(top, reply_tx),
                                    _ => reply_tx,
                                };
                                // A navigation's response says where its document lives,
                                // recorded before the document is parsed and can ask for more.
                                let reply_tx = match (subresource, tab_id, reference) {
                                    (false, Some(tab), Some(RequestReference::Navigation(nav))) => {
                                        record_document_space_then_forward(tab_identities, tab, nav, reply_tx)
                                    }
                                    _ => reply_tx,
                                };

                                match (net, fetchers) {
                                    (Some(net), _) => dispatch_to_net_process(
                                        net,
                                        req,
                                        reach,
                                        grant,
                                        handle.cancel.clone(),
                                        reply_tx,
                                        #[cfg(feature = "process-isolation")]
                                        observer,
                                        #[cfg(not(feature = "process-isolation"))]
                                        None,
                                    ),
                                    (None, Some(fetchers)) => {
                                        let fetcher = fetchers.for_reach(reach);
                                        fetcher.submit(req, handle.cancel.clone(), reply_tx).await;
                                    }
                                    (None, None) => {}
                                }
                            });
                        }
                        Some(IoCommand::SetTopLevel { tab_id, url }) => {
                            router.tab_identities().set_top_level(tab_id, url);
                        }
                        Some(IoCommand::CommitDocument { tab_id, nav_id }) => {
                            router.tab_identities().commit_navigation(tab_id, nav_id);
                        }
                        #[cfg(feature = "process-isolation")]
                        Some(IoCommand::NetPid { reply_tx }) => {
                            let _ = reply_tx.send(router.net_process().and_then(|net| net.pid()));
                        }
                        #[cfg(feature = "process-isolation")]
                        Some(IoCommand::AuditNet { reply_tx }) => {
                            let net = router.net_process();
                            spawn_named("io-audit-net", async move {
                                let report = match net {
                                    Some(net) => tokio::task::spawn_blocking(move || {
                                        net.ensure_alive();
                                        net.audit().ok().flatten()
                                    })
                                        .await
                                        .ok()
                                        .flatten(),
                                    None => None,
                                };
                                let _ = reply_tx.send(report);
                            });
                        }
                        Some(IoCommand::ShutdownZone { zone_id, reply_tx }) => {
                            let _ = router.shutdown_zone(zone_id).await;
                            let _ = reply_tx.send(());
                        }
                        None => break,
                    }
                }
            }
        }

        log::trace!("I/O thread shutting down all zone fetchers");
        router.shutdown_all().await;
    });

    IoHandle {
        tx_submit,
        shutdown_token,
        join_handle,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::time::{sleep, timeout};

    /// A navigation a cross-site page started is a cross-site navigation from its first hop:
    /// `Lax` only for a safe method, never `Strict`. The user's own navigation is same-site.
    #[test]
    fn a_navigations_first_hop_is_judged_from_the_page_that_started_it() {
        use crate::engine::types::NavigationId;
        use crate::net::tab_identity::TabIdentity;
        use url::Url;
        let destination = Url::parse("https://b.test/account").unwrap();
        let page = Url::parse("https://a.test/links").unwrap();
        let nav = NavigationId::new();
        let mut identity = TabIdentity::new(crate::cookies::DefaultCookieJar::new().into());
        identity.top_level = Some(destination.clone());
        let request = |method: http::Method| {
            FetchRequest::builder(method, destination.clone())
                .with_reference(REF_REGISTRY.to_net(RequestReference::Navigation(nav)))
                .with_kind(gosub_sonar::net::types::ResourceKind::Primary)
                .build()
        };

        assert_eq!(
            first_hop_context(&identity, &request(http::Method::GET)),
            SameSiteContext::SameSite
        );
        identity.initiator = Some((nav, page));
        assert_eq!(
            first_hop_context(&identity, &request(http::Method::GET)),
            SameSiteContext::CrossSiteNavigation
        );
        assert_eq!(
            first_hop_context(&identity, &request(http::Method::POST)),
            SameSiteContext::CrossSite
        );
    }

    /// A tab navigating from a public page to a private one: the public
    /// page's own late subresource requests (made for it, so stamped with it)
    /// stay judged as the public page's, not as the private target's.
    #[test]
    fn a_request_is_judged_by_the_document_it_was_made_for() {
        use crate::net::tab_identity::TabIdentity;
        use url::Url;
        let public = Url::parse("https://evil.example/").unwrap();
        let private = Url::parse("http://192.168.1.1/").unwrap();
        let identity = TabIdentity {
            top_level: Some(private.clone()),
            ..TabIdentity::new(crate::cookies::DefaultCookieJar::new().into())
        };
        let for_public = FetchRequest::builder(http::Method::GET, Url::parse("http://192.168.1.1/img.png").unwrap())
            .with_referrer(public.clone())
            .build();
        assert_eq!(policy_document(&for_public, Some(&identity)), Some(public));
        // Without a document of its own a request takes the tab's.
        let bare = FetchRequest::builder(http::Method::GET, Url::parse("http://192.168.1.1/img.png").unwrap()).build();
        assert_eq!(policy_document(&bare, Some(&identity)), Some(private));
        assert_eq!(policy_document(&bare, None), None);
    }

    /// Cookie attachment: what the I/O side puts on a request, given who is asking.
    mod cookies {
        use super::*;
        use crate::cookies::{CookieJarHandle, DefaultCookieJar};
        use http::Method;
        use url::Url;

        fn jar_with(url: &str, set_cookie: &str) -> CookieJarHandle {
            let jar: CookieJarHandle = DefaultCookieJar::new().into();
            let mut headers = http::HeaderMap::new();
            headers.append(http::header::SET_COOKIE, set_cookie.parse().unwrap());
            jar.write()
                .store_response_cookies(&Url::parse(url).unwrap(), &headers, None);
            jar
        }

        fn request_to(url: &str) -> FetchRequest {
            FetchRequest::builder(Method::GET, Url::parse(url).unwrap()).build()
        }

        fn cookie_header(req: &FetchRequest) -> Option<&str> {
            req.headers.get(http::header::COOKIE).map(|v| {
                #[allow(clippy::unwrap_used)] // test-only: values are ASCII literals
                v.to_str().unwrap()
            })
        }

        #[tokio::test]
        async fn a_tab_gets_its_own_cookies() {
            let identity = TabIdentity {
                top_level: Some(Url::parse("https://example.com/page").unwrap()),
                ..TabIdentity::new(jar_with("https://example.com/", "sid=abc; Path=/"))
            };
            let mut req = request_to("https://example.com/api");
            attach_request_cookies(&mut req, Some(&identity)).await;

            assert_eq!(cookie_header(&req), Some("sid=abc"));
        }

        #[tokio::test]
        async fn no_identity_means_no_cookies() {
            // A closed or unregistered tab must not borrow anyone else's jar.
            let mut req = request_to("https://example.com/api");
            attach_request_cookies(&mut req, None).await;

            assert_eq!(cookie_header(&req), None);
        }

        #[tokio::test]
        async fn a_cookie_header_from_the_requester_is_discarded() {
            // The property the whole inversion rests on: a compromised tab cannot
            // send cookies of its own choosing, not even for its own origin.
            let identity = TabIdentity {
                top_level: Some(Url::parse("https://example.com/page").unwrap()),
                ..TabIdentity::new(jar_with("https://example.com/", "sid=real; Path=/"))
            };
            let mut req = request_to("https://example.com/api");
            req.headers
                .insert(http::header::COOKIE, "sid=forged; admin=1".parse().unwrap());

            attach_request_cookies(&mut req, Some(&identity)).await;

            assert_eq!(cookie_header(&req), Some("sid=real"));
        }

        #[tokio::test]
        async fn a_forged_header_is_dropped_even_with_no_identity() {
            let mut req = request_to("https://example.com/api");
            req.headers.insert(http::header::COOKIE, "sid=forged".parse().unwrap());

            attach_request_cookies(&mut req, None).await;

            assert_eq!(cookie_header(&req), None, "an unidentified tab must send nothing");
        }

        #[test]
        fn cross_site_requests_are_classified_as_such() {
            let page = Url::parse("https://example.com/page").unwrap();

            assert_eq!(
                same_site_context(Some(&page), &Url::parse("https://example.com/api").unwrap()),
                SameSiteContext::SameSite
            );
            assert_eq!(
                same_site_context(Some(&page), &Url::parse("https://other.test/api").unwrap()),
                SameSiteContext::CrossSite
            );
            // Subdomains of one registrable domain are one site; a public suffix
            // is not a site.
            assert_eq!(
                same_site_context(
                    Some(&Url::parse("https://www.example.com/").unwrap()),
                    &Url::parse("https://api.example.com/login").unwrap()
                ),
                SameSiteContext::SameSite
            );
            assert_eq!(
                same_site_context(
                    Some(&Url::parse("https://alice.github.io/").unwrap()),
                    &Url::parse("https://bob.github.io/").unwrap()
                ),
                SameSiteContext::CrossSite
            );
            // A scheme change is a site change: an http:// load must not receive
            // cookies set for the https:// page.
            assert_eq!(
                same_site_context(Some(&page), &Url::parse("http://example.com/api").unwrap()),
                SameSiteContext::CrossSite
            );
            // The document load itself has no document behind it.
            assert_eq!(same_site_context(None, &page), SameSiteContext::SameSite);
            // A subdomain shares the page's registrable domain: still same-site.
            assert_eq!(
                same_site_context(Some(&page), &Url::parse("https://api.example.com/x").unwrap()),
                SameSiteContext::SameSite
            );
            // A shared eTLD is not a shared site.
            assert_eq!(
                same_site_context(
                    Some(&Url::parse("https://a.github.io/").unwrap()),
                    &Url::parse("https://b.github.io/x").unwrap()
                ),
                SameSiteContext::CrossSite
            );
        }
    }

    /// Helper to make a minimal EngineContext for tests.
    fn test_engine_ctx() -> Arc<EngineContext> {
        let (tx, _rx) = tokio::sync::broadcast::channel(16);
        let ctx = EngineContext {
            event_tx: tx,
            ..Default::default()
        };
        // In-process networking: the network process would be this test binary re-executed,
        // which libtest refuses as an unknown option before the I/O thread falls back anyway.
        let _ = ctx
            .config_store
            .set_transient("security.network_process", gosub_config::settings::Setting::Bool(false));
        Arc::new(ctx)
    }

    // IoHandle-level tests

    /// IO thread boots and can be globally shut down cleanly.
    #[tokio::test(flavor = "current_thread")]
    async fn io_driver_starts_and_global_shutdown_is_clean() {
        let ctx = test_engine_ctx();
        let handle = spawn_io_thread(ctx);

        // Let the driver spin up
        sleep(Duration::from_millis(10)).await;

        timeout(Duration::from_secs(2), handle.shutdown())
            .await
            .expect("global shutdown timed out");
    }

    /// Shutting down a zone that hasn't been spawned should still ACK promptly.
    #[tokio::test(flavor = "current_thread")]
    async fn io_shutdown_zone_ack_without_prior_fetcher() {
        let ctx = test_engine_ctx();
        let handle = spawn_io_thread(ctx);

        let z = ZoneId::new();
        timeout(Duration::from_secs(2), handle.shutdown_zone(z))
            .await
            .expect("zone shutdown ack timed out")
            .expect("zone shutdown returned error");

        timeout(Duration::from_secs(2), handle.shutdown())
            .await
            .expect("global shutdown timed out");
    }

    // Router-level tests (spawn/shutdown per-zone without network)

    /// Spawns a per-zone fetcher on first use and shuts it down cleanly.
    #[tokio::test(flavor = "current_thread")]
    async fn router_spawns_and_shuts_down_zone() {
        let ctx = test_engine_ctx();

        let router = IoRouter::new(ctx);
        let z = ZoneId::new();

        let f = router
            .get_or_spawn_zone_fetchers(z)
            .unwrap()
            .for_reach(AddressSpace::Loopback);
        assert!(Arc::strong_count(&f) >= 1, "fetcher Arc should be alive");

        let stopped = router.shutdown_zone(z).await;
        assert!(stopped, "zone should have existed and been stopped");
    }

    /// Shutting down one zone must not affect others; the other zone's fetcher should keep running.
    #[tokio::test(flavor = "current_thread")]
    async fn router_isolates_zones() {
        let ctx = test_engine_ctx();

        let router = IoRouter::new(ctx);
        let z1 = ZoneId::new();
        let z2 = ZoneId::new();

        let _f1 = router.get_or_spawn_zone_fetchers(z1).unwrap();
        let f2 = router
            .get_or_spawn_zone_fetchers(z2)
            .unwrap()
            .for_reach(AddressSpace::Public);

        let stopped = router.shutdown_zone(z1).await;
        assert!(stopped, "z1 should have been stopped");

        let f2_again = router
            .get_or_spawn_zone_fetchers(z2)
            .unwrap()
            .for_reach(AddressSpace::Public);
        assert!(Arc::ptr_eq(&f2, &f2_again), "z2 fetcher must remain the same instance");

        // Clean up remaining zones to avoid leaking tasks in test
        router.shutdown_all().await;
    }

    /// Shutting down an unknown zone is a no-op (returns false).
    #[tokio::test(flavor = "current_thread")]
    async fn router_shutdown_unknown_zone_is_noop() {
        let ctx = test_engine_ctx();

        let router = IoRouter::new(ctx);

        let z_never_spawned = ZoneId::new();
        let stopped = router.shutdown_zone(z_never_spawned).await;
        assert!(!stopped, "unknown zone should return false on shutdown");

        router.shutdown_all().await;
    }
}
