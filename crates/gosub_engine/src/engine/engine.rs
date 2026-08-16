//! [`GosubEngine`]: the entry point that owns the zones, the I/O thread, and the
//! [`EngineCommand`]/[`EngineEvent`] bus.

use crate::cookies::CookieStoreHandle;
use crate::engine::events::{EngineCommand, EngineEvent};
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
}

impl Default for EngineContext {
    fn default() -> Self {
        Self {
            event_tx: broadcast::channel::<EngineEvent>(DEFAULT_CHANNEL_CAPACITY).0,
            config: Arc::new(EngineConfig::default()),
            config_store: crate::engine::settings_store::default_config(),
            io_tx: OnceLock::new(),
            request_reference_map: Arc::new(RwLock::new(RequestReferenceMap::new())),
            tab_identities: Arc::new(TabIdentityRegistry::new()),
            #[cfg(all(feature = "process-isolation", target_os = "linux"))]
            renderer_process: OnceLock::new(),
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
                tab_identities: Arc::new(TabIdentityRegistry::new()),
                #[cfg(all(feature = "process-isolation", target_os = "linux"))]
                renderer_process: OnceLock::new(),
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

        // Start I/O thread, building the fetcher config from the settings store.
        let io_cfg = fetcher_config_from(&self.context.config_store);
        let io_handle = spawn_io_thread(io_cfg, self.context.clone());
        // Set once; `start()` already refuses to run twice, so this never races or overwrites.
        let _ = self.context.io_tx.set(io_handle.subscribe());
        self.io_handle = Some(io_handle);

        // Start metrics HTTP server (GET http://127.0.0.1:9090/metrics)
        #[cfg(feature = "metrics")]
        crate::metrics::start(9090);

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
                        "renderer isolation active in exec-per-render mode                          (the configured font system reads font files while operating)"
                    );
                    return;
                }
                Confinement::Unsupported(reason) => {
                    log::warn!(
                        "security.renderer_process is on, but the configured font system cannot run                          isolated ({reason}); rendering stays in-process"
                    );
                    return;
                }
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
                        let _ = self.context.renderer_process.set(Arc::new(Mutex::new(server)));
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
        if let Some(server) = self.context.renderer_process.get() {
            log::trace!("signal: shutting down the renderer fork server");
            server.lock().shutdown();
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
