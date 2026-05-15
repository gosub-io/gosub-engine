use crate::engine::types::IoChannel;
use crate::engine::EngineContext;
use crate::events::IoCommand;
use crate::net::fetcher::{EngineNetContext, Fetcher, FetcherConfig};
use crate::net::req_ref_tracker::RequestRefTracker;
use crate::net::types::{FetchHandle, FetchRequest, FetchResult};
use crate::util::spawn_named;
use crate::zone::ZoneId;
use dashmap::DashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::instrument;

/// Handle to the I/O runtime thread and its submission channel.
pub struct IoHandle {
    /// Channel to submit I/O requests
    tx_submit: IoChannel,
    // Send "true" when we want to shut down the IO thread (all zones)
    shutdown_tx: watch::Sender<bool>,
    // Join handle for shutdown sync
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
        log::trace!("signal: global shutdown -> I/O thread");
        // Signal global shutdown to the IO thread
        let _ = self.shutdown_tx.send(true);

        log::trace!("signal: closing submit channel");
        // Drop the submit channel so the IO loop sees EOF on rx_submit
        drop(self.tx_submit.clone());

        // Wait for the IO thread to exit
        log::trace!("await: I/O thread join");
        match self.join_handle.await {
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
    shutdown_tx: watch::Sender<bool>,
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
    // // Send "true" when we want to shut down the IO thread including ALL zone fetchers
    // io_shutdown_rx: watch::Receiver<bool>,
}

impl IoRouter {
    pub fn new(cfg: FetcherConfig, engine_ctx: Arc<EngineContext>) -> Self {
        Self {
            zones: DashMap::new(),
            cfg,
            engine_ctx,
        }
    }

    pub fn get_or_spawn_zone_fetcher(&self, zone_id: ZoneId) -> Arc<Fetcher> {
        if let Some(f) = self.zones.get(&zone_id) {
            return f.fetcher.clone();
        }

        let (zone_shutdown_tx, zone_shutdown_rx) = watch::channel(false);

        let engine_ctx = Arc::new(EngineNetContext {
            event_tx: self.engine_ctx.event_tx.clone(),
            request_reference_map: self.engine_ctx.request_reference_map.clone(),
            request_ref_tracker: Arc::new(RequestRefTracker::new()),
        });
        let f = Arc::new(Fetcher::new(self.cfg.clone(), engine_ctx).expect("reqwest client build failed"));

        let f_run = f.clone();
        let title = format!("I/O Fetcher Zone {}", zone_id);
        let join_handle = spawn_named(&title, async move {
            f_run.run(zone_shutdown_rx).await;
        });

        self.zones.insert(
            zone_id,
            ZoneEntry {
                fetcher: f.clone(),
                shutdown_tx: zone_shutdown_tx.clone(),
                join: join_handle,
            },
        );

        f
    }

    #[instrument(
        name = "zone.shutdown",
        level = "debug",
        skip(self),
        fields(zone_id = %zone_id)
    )]
    pub async fn shutdown_zone(&self, zone_id: ZoneId) -> bool {
        log::trace!("removing zone fetcher");
        if let Some((_, entry)) = self.zones.remove(&zone_id) {
            // Shutdown the fetcher
            log::trace!("signal: shutdown to zone fetcher");
            let _ = entry.shutdown_tx.send(true);
            // Wait for it to finish
            log::trace!("await: zone fetcher join");
            let _ = entry.join.await;

            true
        } else {
            false
        }
    }

    /// Shutdown the IO thread
    #[instrument(name = "io.shutdown", level = "debug", skip(self))]
    pub async fn shutdown_all(self) {
        let mut tasks = Vec::new();

        let keys: Vec<_> = self.zones.iter().map(|kv| *kv.key()).collect();
        for zone_id in keys {
            if let Some((_, entry)) = self.zones.remove(&zone_id) {
                let _ = entry.shutdown_tx.send(true);
                tasks.push(entry.join);
            }
        }

        log::trace!("await: all zone fetcher joins");
        for j in tasks {
            let _ = j.await;
        }
    }
}

pub async fn submit_to_io(
    zone_id: ZoneId,
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
        key: req.key_data.clone(),
        cancel: cancel.clone(),
    };

    io_tx
        .send(IoCommand::Fetch {
            zone_id,
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
    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);

    let join_handle = spawn_named("I/O Thread", async move {
        let router = IoRouter::new(cfg, engine_ctx);

        loop {
            tokio::select! {
                maybe_req = rx_submit.recv() => {
                    match maybe_req {
                        Some(IoCommand::Fetch { zone_id, req, handle, reply_tx } ) => {
                            let fetcher = router.get_or_spawn_zone_fetcher(zone_id);
                            fetcher.submit(req, handle, reply_tx).await;
                        }
                        Some(IoCommand::Decision { zone_id, token,action }) => {
                            let fetcher = router.get_or_spawn_zone_fetcher(zone_id);
                            fetcher.fulfill(token, action).await;
                        }
                        Some(IoCommand::ShutdownZone { zone_id, reply_tx }) => {
                            let _ = router.shutdown_zone(zone_id).await;
                            let _ = reply_tx.send(());
                        }
                        None => {
                            // All producers have dropped. Signal shutdown
                            break
                            }
                    }
                }
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        log::trace!("I/O thread received global shutdown signal");
                        break;
                    }
                    // log::error!("I/O thread spurious wakeup on shutdown signal");
                    // break;
                }
            }
        }

        // global shutdown: stop all zones cleanly
        log::trace!("I/O thread shutting down all zone fetchers");
        router.shutdown_all().await;
    });

    IoHandle {
        tx_submit,
        shutdown_tx,
        join_handle,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::time::{sleep, timeout};

    fn test_cfg() -> FetcherConfig {
        FetcherConfig {
            global_slots: 2,
            h1_per_origin: 2,
            h2_per_origin: 2,
            connect_timeout: Duration::from_millis(50),
            req_timeout: Duration::from_millis(100),
            read_idle_timeout: Duration::from_millis(100),
            total_body_timeout: Some(Duration::from_millis(150)),
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

    // -----------------------------
    // IoHandle-level tests
    // -----------------------------

    /// IO thread boots and can be globally shut down cleanly.
    #[tokio::test(flavor = "current_thread")]
    async fn io_driver_starts_and_global_shutdown_is_clean() {
        let ctx = test_engine_ctx();
        let handle = spawn_io_thread(test_cfg(), ctx);

        // Let the driver spin up
        sleep(Duration::from_millis(10)).await;

        // Global shutdown should complete promptly
        // (Assumes IoHandle::shutdown() exists, as in your earlier code.)
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

    // -----------------------------
    // Router-level tests (spawn/shutdown per-zone without network)
    // -----------------------------

    /// Spawns a per-zone fetcher on first use and shuts it down cleanly.
    #[tokio::test(flavor = "current_thread")]
    async fn router_spawns_and_shuts_down_zone() {
        let cfg = test_cfg();
        let ctx = test_engine_ctx();

        let router = IoRouter::new(cfg, ctx);
        let z = ZoneId::new();

        // Lazily create fetcher for zone z
        let f = router.get_or_spawn_zone_fetcher(z);
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
        let _f1 = router.get_or_spawn_zone_fetcher(z1);
        let f2 = router.get_or_spawn_zone_fetcher(z2);

        // Shut down z1 only
        let stopped = router.shutdown_zone(z1).await;
        assert!(stopped, "z1 should have been stopped");

        // z2 should still have a running fetcher; get_or_spawn must return the same Arc ptr
        let f2_again = router.get_or_spawn_zone_fetcher(z2);
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
