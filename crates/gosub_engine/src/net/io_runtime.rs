use crate::cookies::SameSiteContext;
use crate::engine::types::IoChannel;
use crate::engine::EngineContext;
use crate::events::IoCommand;
use crate::net::decision_hub::DecisionHub;
use crate::net::fetcher::{EngineNetContext, Fetcher, FetcherConfig};
use crate::net::req_ref_tracker::RequestRefTracker;
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

        // Note: subscribers hold clones of this sender, so the channel only fully
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
    pub fn subscribe(&self) -> IoChannel {
        self.tx_submit.clone()
    }
}

pub struct ZoneEntry {
    fetcher: Arc<Fetcher>,
    shutdown: CancellationToken,
    join: JoinHandle<()>,
}

/// Routes I/O requests to per-zone fetchers, spawning them on first use.
pub struct IoRouter {
    /// Map of zone ID to zone entries
    zones: DashMap<ZoneId, ZoneEntry>,
    /// Default fetcher config to use when spawning new fetchers
    cfg: FetcherConfig,
    /// Shared engine context for event broadcasting and request tracking
    engine_ctx: Arc<EngineContext>,
    /// Pending UA decisions (render/download/...) keyed by decision token.
    /// Tokens are process-wide unique, so one hub serves all zones.
    decision_hub: Arc<DecisionHub>,
    /// Observer factory for requests the engine serves itself (the `file://` scheme),
    /// so they emit the same resource events a gosub-sonar fetch would.
    local_ctx: EngineNetContext,
}

impl IoRouter {
    pub fn new(cfg: FetcherConfig, engine_ctx: Arc<EngineContext>) -> Self {
        let local_ctx = EngineNetContext {
            event_tx: engine_ctx.event_tx.clone(),
            request_reference_map: engine_ctx.request_reference_map.clone(),
            request_ref_tracker: Arc::new(RequestRefTracker::new()),
        };
        Self {
            zones: DashMap::new(),
            cfg,
            engine_ctx,
            decision_hub: Arc::new(DecisionHub::new()),
            local_ctx,
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

    /// Who each tab is, for resolving cookies without trusting the request.
    pub fn tab_identities(&self) -> &TabIdentityRegistry {
        &self.engine_ctx.tab_identities
    }

    pub fn get_or_spawn_zone_fetcher(&self, zone_id: ZoneId) -> Result<Arc<Fetcher>, EngineError> {
        if let Some(f) = self.zones.get(&zone_id) {
            return Ok(f.fetcher.clone());
        }

        let zone_shutdown = CancellationToken::new();

        let engine_ctx = Arc::new(EngineNetContext {
            event_tx: self.engine_ctx.event_tx.clone(),
            request_reference_map: self.engine_ctx.request_reference_map.clone(),
            request_ref_tracker: Arc::new(RequestRefTracker::new()),
        });
        let f =
            Arc::new(Fetcher::new(self.cfg.clone(), engine_ctx).map_err(|e| EngineError::NetworkError(e.to_string()))?);

        let f_run = f.clone();
        let cancel = zone_shutdown.clone();
        let title = format!("I/O Fetcher Zone {}", zone_id);
        let join_handle = spawn_named(&title, async move {
            f_run.run(cancel).await;
        });

        self.zones.insert(
            zone_id,
            ZoneEntry {
                fetcher: f.clone(),
                shutdown: zone_shutdown,
                join: join_handle,
            },
        );

        Ok(f)
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
    }
}

/// Put the requesting tab's cookies on an outbound request.
fn attach_request_cookies(req: &mut FetchRequest, identity: Option<&TabIdentity>) {
    req.headers.remove(http::header::COOKIE);

    let Some(identity) = identity else {
        return;
    };
    let context = same_site_context(identity.top_level.as_ref(), &req.url);
    let Some(cookies) = identity
        .cookie_jar
        .read()
        .get_request_cookies(&req.url, identity.top_level.as_ref(), context)
    else {
        return;
    };
    if let Ok(value) = cookies.parse() {
        req.headers.insert(http::header::COOKIE, value);
    }
}

/// Classify a request against the document that caused it, so `SameSite`
/// cookies are withheld from genuinely cross-site loads.
fn same_site_context(top_level: Option<&url::Url>, url: &url::Url) -> SameSiteContext {
    match top_level {
        // A request with no document behind it is the document load itself.
        None => SameSiteContext::SameSite,
        Some(top) if top.host_str() == url.host_str() && top.scheme() == url.scheme() => SameSiteContext::SameSite,
        Some(_) => SameSiteContext::CrossSite,
    }
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
        if let Some(meta) = result.meta() {
            identity.cookie_jar.write().store_response_cookies(
                &meta.final_url,
                &meta.headers,
                identity.top_level.as_ref(),
            );
        }
        let _ = reply_tx.send(result);
    });

    inner_tx
}

/// Submit a fetch on behalf of `tab_id`.
pub async fn submit_to_io(
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

/// Spawns the IO thread and runs a single fetcher on top. If needed, we can expand this system to
/// run multiple fetchers on different OS threads for instance, but most likely the fetching itself
/// isn't the biggest bottleneck.
pub fn spawn_io_thread(cfg: FetcherConfig, engine_ctx: Arc<EngineContext>) -> IoHandle {
    let (tx_submit, mut rx_submit) = mpsc::unbounded_channel::<IoCommand>();
    let shutdown_token = CancellationToken::new();
    let cancel = shutdown_token.clone();

    let join_handle = spawn_named("I/O Thread", async move {
        let router = IoRouter::new(cfg, engine_ctx);

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
                            // The engine serves file:// itself; everything else goes to the
                            // zone's gosub-sonar fetcher.
                            if crate::net::file_loader::handles(&req) {
                                router.serve_file_request(req, reply_tx);
                                continue;
                            }
                            // Cookies are attached here, never by the requester: see
                            // `net::tab_identity`. `identity` is None for a tab that has
                            // closed or never registered, which sends no cookies.
                            let identity = tab_id.and_then(|id| router.tab_identities().get(id));
                            attach_request_cookies(&mut req, identity.as_ref());

                            // The reply is intercepted so `Set-Cookie` is stored on this
                            // side too; the requester still receives the untouched result.
                            let reply_tx = match identity {
                                Some(id) => store_response_cookies_then_forward(id, reply_tx),
                                None => reply_tx,
                            };

                            // The I/O thread must keep running; drop the request on fetcher failure.
                            match router.get_or_spawn_zone_fetcher(zone_id) {
                                Ok(fetcher) => fetcher.submit(req, handle.cancel.clone(), reply_tx).await,
                                Err(e) => log::error!("Failed to create fetcher for zone {zone_id}: {e}"),
                            }
                        }
                        Some(IoCommand::Decision { token, action }) => {
                            // Decisions are engine-owned (gosub-sonar has no decision hub);
                            // tokens are unique so a single hub covers every zone.
                            router.decision_hub.fulfill(token, action);
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

        #[test]
        fn a_tab_gets_its_own_cookies() {
            let identity = TabIdentity {
                cookie_jar: jar_with("https://example.com/", "sid=abc; Path=/"),
                top_level: Some(Url::parse("https://example.com/page").unwrap()),
            };
            let mut req = request_to("https://example.com/api");
            attach_request_cookies(&mut req, Some(&identity));

            assert_eq!(cookie_header(&req), Some("sid=abc"));
        }

        #[test]
        fn no_identity_means_no_cookies() {
            // A closed or unregistered tab must not borrow anyone else's jar.
            let mut req = request_to("https://example.com/api");
            attach_request_cookies(&mut req, None);

            assert_eq!(cookie_header(&req), None);
        }

        #[test]
        fn a_cookie_header_from_the_requester_is_discarded() {
            // The property the whole inversion rests on: a compromised tab cannot
            // send cookies of its own choosing, not even for its own origin.
            let identity = TabIdentity {
                cookie_jar: jar_with("https://example.com/", "sid=real; Path=/"),
                top_level: Some(Url::parse("https://example.com/page").unwrap()),
            };
            let mut req = request_to("https://example.com/api");
            req.headers
                .insert(http::header::COOKIE, "sid=forged; admin=1".parse().unwrap());

            attach_request_cookies(&mut req, Some(&identity));

            assert_eq!(cookie_header(&req), Some("sid=real"));
        }

        #[test]
        fn a_forged_header_is_dropped_even_with_no_identity() {
            let mut req = request_to("https://example.com/api");
            req.headers.insert(http::header::COOKIE, "sid=forged".parse().unwrap());

            attach_request_cookies(&mut req, None);

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
            // A scheme change is a site change: an http:// load must not receive
            // cookies set for the https:// page.
            assert_eq!(
                same_site_context(Some(&page), &Url::parse("http://example.com/api").unwrap()),
                SameSiteContext::CrossSite
            );
            // The document load itself has no document behind it.
            assert_eq!(same_site_context(None, &page), SameSiteContext::SameSite);
        }
    }

    fn test_cfg() -> FetcherConfig {
        FetcherConfig {
            global_slots: 2,
            h1_per_origin: 2,
            h2_per_origin: 2,
            connect_timeout: Duration::from_millis(50),
            req_timeout: Duration::from_millis(100),
            read_idle_timeout: Duration::from_millis(100),
            total_body_timeout: Some(Duration::from_millis(150)),
            ..FetcherConfig::default()
        }
    }

    /// Helper to make a minimal EngineContext for tests.
    fn test_engine_ctx() -> Arc<EngineContext> {
        let (tx, _rx) = tokio::sync::broadcast::channel(16);
        Arc::new(EngineContext {
            event_tx: tx,
            ..Default::default()
        })
    }

    // IoHandle-level tests

    /// IO thread boots and can be globally shut down cleanly.
    #[tokio::test(flavor = "current_thread")]
    async fn io_driver_starts_and_global_shutdown_is_clean() {
        let ctx = test_engine_ctx();
        let handle = spawn_io_thread(test_cfg(), ctx);

        // Let the driver spin up
        sleep(Duration::from_millis(10)).await;

        // Global shutdown should complete promptly
        timeout(Duration::from_secs(2), handle.shutdown())
            .await
            .expect("global shutdown timed out");
    }

    /// Shutting down a zone that hasn't been spawned should still ACK promptly.
    #[tokio::test(flavor = "current_thread")]
    async fn io_shutdown_zone_ack_without_prior_fetcher() {
        let ctx = test_engine_ctx();
        let handle = spawn_io_thread(test_cfg(), ctx);

        let z = ZoneId::new();
        // Should ACK even if the zone was never created
        timeout(Duration::from_secs(2), handle.shutdown_zone(z))
            .await
            .expect("zone shutdown ack timed out")
            .expect("zone shutdown returned error");

        // Cleanly stop IO
        timeout(Duration::from_secs(2), handle.shutdown())
            .await
            .expect("global shutdown timed out");
    }

    // Router-level tests (spawn/shutdown per-zone without network)

    /// Spawns a per-zone fetcher on first use and shuts it down cleanly.
    #[tokio::test(flavor = "current_thread")]
    async fn router_spawns_and_shuts_down_zone() {
        let cfg = test_cfg();
        let ctx = test_engine_ctx();

        let router = IoRouter::new(cfg, ctx);
        let z = ZoneId::new();

        // Lazily create fetcher for zone z
        let f = router.get_or_spawn_zone_fetcher(z).unwrap();
        assert!(Arc::strong_count(&f) >= 1, "fetcher Arc should be alive");

        // Shut down zone z; should return true (existed)
        let stopped = router.shutdown_zone(z).await;
        assert!(stopped, "zone should have existed and been stopped");
    }

    /// Shutting down one zone must not affect others; the other zone's fetcher should keep running.
    #[tokio::test(flavor = "current_thread")]
    async fn router_isolates_zones() {
        let cfg = test_cfg();
        let ctx = test_engine_ctx();

        let router = IoRouter::new(cfg, ctx);
        let z1 = ZoneId::new();
        let z2 = ZoneId::new();

        // Spawn both zones
        let _f1 = router.get_or_spawn_zone_fetcher(z1).unwrap();
        let f2 = router.get_or_spawn_zone_fetcher(z2).unwrap();

        // Shut down z1 only
        let stopped = router.shutdown_zone(z1).await;
        assert!(stopped, "z1 should have been stopped");

        // z2 should still have a running fetcher; get_or_spawn must return the same Arc ptr
        let f2_again = router.get_or_spawn_zone_fetcher(z2).unwrap();
        assert!(Arc::ptr_eq(&f2, &f2_again), "z2 fetcher must remain the same instance");

        // Clean up remaining zones to avoid leaking tasks in test
        router.shutdown_all().await;
    }

    /// Shutting down an unknown zone is a no-op (returns false).
    #[tokio::test(flavor = "current_thread")]
    async fn router_shutdown_unknown_zone_is_noop() {
        let cfg = test_cfg();
        let ctx = test_engine_ctx();

        let router = IoRouter::new(cfg, ctx);

        let z_never_spawned = ZoneId::new();
        let stopped = router.shutdown_zone(z_never_spawned).await;
        assert!(!stopped, "unknown zone should return false on shutdown");

        // Clean (no zones to stop)
        router.shutdown_all().await;
    }
}
