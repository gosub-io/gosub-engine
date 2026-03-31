use crate::emitter::ObserverFactory;
use crate::fetcher::{Fetcher, FetcherConfig};
use crate::io_types::{IoChannel, IoCommand};
use crate::net_types::{FetchHandle, FetchRequest, FetchResult};
use crate::req_ref_tracker::RequestReferenceMap;
use crate::spawn::spawn_named;
use crate::types::ZoneId;
use dashmap::DashMap;
use std::sync::{Arc, RwLock};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::instrument;

/// Context for the IO runtime, replacing EngineContext.
pub struct IoContext {
    pub observer_factory: Arc<dyn ObserverFactory>,
    pub request_reference_map: Arc<RwLock<RequestReferenceMap>>,
}

impl Default for IoContext {
    fn default() -> Self {
        Self {
            observer_factory: Arc::new(crate::emitter::NullObserverFactory),
            request_reference_map: Arc::new(RwLock::new(RequestReferenceMap::default())),
        }
    }
}

/// Handle to the I/O runtime thread and its submission channel.
pub struct IoHandle {
    tx_submit: IoChannel,
    shutdown_tx: watch::Sender<bool>,
    join_handle: JoinHandle<()>,
}

impl IoHandle {
    pub async fn shutdown_zone(&self, zone_id: ZoneId) -> anyhow::Result<()> {
        let (tx, rx) = oneshot::channel();
        self.tx_submit
            .send(IoCommand::ShutdownZone { zone_id, reply_tx: tx })
            .map_err(|e| anyhow::anyhow!("send ShutdownZone failed: {e}"))?;
        let _ = rx
            .await
            .map_err(|e| anyhow::anyhow!("ShutdownZone ack failed: {e}"))?;
        Ok(())
    }

    #[instrument(
        name = "io.shutdown",
        level = "debug",
        skip(self),
    )]
    pub async fn shutdown(self) {
        log::trace!("signal: global shutdown -> I/O thread");
        let _ = self.shutdown_tx.send(true);

        log::trace!("signal: closing submit channel");
        drop(self.tx_submit.clone());

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

    /// Get a clone of the submission channel.
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
    zones: DashMap<ZoneId, ZoneEntry>,
    cfg: FetcherConfig,
    io_ctx: Arc<IoContext>,
}

impl IoRouter {
    pub fn new(cfg: FetcherConfig, io_ctx: Arc<IoContext>) -> Self {
        Self {
            zones: DashMap::new(),
            cfg,
            io_ctx,
        }
    }

    pub fn get_or_spawn_zone_fetcher(&self, zone_id: ZoneId) -> Arc<Fetcher> {
        if let Some(f) = self.zones.get(&zone_id) {
            return f.fetcher.clone();
        }

        let (zone_shutdown_tx, zone_shutdown_rx) = watch::channel(false);

        let f = Arc::new(Fetcher::new(
            self.cfg.clone(),
            self.io_ctx.observer_factory.clone(),
            self.io_ctx.request_reference_map.clone(),
        ));

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
            log::trace!("signal: shutdown to zone fetcher");
            let _ = entry.shutdown_tx.send(true);
            log::trace!("await: zone fetcher join");
            let _ = entry.join.await;

            true
        } else {
            false
        }
    }

    #[instrument(
        name = "io.shutdown",
        level = "debug",
        skip(self),
    )]
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

/// Spawns the IO thread and runs a single fetcher on top.
pub fn spawn_io_thread(cfg: FetcherConfig, io_ctx: Arc<IoContext>) -> IoHandle {
    let (tx_submit, mut rx_submit) = mpsc::unbounded_channel::<IoCommand>();
    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);

    let join_handle = spawn_named("I/O Thread", async move {
        let router = IoRouter::new(cfg, io_ctx);

        loop {
            tokio::select! {
                maybe_req = rx_submit.recv() => {
                    match maybe_req {
                        Some(IoCommand::Fetch { zone_id, req, handle, reply_tx }) => {
                            let fetcher = router.get_or_spawn_zone_fetcher(zone_id);
                            fetcher.submit(req, handle, reply_tx).await;
                        }
                        Some(IoCommand::Decision { zone_id, token, action }) => {
                            let fetcher = router.get_or_spawn_zone_fetcher(zone_id);
                            fetcher.fulfill(token, action).await;
                        }
                        Some(IoCommand::ShutdownZone { zone_id, reply_tx }) => {
                            let _ = router.shutdown_zone(zone_id).await;
                            let _ = reply_tx.send(());
                        }
                        None => {
                            break
                        }
                    }
                }
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        log::trace!("I/O thread received global shutdown signal");
                        break;
                    }
                }
            }
        }

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

    fn test_io_ctx() -> Arc<IoContext> {
        Arc::new(IoContext::default())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn io_driver_starts_and_global_shutdown_is_clean() {
        let ctx = test_io_ctx();
        let handle = spawn_io_thread(test_cfg(), ctx);

        sleep(Duration::from_millis(10)).await;

        timeout(Duration::from_secs(2), handle.shutdown())
            .await
            .expect("global shutdown timed out");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn io_shutdown_zone_ack_without_prior_fetcher() {
        let ctx = test_io_ctx();
        let handle = spawn_io_thread(test_cfg(), ctx);

        let z = ZoneId::new();
        timeout(Duration::from_secs(2), handle.shutdown_zone(z))
            .await
            .expect("zone shutdown ack timed out")
            .expect("zone shutdown returned error");

        timeout(Duration::from_secs(2), handle.shutdown())
            .await
            .expect("global shutdown timed out");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn router_spawns_and_shuts_down_zone() {
        let cfg = test_cfg();
        let ctx = test_io_ctx();

        let router = IoRouter::new(cfg, ctx);
        let z = ZoneId::new();

        let f = router.get_or_spawn_zone_fetcher(z);
        assert!(Arc::strong_count(&f) >= 1, "fetcher Arc should be alive");

        let stopped = router.shutdown_zone(z).await;
        assert!(stopped, "zone should have existed and been stopped");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn router_isolates_zones() {
        let cfg = test_cfg();
        let ctx = test_io_ctx();

        let router = IoRouter::new(cfg, ctx);
        let z1 = ZoneId::new();
        let z2 = ZoneId::new();

        let _f1 = router.get_or_spawn_zone_fetcher(z1);
        let f2 = router.get_or_spawn_zone_fetcher(z2);

        let stopped = router.shutdown_zone(z1).await;
        assert!(stopped, "z1 should have been stopped");

        let f2_again = router.get_or_spawn_zone_fetcher(z2);
        assert!(
            Arc::ptr_eq(&f2, &f2_again),
            "z2 fetcher must remain the same instance"
        );

        router.shutdown_all().await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn router_shutdown_unknown_zone_is_noop() {
        let cfg = test_cfg();
        let ctx = test_io_ctx();

        let router = IoRouter::new(cfg, ctx);

        let z_never_spawned = ZoneId::new();
        let stopped = router.shutdown_zone(z_never_spawned).await;
        assert!(!stopped, "unknown zone should return false on shutdown");

        router.shutdown_all().await;
    }
}
