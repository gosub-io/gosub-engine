//! [`GosubEngine`]: the entry point that owns the zones, the I/O thread, and the
//! [`EngineCommand`]/[`EngineEvent`] bus.

use crate::cookies::CookieStoreHandle;
use crate::engine::events::{EngineCommand, EngineEvent};
use crate::engine::internal_pages::InternalPages;
use crate::engine::types::{EventChannel, IoChannel};
use crate::engine::DEFAULT_CHANNEL_CAPACITY;
use crate::html::RenderConfiguration;
use crate::net::req_ref_tracker::RequestReferenceMap;
use crate::net::tab_identity::TabIdentityRegistry;
use crate::net::{fetcher_config_from, spawn_io_thread, IoHandle};
use crate::zone::{Zone, ZoneConfig, ZoneId, ZoneServices, ZoneSink};
use crate::{EngineConfig, EngineError};
use anyhow::Result;
use gosub_config::Config;
use parking_lot::{Mutex, RwLock};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::sync::{broadcast, mpsc};
use tokio::time::timeout;
use tracing::instrument;

/// Main Gosub engine struct
pub struct GosubEngine<C: RenderConfiguration = crate::html::DefaultRenderConfig> {
    /// Context is what can be shared downstream
    context: Arc<EngineContext>,
    /// Active render backend, concrete per the module config `C`.
    render_backend: Arc<C::RenderBackend>,
    /// Compositor sink that receives finished frames, concrete per the module config `C`.
    /// Shared behind a plain `Arc`: the sink is interior-mutable (`submit_frame(&self)`), so no
    /// outer `RwLock` is required.
    compositor: Arc<C::CompositorSink>,
    /// The engine's single font system (the config's `FontSystem`), shared with the layouter
    /// (measurement) and the renderer (drawing) so the two agree.
    font_system: Arc<Mutex<C::FontSystem>>,
    /// Zones managed by this engine, indexed by [`ZoneId`].
    zones: HashMap<ZoneId, Arc<ZoneSink>>,
    /// Cookie stores of zones that requested persistence, flushed on shutdown.
    cookie_stores: HashMap<ZoneId, CookieStoreHandle>,
    /// Command sender used to send commands to the engine run loop.
    cmd_tx: mpsc::Sender<EngineCommand>,
    /// Command receiver (owned by the engine run loop).
    cmd_rx: Option<mpsc::Receiver<EngineCommand>>,
    /// Is the engine running?
    running: bool,

    /// I/O thread handle
    io_handle: Option<IoHandle>,
}

// Engine context that is shared downwards to zones. Renderer-agnostic: the render backend and
// compositor are concrete (per the module config) and live on `GosubEngine`/`ZoneContext`, so the
// network I/O runtime can share this context without being generic.
#[derive(Clone)]
pub struct EngineContext {
    /// Event sender
    pub event_tx: EventChannel,
    /// Global engine configuration
    pub config: Arc<EngineConfig>,
    /// Per-engine settings store (key/value config with persistence and change subscriptions).
    /// A clone of this handle is threaded down to each zone and tab.
    pub config_store: Config,
    /// I/O submission channel, installed once when the engine starts (`start()`), read by each
    /// zone at creation. A `OnceLock` rather than `Arc<RwLock<Option<..>>>`: it is set exactly once
    /// and never swapped, and `EngineContext` is already shared behind an `Arc`, so no inner lock
    /// or `Arc` is needed. Reading before `start()` yields `None` (`EngineError::IoNotStarted`).
    pub io_tx: OnceLock<IoChannel>,
    /// Map for requests to tabs
    pub request_reference_map: Arc<RwLock<RequestReferenceMap>>,
    /// `gosub://` page registry (built-ins + embedder overrides), shared with every tab.
    pub internal_pages: InternalPages,
    /// Which cookie jar and top-level document each tab has. The I/O side reads
    /// this to attach cookies itself, so no cookie value is ever handled by tab
    /// code - see [`TabIdentityRegistry`].
    pub tab_identities: Arc<TabIdentityRegistry>,
    /// The fork server renderers are forked from, if `security.renderer_process`
    /// is on and it started (set once at [`GosubEngine::start`], like `io_tx`).
    /// One per engine: what it holds (a warmed font system, a confinement tier)
    /// is engine-wide state. On the shared context so tab workers can route
    /// their renders through it; behind a `Mutex` because its request/reply
    /// protocol is strictly serial.
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    pub renderer_process: OnceLock<Arc<Mutex<crate::fork_server::client::ForkServer>>>,
    /// The resident renderers forked from it, one per (zone, site); set
    /// together with `renderer_process`. Tabs render through this.
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    pub renderer_pool: OnceLock<Arc<crate::fork_server::pool::RendererPool>>,
}

impl Default for EngineContext {
    fn default() -> Self {
        Self {
            event_tx: broadcast::channel::<EngineEvent>(DEFAULT_CHANNEL_CAPACITY).0,
            config: Arc::new(EngineConfig::default()),
            config_store: crate::engine::settings_store::default_config(),
            io_tx: OnceLock::new(),
            request_reference_map: Arc::new(RwLock::new(RequestReferenceMap::new())),
            internal_pages: InternalPages::with_builtins(),
            tab_identities: Arc::new(TabIdentityRegistry::new()),
            #[cfg(all(feature = "process-isolation", target_os = "linux"))]
            renderer_process: OnceLock::new(),
            #[cfg(all(feature = "process-isolation", target_os = "linux"))]
            renderer_pool: OnceLock::new(),
        }
    }
}

impl<C: RenderConfiguration> GosubEngine<C> {
    /// Create a new engine.
    ///
    /// If `config` is `None`, [`EngineConfig::default`] is used.
    ///
    /// ```
    /// # use gosub_engine as ge;
    /// # use std::sync::Arc;
    /// # use gosub_render_pipeline::render::backends::null::NullBackend;
    /// # use gosub_render_pipeline::render::DefaultCompositor;
    /// let backend = NullBackend::new();
    /// let compositor = DefaultCompositor::default();
    /// let engine = ge::GosubEngine::<ge::DefaultRenderConfig>::new(None, Arc::new(backend), Arc::new(compositor));
    /// ```
    pub fn new(
        config: Option<EngineConfig>,
        backend: Arc<C::RenderBackend>,
        compositor: Arc<C::CompositorSink>,
    ) -> Self {
        let resolved_config = config.unwrap_or_default();

        // Command channel on which to send and receive engine commands from the UA.
        let (cmd_tx, cmd_rx) = mpsc::channel::<EngineCommand>(DEFAULT_CHANNEL_CAPACITY);

        // Broadcast event bus. Subscribe to receive engine events (including zone and tab events)
        let (event_tx, _first_rx) = broadcast::channel::<EngineEvent>(DEFAULT_CHANNEL_CAPACITY);

        Self {
            context: Arc::new(EngineContext {
                event_tx: event_tx.clone(),
                config: Arc::new(resolved_config),
                config_store: crate::engine::settings_store::default_config(),
                io_tx: OnceLock::new(),
                request_reference_map: Arc::new(RwLock::new(RequestReferenceMap::new())),
                internal_pages: InternalPages::with_builtins(),
                tab_identities: Arc::new(TabIdentityRegistry::new()),
                #[cfg(all(feature = "process-isolation", target_os = "linux"))]
                renderer_process: OnceLock::new(),
                #[cfg(all(feature = "process-isolation", target_os = "linux"))]
                renderer_pool: OnceLock::new(),
            }),
            render_backend: backend,
            compositor,
            font_system: Arc::new(Mutex::new(C::FontSystem::default())),
            zones: HashMap::new(),
            cookie_stores: HashMap::new(),
            cmd_tx,
            cmd_rx: Some(cmd_rx),
            io_handle: None,
            running: false,
        }
    }

    /// Starts the engine's I/O runtime and returns the main run-loop future.
    ///
    /// The returned future is intentionally not spawned: the caller decides how to drive it -
    /// `tokio::spawn` it onto a background task, `.await` it inline, or poll it inside a `select!`.
    /// This keeps the engine from imposing a runtime/threading model on the embedder (it can be
    /// driven on the caller's current task/thread). The engine is considered running as soon as
    /// this returns `Ok`; driving the future processes engine commands such as shutdown.
    pub fn start(&mut self) -> Result<impl std::future::Future<Output = ()> + 'static, EngineError> {
        if self.running {
            return Err(EngineError::AlreadyRunning);
        }

        // Isolation needs the embedder's cooperation; without it every child
        // would boot the embedder instead. Decided before the I/O thread, which
        // is what spawns the network process.
        #[cfg(feature = "process-isolation")]
        self.isolation_needs_dispatch();

        // Start I/O thread, building the fetcher config from the settings store.
        let io_cfg = fetcher_config_from(&self.context.config_store);
        let io_handle = spawn_io_thread(io_cfg, self.context.clone());
        // Set once; `start()` already refuses to run twice, so this never races or overwrites.
        let _ = self.context.io_tx.set(io_handle.subscribe());
        self.io_handle = Some(io_handle);

        // Start metrics HTTP server (GET http://127.0.0.1:9090/metrics)
        #[cfg(feature = "metrics")]
        crate::metrics::start(9090, Arc::clone(&self.context));

        // Spawn the renderer fork server if asked to. Blocks briefly (spawn
        // plus font warm-up, ~200 ms typical) - acceptable at startup, and
        // the answer decides engine-wide behaviour, so it belongs here.
        #[cfg(all(feature = "process-isolation", target_os = "linux"))]
        self.start_renderer_process();

        // Hand the run-loop future to the caller to drive (spawn / await / select!) rather than
        // spawning it ourselves. `run()` yields `None` only if the loop was already taken, which
        // cannot happen here since `self.running` was false above.
        self.run().ok_or(EngineError::AlreadyRunning)
    }

    /// Turn the `security.*` process settings off when the embedder never called
    /// `child_process::dispatch()`: a child is this binary re-exec'd, and without
    /// dispatch it would run the embedder's own `main()` - for a GUI embedder, a
    /// phantom window per spawn. One warning names the omission.
    #[cfg(feature = "process-isolation")]
    fn isolation_needs_dispatch(&self) {
        const PROCESS_SETTINGS: [&str; 3] = [
            "security.network_process",
            "security.image_decoder_process",
            "security.renderer_process",
        ];
        if crate::child_process::was_dispatched() {
            return;
        }
        let requested: Vec<&str> = PROCESS_SETTINGS
            .into_iter()
            .filter(|key| self.context.config_store.get_bool(key))
            .collect();
        if requested.is_empty() {
            return;
        }
        log::warn!(
            "{} requested, but gosub_engine::child_process::dispatch() was not called at the top of \
             main(); running without process isolation",
            requested.join(", ")
        );
        for key in requested {
            let _ = self
                .context
                .config_store
                .set(key, gosub_config::settings::Setting::Bool(false));
        }
    }

    /// Spawn the fork server when `security.renderer_process` asks for it.
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    fn start_renderer_process(&mut self) {
        use crate::fork_server::client::ForkServer;
        use crate::fork_server::protocol::ConfinementTier;

        if !self.context.config_store.get_bool("security.renderer_process") {
            return;
        }

        // The configured font system's (static) tier decides the mechanism:
        // only `Full` systems benefit from a warmed fork server.
        // `FontPathsReadable` renders in throwaway exec'd processes spawned
        // per render (see `render_process`) - nothing to start here.
        {
            use gosub_interface::font_system::{Confinement, FontSystem as _};
            match C::FontSystem::confinement() {
                Confinement::Full => {}
                Confinement::FontPathsReadable => {
                    log::info!(
                        "renderer isolation active in exec-per-render mode \
                         (the configured font system reads font files while operating)"
                    );
                    return;
                }
                Confinement::Unsupported(reason) => {
                    log::warn!(
                        "security.renderer_process is on, but the configured font system cannot run \
                         isolated ({reason}); rendering stays in-process"
                    );
                    return;
                }
            }
        }

        // A renderer that cannot rasterize would ship geometry and no pixels:
        // blank tabs with no way back. Better to say so and stay in-process.
        {
            let fonts: Arc<Mutex<dyn gosub_interface::font_system::FontSystem>> =
                Arc::new(Mutex::new(C::FontSystem::default()));
            if C::forked_tile_rasterizer(fonts).is_none() {
                log::warn!(
                    "security.renderer_process is on, but this RenderConfiguration provides no \
                     forked_tile_rasterizer (enable the engine's `cairo-tiles`/`skia-tiles` feature, \
                     or implement it); rendering stays in-process"
                );
                let _ = self.context.config_store.set(
                    "security.renderer_process",
                    gosub_config::settings::Setting::Bool(false),
                );
                return;
            }
        }

        match ForkServer::spawn() {
            Ok(mut server) => {
                let tier = server.confinement().clone();
                match tier {
                    ConfinementTier::Unsupported(reason) => {
                        log::warn!(
                            "security.renderer_process is on, but the configured font system cannot run \
                             isolated ({reason}); rendering stays in-process"
                        );
                        server.shutdown();
                    }
                    tier => {
                        log::info!("renderer fork server ready (confinement tier: {tier:?})");
                        // Set once, like `io_tx`; `start()` refuses to run twice.
                        let server = Arc::new(Mutex::new(server));
                        let _ = self
                            .context
                            .renderer_pool
                            .set(Arc::new(crate::fork_server::pool::RendererPool::new(
                                Arc::clone(&server),
                                Some(self.context.event_tx.clone()),
                            )));
                        let _ = self.context.renderer_process.set(server);
                    }
                }
            }
            Err(e) => {
                log::warn!(
                    "security.renderer_process is on, but the fork server could not be started ({e}); \
                     rendering stays in-process. The most likely cause is an embedder that has not \
                     called gosub_engine::child_process::dispatch_with() first thing in main()."
                );
            }
        }
    }

    /// The running renderer fork server, when `security.renderer_process` is on
    /// and it started - the handle render routing goes through. `None` means
    /// this engine renders in-process.
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    pub fn renderer_process(&self) -> Option<&Arc<Mutex<crate::fork_server::client::ForkServer>>> {
        self.context.renderer_process.get()
    }

    /// The pool of resident renderers, when `security.renderer_process` is on
    /// and the fork server started: one process per (zone, site), listable
    /// for diagnostics.
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    pub fn renderer_pool(&self) -> Option<&Arc<crate::fork_server::pool::RendererPool>> {
        self.context.renderer_pool.get()
    }

    /// The confinement tier the renderer fork server announced, when one is
    /// running: how confined this engine's forked renderers are.
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    pub fn renderer_process_tier(&self) -> Option<crate::fork_server::protocol::ConfinementTier> {
        self.context
            .renderer_process
            .get()
            .map(|server| server.lock().confinement().clone())
    }

    /// Return a receiver for engine events.
    pub fn subscribe_events(&self) -> broadcast::Receiver<EngineEvent> {
        self.context.event_tx.subscribe()
    }

    /// The `gosub://` page registry. Register a provider to add an internal page or override
    /// a built-in one (e.g. a branded `home`); see [`InternalPages`].
    pub fn internal_pages(&self) -> &InternalPages {
        &self.context.internal_pages
    }

    /// The engine's settings store, for reading or overriding settings (e.g.
    /// `net.user_agent`). Network settings are read once when [`start`](Self::start)
    /// builds the I/O runtime, so overrides must land before then.
    pub fn settings(&self) -> &Config {
        &self.context.config_store
    }

    pub fn backend(&self) -> Arc<C::RenderBackend> {
        Arc::clone(&self.render_backend)
    }

    /// Give this to zones/tabs when constructing them.
    pub fn compositor(&self) -> Arc<C::CompositorSink> {
        Arc::clone(&self.compositor)
    }

    /// Get a clone of the engine’s command sender (mainly for testing or
    /// custom handles).
    #[cfg(test)]
    #[allow(unused)]
    fn command_sender(&self) -> mpsc::Sender<EngineCommand> {
        self.cmd_tx.clone()
    }

    /// Build the engine’s inbound command-loop future (owns everything it needs, hence `'static`).
    ///
    /// Returns `None` if the loop was already taken (engine already started). The caller drives the
    /// future; this method does not spawn it.
    pub fn run(&mut self) -> Option<impl std::future::Future<Output = ()> + 'static> {
        self.running = true;

        let _ = self.context.event_tx.send(EngineEvent::EngineStarted);

        let mut cmd_rx = self.cmd_rx.take()?;

        Some(async move {
            // `Shutdown` is currently the only engine command; turn this back into a
            // dispatch loop once more commands exist.
            if let Some(EngineCommand::Shutdown { reply }) = cmd_rx.recv().await {
                log::trace!("Engine received shutdown command. Shutting down main engine::run() loop");
                let _ = reply.send(Ok(()));
            }
        })
    }

    /// Shuts down the engine
    ///
    #[instrument(name = "engine.shutdown", level = "debug", skip(self))]
    pub async fn shutdown(&mut self) -> Result<(), EngineError> {
        if !self.running {
            return Err(EngineError::NotRunning);
        }

        // Persist cookie stores before tearing anything down.
        self.flush_persistence();

        // Ask the fork server for a clean exit (it kills-and-reaps on drop
        // regardless, but a Shutdown lets it leave without a SIGKILL).
        #[cfg(all(feature = "process-isolation", target_os = "linux"))]
        {
            if let Some(pool) = self.context.renderer_pool.get() {
                log::trace!("signal: shutting down the resident renderers");
                pool.shutdown_all();
            }
            if let Some(server) = self.context.renderer_process.get() {
                log::trace!("signal: shutting down the renderer fork server");
                server.lock().shutdown();
            }
        }

        // Shutdown I/O thread
        log::trace!("signal: shutting down I/O thread");
        let shutdown_secs = self.context.config_store.get_uint("engine.io_shutdown_secs") as u64;
        if let Some(io) = self.io_handle.take() {
            if let Err(e) = timeout(Duration::from_secs(shutdown_secs), io.shutdown()).await {
                log::warn!("I/O shutdown timed out: {e}");
            }
        } else {
            log::debug!("I/O handle already gone");
        }

        // Send shutdown command to the run loop
        log::trace!("signal: sending shutdown to run loop");
        let (tx, rx) = tokio::sync::oneshot::channel();
        let _ = self.cmd_tx.try_send(EngineCommand::Shutdown { reply: tx });

        // Wait for confirmation that the run loop has exited
        let _ = rx.await.map_err(|e| EngineError::Internal(e.into()))?;
        log::trace!("engine shutdown complete");

        Ok(())
    }

    /// Flush all persistent state (currently: cookie stores) to disk.
    fn flush_persistence(&self) {
        for (zone_id, store) in &self.cookie_stores {
            log::trace!("persisting cookie store of zone {zone_id}");
            store.persist_all();
        }
    }

    /// Create and register a new zone, returning a [`Zone`] for userland code.
    ///
    /// `None` for `config` uses the engine's [`EngineConfig::default_zone_config`];
    /// `None` for `zone_id` generates a fresh id. Fails with
    /// [`EngineError::ZoneLimitExceeded`] once the engine holds
    /// [`EngineConfig::max_zones`] zones. The returned handle carries the [`ZoneId`]
    /// and a clone of the engine's command sender, so the caller can send zone
    /// commands without holding a reference to the engine.
    pub fn create_zone(
        &mut self,
        config: Option<ZoneConfig>,
        services: ZoneServices,
        zone_id: Option<ZoneId>,
    ) -> Result<Zone<C>, EngineError> {
        if self.zones.len() >= self.context.config.max_zones {
            return Err(EngineError::ZoneLimitExceeded);
        }
        let config = config.unwrap_or_else(|| self.context.config.default_zone_config.clone());
        let cookie_store = services.cookie_store.clone();

        let zone = match zone_id {
            Some(zone_id) => Zone::new_with_id(
                zone_id,
                config,
                services,
                self.context.clone(),
                self.render_backend.clone(),
                self.compositor.clone(),
                self.font_system.clone(),
            )?,
            None => Zone::new(
                config,
                services,
                self.context.clone(),
                self.render_backend.clone(),
                self.compositor.clone(),
                self.font_system.clone(),
            )?,
        };

        let zone_id = zone.id;
        self.zones.insert(zone.id, zone.sink.clone());
        if let Some(store) = cookie_store {
            self.cookie_stores.insert(zone_id, store);
        }

        self.context
            .event_tx
            .send(EngineEvent::ZoneCreated { zone_id })
            .map_err(|e| EngineError::Internal(e.into()))?;

        Ok(zone)
    }

    /// Close a zone: stop its tabs and fetcher, release its cookie jar, and free
    /// its [`EngineConfig::max_zones`] slot.
    ///
    /// Persisted cookie data stays on disk (the zone can be reopened later with the
    /// same [`ZoneId`]); only the in-memory state is released. Emits
    /// [`EngineEvent::ZoneClosed`] when done.
    #[instrument(name = "engine.close_zone", level = "debug", skip(self, zone))]
    pub async fn close_zone(&mut self, zone: Zone<C>) {
        let zone_id = zone.id;

        // Stop all tab workers first, so nothing fetches or mutates cookies below.
        zone.close().await;

        // Shut down the zone's fetcher on the I/O thread (ack'd).
        if let Some(io) = &self.io_handle {
            let secs = self.context.config_store.get_uint("engine.io_shutdown_secs") as u64;
            match timeout(Duration::from_secs(secs), io.shutdown_zone(zone_id)).await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => log::warn!("Zone {zone_id} I/O shutdown failed: {e}"),
                Err(_) => log::warn!("Zone {zone_id} I/O shutdown timed out after {secs}s"),
            }
        }

        // Final cookie snapshot + cache eviction; durable data stays on disk.
        if let Some(store) = self.cookie_stores.remove(&zone_id) {
            store.release_zone(zone_id);
        }

        self.zones.remove(&zone_id);

        let _ = self.context.event_tx.send(EngineEvent::ZoneClosed { zone_id });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{InMemoryLocalStore, InMemorySessionStore, PartitionPolicy, StorageService};
    use gosub_render_pipeline::render::backends::null::NullBackend;
    use gosub_render_pipeline::render::DefaultCompositor;

    fn services() -> ZoneServices {
        ZoneServices {
            storage: Arc::new(StorageService::new(
                Arc::new(InMemoryLocalStore::new()),
                Arc::new(InMemorySessionStore::new()),
            )),
            cookie_store: None,
            cookie_jar: None,
            partition_policy: PartitionPolicy::None,
            places: None,
        }
    }

    /// Without `child_process::dispatch()` the process settings must not survive
    /// `start()`: a child would re-exec into this test binary's own startup.
    #[cfg(feature = "process-isolation")]
    #[tokio::test]
    async fn process_settings_are_dropped_without_dispatch() {
        use gosub_config::settings::Setting;
        let mut engine = engine_with_max_zones(1);
        for key in [
            "security.network_process",
            "security.image_decoder_process",
            "security.renderer_process",
        ] {
            engine.settings().set(key, Setting::Bool(true)).expect("set");
            assert!(engine.settings().get_bool(key));
        }
        assert!(!crate::child_process::was_dispatched());
        let _join = tokio::spawn(engine.start().expect("start"));
        for key in [
            "security.network_process",
            "security.image_decoder_process",
            "security.renderer_process",
        ] {
            assert!(!engine.settings().get_bool(key), "{key} should have been turned off");
        }
    }

    fn engine_with_max_zones(max_zones: usize) -> GosubEngine {
        let settings = EngineConfig::builder().max_zones(max_zones).build().unwrap();
        GosubEngine::new(
            Some(settings),
            Arc::new(NullBackend::new()),
            Arc::new(DefaultCompositor::default()),
        )
    }

    /// The inversion, end to end: the I/O side stores a `Set-Cookie` from one
    /// navigation and attaches it to the next, with no cookie code on the tab
    /// path at all. Both halves are covered - a failure to store and a failure to
    /// attach look identical here, which is why the second request is inspected
    /// rather than the jar.
    #[tokio::test]
    async fn cookies_are_stored_and_replayed_by_the_io_side() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        // Only the second request matters; the first exists to hand out the cookie.
        let second_request = Arc::new(Mutex::new(String::new()));
        let captured = second_request.clone();

        tokio::spawn(async move {
            for i in 0..2 {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                let mut buf = vec![0u8; 4096];
                let n = stream.read(&mut buf).await.unwrap_or(0);
                if i == 1 {
                    *captured.lock() = String::from_utf8_lossy(&buf[..n]).to_string();
                }

                let body = b"<html><title>hi</title></html>";
                let set_cookie = if i == 0 {
                    "Set-Cookie: sid=abc123; Path=/\r\n"
                } else {
                    ""
                };
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n{set_cookie}Content-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes()).await;
                let _ = stream.write_all(body).await;
            }
        });

        let mut engine = engine_with_max_zones(1);
        let _event_rx = engine.subscribe_events();
        let _join = tokio::spawn(engine.start().expect("start"));

        let mut zone = engine.create_zone(None, services(), None).expect("zone");
        // One tab for both navigations: with no zone store or jar configured every
        // tab gets its own jar, so a second tab would start empty.
        let tab = zone.create_tab(Default::default(), None).await.expect("tab");

        tab.navigate(format!("http://127.0.0.1:{port}/first"))
            .await
            .expect("first navigation");
        // The store happens on the I/O side after the response arrives, so the
        // second navigation must not start until the first has been answered.
        tokio::time::sleep(Duration::from_millis(300)).await;

        tab.navigate(format!("http://127.0.0.1:{port}/second"))
            .await
            .expect("second navigation");

        let mut request = String::new();
        for _ in 0..100 {
            request = second_request.lock().clone();
            if !request.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        use cow_utils::CowUtils;
        assert!(
            request.cow_to_ascii_lowercase().contains("cookie: sid=abc123"),
            "the I/O side should have stored and replayed the cookie, got:\n{request}"
        );

        engine.close_zone(zone).await;
        engine.shutdown().await.expect("shutdown");
    }

    #[tokio::test]
    async fn cookie_store_persists_on_shutdown() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cookies.json");
        let store: CookieStoreHandle = crate::cookies::JsonCookieStore::new(path.clone()).unwrap().into();

        let mut engine = engine_with_max_zones(1);
        let _event_rx = engine.subscribe_events();
        let _join = tokio::spawn(engine.start().expect("start"));

        let mut zone_services = services();
        zone_services.cookie_store = Some(store.clone());
        let mut zone = engine.create_zone(None, zone_services, None).expect("zone");

        // Tab creation resolves the persistent per-zone jar from the store.
        let _tab = zone.create_tab(Default::default(), None).await.expect("tab");

        // Store a cookie through the zone's (memoized) persistent jar.
        let jar = store.jar_for(zone.id).expect("persistent jar");
        let url = url::Url::parse("https://example.com/").unwrap();
        let mut headers = http::HeaderMap::new();
        headers.append(http::header::SET_COOKIE, "sid=abc123; Path=/".parse().unwrap());
        jar.write().store_response_cookies(&url, &headers, None);

        engine.shutdown().await.expect("shutdown");

        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(
            contents.contains("sid") && contents.contains("abc123"),
            "cookie should be persisted on shutdown, got: {contents}"
        );
    }

    #[tokio::test]
    async fn accept_language_is_sent_with_navigation_requests() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        // Tiny one-shot HTTP server that captures the request it receives.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let captured = Arc::new(Mutex::new(String::new()));
        let captured_srv = captured.clone();
        tokio::spawn(async move {
            if let Ok((mut stream, _)) = listener.accept().await {
                let mut buf = vec![0u8; 4096];
                let n = stream.read(&mut buf).await.unwrap_or(0);
                *captured_srv.lock() = String::from_utf8_lossy(&buf[..n]).to_string();
                let body = b"<html><title>hi</title></html>";
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes()).await;
                let _ = stream.write_all(body).await;
            }
        });

        let mut engine = engine_with_max_zones(1);
        let _event_rx = engine.subscribe_events();
        let _join = tokio::spawn(engine.start().expect("start"));

        let zone_cfg = ZoneConfig::builder()
            .accept_languages("fr-CH, fr;q=0.9")
            .build()
            .unwrap();
        let mut zone = engine.create_zone(Some(zone_cfg), services(), None).expect("zone");
        let tab = zone.create_tab(Default::default(), None).await.expect("tab");
        tab.navigate(format!("http://127.0.0.1:{port}/"))
            .await
            .expect("navigate");

        // Wait for the server to capture the request.
        let mut request = String::new();
        for _ in 0..100 {
            request = captured.lock().clone();
            if !request.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        use cow_utils::CowUtils;
        assert!(
            request
                .cow_to_ascii_lowercase()
                .contains("accept-language: fr-ch, fr;q=0.9"),
            "expected Accept-Language header in request, got:\n{request}"
        );

        engine.close_zone(zone).await;
        engine.shutdown().await.expect("shutdown");
    }

    /// Session history end to end: two navigations push two entries, GoBack moves the cursor
    /// (announced immediately via HistoryChanged) and refetches the first page, GoForward
    /// returns to the second. Verifies the tree from the embedder's point of view only.
    /// Downloads end to end: navigating to binary content emits a DownloadRequested offer
    /// (with the Content-Disposition filename) and cancels the navigation; StartDownload
    /// streams the bytes to the chosen path and reports progress and completion.
    #[tokio::test]
    async fn navigation_download_offer_and_save() {
        use crate::events::{DownloadId, TabCommand};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        // 700 KiB of deterministic bytes: enough for several progress reports.
        let payload: Vec<u8> = (0..700 * 1024).map(|i| (i % 251) as u8).collect();
        let payload_srv = payload.clone();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let payload = payload_srv.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 4096];
                    let _ = stream.read(&mut buf).await;
                    let head = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\n\
                         Content-Disposition: attachment; filename=\"pretty.bin\"\r\n\
                         Content-Length: {}\r\nConnection: close\r\n\r\n",
                        payload.len()
                    );
                    let _ = stream.write_all(head.as_bytes()).await;
                    let _ = stream.write_all(&payload).await;
                });
            }
        });

        let mut engine = engine_with_max_zones(1);
        let mut event_rx = engine.subscribe_events();
        let _join = tokio::spawn(engine.start().expect("start"));
        let mut zone = engine.create_zone(None, services(), None).expect("zone");
        let tab = zone.create_tab(Default::default(), None).await.expect("tab");

        // Navigating to the binary produces an offer, not an error page.
        tab.navigate(format!("http://127.0.0.1:{port}/data/raw.bin"))
            .await
            .expect("navigate");
        let offer = tokio::time::timeout(Duration::from_secs(10), async {
            let mut nav_progress = false;
            loop {
                match event_rx.recv().await {
                    // The document fetch of the navigation reports load progress.
                    Ok(EngineEvent::Navigation {
                        event: crate::events::NavigationEvent::Progress { .. },
                        ..
                    }) => nav_progress = true,
                    Ok(EngineEvent::DownloadRequested {
                        url,
                        suggested_filename,
                        total_bytes,
                        ..
                    }) => return (url, suggested_filename, total_bytes, nav_progress),
                    Ok(_) => continue,
                    Err(e) => panic!("event stream closed: {e}"),
                }
            }
        })
        .await
        .expect("timed out waiting for DownloadRequested");
        assert!(offer.3, "expected NavigationEvent::Progress during the document fetch");
        assert_eq!(offer.1, "pretty.bin", "Content-Disposition filename wins");
        assert_eq!(offer.2, Some(payload.len() as u64));

        // Accept the offer into a temp dir.
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("saved.bin");
        tab.send(TabCommand::StartDownload {
            id: DownloadId(7),
            url: offer.0.to_string(),
            target_path: target.clone(),
        })
        .await
        .expect("start download");

        let (progress_seen, partial_seen, received) = tokio::time::timeout(Duration::from_secs(10), async {
            let mut progress_seen = false;
            let mut partial_seen = false;
            loop {
                match event_rx.recv().await {
                    Ok(EngineEvent::DownloadProgress {
                        id: DownloadId(7),
                        received_bytes,
                        total_bytes,
                        ..
                    }) => {
                        progress_seen = true;
                        // Granular progress: at least one event mid-transfer, not only the
                        // final full-size report.
                        if total_bytes.is_some_and(|t| received_bytes < t) {
                            partial_seen = true;
                        }
                    }
                    Ok(EngineEvent::DownloadFinished {
                        id: DownloadId(7),
                        received_bytes,
                        path,
                        ..
                    }) => {
                        assert_eq!(path, target);
                        return (progress_seen, partial_seen, received_bytes);
                    }
                    Ok(EngineEvent::DownloadFailed { error, .. }) => panic!("download failed: {error}"),
                    Ok(_) => continue,
                    Err(e) => panic!("event stream closed: {e}"),
                }
            }
        })
        .await
        .expect("timed out waiting for DownloadFinished");
        assert!(progress_seen, "expected at least one progress event");
        assert!(
            partial_seen,
            "expected granular mid-transfer progress, not only the final report"
        );
        assert_eq!(received, payload.len() as u64);
        assert_eq!(std::fs::read(&target).unwrap(), payload, "file content must match");

        // The failed-fetch path reports too (connection refused port).
        tab.send(TabCommand::StartDownload {
            id: DownloadId(8),
            url: "http://127.0.0.1:9/off".into(),
            target_path: dir.path().join("nope.bin"),
        })
        .await
        .expect("start failing download");
        let failed = wait_for(&mut event_rx, |ev| {
            matches!(ev, EngineEvent::DownloadFailed { id: DownloadId(8), .. })
        })
        .await;
        assert!(failed, "expected DownloadFailed for unreachable server");

        engine.close_zone(zone).await;
        engine.shutdown().await.expect("shutdown");
    }

    /// Committed http navigations are recorded into the zone's places store (visited
    /// history), with the page title; internal pages are not.
    #[tokio::test]
    async fn visits_are_recorded_in_places() {
        use crate::places::{MemoryPlaces, Places};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 4096];
                    let _ = stream.read(&mut buf).await;
                    let body = "<html><title>A Page</title><body>hi</body></html>";
                    let head = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(head.as_bytes()).await;
                    let _ = stream.write_all(body.as_bytes()).await;
                });
            }
        });

        let places = Arc::new(MemoryPlaces::new());
        let mut engine = engine_with_max_zones(1);
        let mut event_rx = engine.subscribe_events();
        let _join = tokio::spawn(engine.start().expect("start"));
        let mut zone_services = services();
        zone_services.places = Some(places.clone());
        let mut zone = engine.create_zone(None, zone_services, None).expect("zone");
        let tab = zone.create_tab(Default::default(), None).await.expect("tab");

        let url = format!("http://127.0.0.1:{port}/page");
        tab.navigate(url.clone()).await.expect("navigate");
        let finished = wait_for(&mut event_rx, |ev| {
            matches!(
                ev,
                EngineEvent::Navigation {
                    event: crate::events::NavigationEvent::Finished { .. },
                    ..
                }
            )
        })
        .await;
        assert!(finished);
        let visited = places.query_visited("", 10);
        assert_eq!(visited.len(), 1);
        assert_eq!(visited[0].url, url);
        assert_eq!(visited[0].title, "A Page");

        // Internal pages leave no trace.
        tab.navigate("gosub://version").await.expect("navigate internal");
        let finished = wait_for(&mut event_rx, |ev| {
            matches!(
                ev,
                EngineEvent::Navigation {
                    event: crate::events::NavigationEvent::Finished { url, .. },
                    ..
                } if url.scheme() == "gosub"
            )
        })
        .await;
        assert!(finished);
        assert_eq!(places.query_visited("", 10).len(), 1);

        engine.close_zone(zone).await;
        engine.shutdown().await.expect("shutdown");
    }

    /// Crash containment: a panicking tab worker produces a TabCrashed event (instead of
    /// dying silently), and the tab's handle then reports closed on further commands.
    #[tokio::test]
    async fn worker_panic_emits_tab_crashed() {
        use crate::events::TabCommand;

        let mut engine = engine_with_max_zones(1);
        let mut event_rx = engine.subscribe_events();
        let _join = tokio::spawn(engine.start().expect("start"));
        let mut zone = engine.create_zone(None, services(), None).expect("zone");
        let tab = zone.create_tab(Default::default(), None).await.expect("tab");
        let tab_id = tab.tab_id;

        tab.send(TabCommand::CrashForTest).await.expect("send crash command");

        let crashed = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                match event_rx.recv().await {
                    Ok(EngineEvent::TabCrashed {
                        tab_id: crashed_id,
                        error,
                        ..
                    }) => return (crashed_id, error),
                    Ok(_) => continue,
                    Err(e) => panic!("event stream closed: {e}"),
                }
            }
        })
        .await
        .expect("timed out waiting for TabCrashed");
        assert_eq!(crashed.0, tab_id);
        assert!(
            crashed.1.contains("deliberate test crash"),
            "panic message: {}",
            crashed.1
        );

        // The dead tab's handle fails cleanly rather than hanging.
        assert!(tab.send(TabCommand::Reload { ignore_cache: false }).await.is_err());

        engine.close_zone(zone).await;
        engine.shutdown().await.expect("shutdown");
    }

    /// Keyboard focus end to end: Tab focuses the first link (FocusChanged), Enter
    /// activates it (a real navigation to its href).
    #[tokio::test]
    async fn keyboard_focus_and_link_activation() {
        use crate::events::{NavigationEvent, TabCommand};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 4096];
                    let _ = stream.read(&mut buf).await;
                    let body = "<html><title>t</title><body><a href=\"/target\">go</a></body></html>";
                    let head = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(head.as_bytes()).await;
                    let _ = stream.write_all(body.as_bytes()).await;
                });
            }
        });

        let mut engine = engine_with_max_zones(1);
        let mut event_rx = engine.subscribe_events();
        let _join = tokio::spawn(engine.start().expect("start"));
        let mut zone = engine.create_zone(None, services(), None).expect("zone");
        let tab = zone.create_tab(Default::default(), None).await.expect("tab");
        tab.send(TabCommand::SetViewport {
            x: 0,
            y: 0,
            width: 640,
            height: 480,
        })
        .await
        .expect("viewport");

        tab.navigate(format!("http://127.0.0.1:{port}/"))
            .await
            .expect("navigate");
        wait_for(&mut event_rx, |ev| {
            matches!(
                ev,
                EngineEvent::Navigation {
                    event: NavigationEvent::Finished { .. },
                    ..
                }
            )
        })
        .await;

        // Tab -> the link gets focus; the engine announces it (non-editable).
        tab.send(TabCommand::KeyDown {
            key: "Tab".into(),
            code: "Tab".into(),
            modifiers: crate::engine::events::Modifiers::empty(),
        })
        .await
        .expect("tab key");
        let matched = wait_for(&mut event_rx, |ev| {
            matches!(
                ev,
                EngineEvent::FocusChanged {
                    focused: true,
                    editable: false,
                    ..
                }
            )
        })
        .await;
        assert!(matched, "expected FocusChanged after Tab");

        // Enter -> activates the focused link: a navigation to /target starts.
        tab.send(TabCommand::KeyDown {
            key: "Enter".into(),
            code: "Enter".into(),
            modifiers: crate::engine::events::Modifiers::empty(),
        })
        .await
        .expect("enter key");
        let matched = wait_for(&mut event_rx, |ev| {
            matches!(
                ev,
                EngineEvent::Navigation {
                    event: NavigationEvent::Started { url, .. },
                    ..
                } if url.path() == "/target"
            )
        })
        .await;
        assert!(matched, "expected navigation to /target after Enter");

        engine.close_zone(zone).await;
        engine.shutdown().await.expect("shutdown");
    }

    /// Wait (with timeout) for an event matching `pred`; true when it arrived.
    async fn wait_for(rx: &mut broadcast::Receiver<EngineEvent>, pred: impl Fn(&EngineEvent) -> bool) -> bool {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                match rx.recv().await {
                    Ok(ev) if pred(&ev) => return true,
                    Ok(_) => continue,
                    Err(_) => return false,
                }
            }
        })
        .await
        .unwrap_or(false)
    }

    #[tokio::test]
    async fn session_history_back_and_forward() {
        use crate::events::NavigationEvent;
        use crate::tab::HistoryEntryId;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        // Tiny HTTP server answering every request; records the paths it served.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let served = Arc::new(Mutex::new(Vec::<String>::new()));
        let served_srv = served.clone();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let served = served_srv.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 4096];
                    let n = stream.read(&mut buf).await.unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]).to_string();
                    let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
                    if path != "/icon.png" {
                        served.lock().push(path.clone());
                    }
                    // Every page advertises the same icon; the icon itself is a fixed byte blob.
                    let (ctype, body): (&str, Vec<u8>) = if path == "/icon.png" {
                        ("image/png", b"PNGBYTES".to_vec())
                    } else {
                        (
                            "text/html",
                            format!(
                                "<html><head><title>{path}</title><link rel=\"icon\" href=\"/icon.png\"></head><body>{path}</body></html>"
                            )
                            .into_bytes(),
                        )
                    };
                    let head = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(head.as_bytes()).await;
                    let _ = stream.write_all(&body).await;
                });
            }
        });

        let mut engine = engine_with_max_zones(1);
        let mut event_rx = engine.subscribe_events();
        let _join = tokio::spawn(engine.start().expect("start"));
        let mut zone = engine.create_zone(None, services(), None).expect("zone");
        let tab = zone.create_tab(Default::default(), None).await.expect("tab");

        // Collect HistoryChanged snapshots until `pred` holds (or time out).
        async fn next_history(
            rx: &mut tokio::sync::broadcast::Receiver<EngineEvent>,
            pred: impl Fn(&crate::tab::HistorySnapshot) -> bool,
        ) -> crate::tab::HistorySnapshot {
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    if let Ok(EngineEvent::Navigation {
                        event: NavigationEvent::HistoryChanged { history },
                        ..
                    }) = rx.recv().await
                    {
                        if pred(&history) {
                            return history;
                        }
                    }
                }
            })
            .await
            .expect("timed out waiting for HistoryChanged")
        }

        let a = format!("http://127.0.0.1:{port}/a");
        let b = format!("http://127.0.0.1:{port}/b");

        tab.navigate(a.clone()).await.expect("navigate a");
        let h = next_history(&mut event_rx, |h| h.entries.len() == 1).await;
        assert_eq!(h.current, Some(HistoryEntryId(0)));
        assert!(!h.can_go_back);
        assert!(h.forward.is_empty());
        assert_eq!(h.entries[0].url.as_str(), a);
        assert_eq!(h.entries[0].title.as_deref(), Some("/a"));

        // The page's <link rel=icon> is fetched through the zone fetcher and delivered as bytes.
        let favicon = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Ok(EngineEvent::FavIconChanged { favicon, .. }) = event_rx.recv().await {
                    return favicon;
                }
            }
        })
        .await
        .expect("timed out waiting for FavIconChanged");
        assert_eq!(favicon, b"PNGBYTES");

        tab.navigate(b.clone()).await.expect("navigate b");
        let h = next_history(&mut event_rx, |h| h.entries.len() == 2).await;
        assert_eq!(h.current, Some(HistoryEntryId(1)));
        assert!(h.can_go_back);
        assert!(h.forward.is_empty());
        assert_eq!(h.entries[1].parent, Some(HistoryEntryId(0)));

        // Back: cursor moves to /a immediately, /a is refetched, /b becomes the forward entry.
        let served_before = served.lock().len();
        tab.go_back().await.expect("go back");
        let h = next_history(&mut event_rx, |h| h.current == Some(HistoryEntryId(0))).await;
        assert!(!h.can_go_back);
        assert_eq!(h.forward.len(), 1);
        assert_eq!(h.forward[0].id, HistoryEntryId(1));
        assert_eq!(h.forward[0].url.as_str(), b);
        // The traversal commits (Finished + another HistoryChanged) and still has 2 entries:
        // a traversal must not push.
        let h = next_history(&mut event_rx, |h| h.current == Some(HistoryEntryId(0))).await;
        assert_eq!(h.entries.len(), 2, "back must not create a new entry");
        for _ in 0..100 {
            if served.lock().len() > served_before {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert_eq!(
            served.lock().last().map(String::as_str),
            Some("/a"),
            "back must refetch /a"
        );

        // Forward returns to /b, again without pushing.
        tab.go_forward().await.expect("go forward");
        let h = next_history(&mut event_rx, |h| h.current == Some(HistoryEntryId(1))).await;
        assert!(h.can_go_back);
        assert!(h.forward.is_empty());
        assert_eq!(h.entries.len(), 2);
        // A traversal announces the cursor move first and again when the load commits; wait
        // for the commit so the document is loaded before navigating within it.
        let _ = next_history(&mut event_rx, |h| h.current == Some(HistoryEntryId(1))).await;

        // Fragment navigation within /b: a history entry, but no fetch.
        let served_before = served.lock().len();
        let b_frag = format!("{b}#section");
        tab.navigate(b_frag.clone()).await.expect("navigate fragment");
        let h = next_history(&mut event_rx, |h| h.entries.len() == 3).await;
        assert_eq!(h.current, Some(HistoryEntryId(2)));
        assert_eq!(h.entries[2].url.as_str(), b_frag);
        assert_eq!(h.entries[2].parent, Some(HistoryEntryId(1)));
        // Back to /b (same document): cursor moves, still no fetch.
        tab.go_back().await.expect("go back from fragment");
        let h = next_history(&mut event_rx, |h| h.current == Some(HistoryEntryId(1))).await;
        assert_eq!(h.forward[0].id, HistoryEntryId(2));
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(
            served.lock().len(),
            served_before,
            "fragment navigation and same-document back must not refetch"
        );

        // Internal pages: served by the engine's registry (no fetch), real history entries,
        // titles from the page, `about:` alias, and embedder overrides.
        engine.internal_pages().register_html(
            "mine",
            "<html><head><title>Mine</title></head><body>custom</body></html>",
        );
        let served_before = served.lock().len();
        tab.navigate("gosub://version").await.expect("navigate internal");
        let h = next_history(&mut event_rx, |h| {
            h.entries.last().is_some_and(|e| e.url.as_str() == "gosub://version")
        })
        .await;
        assert_eq!(h.entries.last().unwrap().title.as_deref(), Some("Version"));
        tab.navigate("about:blank").await.expect("navigate about");
        let _ = next_history(&mut event_rx, |h| {
            h.entries.last().is_some_and(|e| e.url.as_str() == "about:blank")
        })
        .await;
        tab.navigate("gosub://mine").await.expect("navigate override");
        let h = next_history(&mut event_rx, |h| {
            h.entries.last().is_some_and(|e| e.url.as_str() == "gosub://mine")
        })
        .await;
        assert_eq!(h.entries.last().unwrap().title.as_deref(), Some("Mine"));
        // Back over internal pages traverses (no new entries) and still fetches nothing.
        let n = h.entries.len();
        tab.go_back().await.expect("back");
        let h = next_history(&mut event_rx, |h| {
            h.entries.last().is_some_and(|e| e.title.as_deref() == Some("Mine")) && h.forward.len() == 1
        })
        .await;
        assert_eq!(h.entries.len(), n, "back over internal pages must not push");
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(
            served.lock().len(),
            served_before,
            "internal pages must never hit the network"
        );

        engine.close_zone(zone).await;
        engine.shutdown().await.expect("shutdown");
    }

    #[tokio::test]
    async fn close_zone_frees_slot_and_releases_cookies() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cookies.json");
        let store: CookieStoreHandle = crate::cookies::JsonCookieStore::new(path.clone()).unwrap().into();

        let mut engine = engine_with_max_zones(1);
        let mut event_rx = engine.subscribe_events();
        let _join = tokio::spawn(engine.start().expect("start"));

        let mut zone_services = services();
        zone_services.cookie_store = Some(store.clone());
        let mut zone = engine.create_zone(None, zone_services, None).expect("zone");
        let zone_id = zone.id;
        let _tab = zone.create_tab(Default::default(), None).await.expect("tab");

        // Store a cookie through the zone's persistent jar.
        let jar = store.jar_for(zone_id).expect("persistent jar");
        let url = url::Url::parse("https://example.com/").unwrap();
        let mut headers = http::HeaderMap::new();
        headers.append(http::header::SET_COOKIE, "sid=closed42; Path=/".parse().unwrap());
        jar.write().store_response_cookies(&url, &headers, None);

        // The single max_zones slot is taken.
        assert!(matches!(
            engine.create_zone(None, services(), None),
            Err(EngineError::ZoneLimitExceeded)
        ));

        engine.close_zone(zone).await;

        // ZoneClosed must have been emitted.
        let mut saw_closed = false;
        while let Ok(ev) = event_rx.try_recv() {
            if matches!(ev, EngineEvent::ZoneClosed { zone_id: z } if z == zone_id) {
                saw_closed = true;
            }
        }
        assert!(saw_closed, "expected a ZoneClosed event");

        // The slot is free again.
        let zone2 = engine
            .create_zone(None, services(), None)
            .expect("slot freed after close");

        // The closed zone's cookies survived on disk (release, not remove).
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(
            contents.contains("closed42"),
            "cookies must survive zone close, got: {contents}"
        );

        engine.close_zone(zone2).await;
        engine.shutdown().await.expect("shutdown");
    }

    #[tokio::test]
    async fn create_zone_enforces_max_zones() {
        let mut engine = engine_with_max_zones(1);
        // Keep a receiver alive: create_zone emits ZoneCreated on the broadcast bus.
        let _event_rx = engine.subscribe_events();
        // Zones need the I/O runtime.
        let _join = tokio::spawn(engine.start().expect("start"));

        // `None` config also exercises the default_zone_config fallback.
        engine.create_zone(None, services(), None).expect("first zone fits");

        let err = engine.create_zone(None, services(), None).unwrap_err();
        assert!(matches!(err, EngineError::ZoneLimitExceeded));

        engine.shutdown().await.expect("shutdown");
    }
}
