//! [`GosubEngine`]: the entry point that owns the zones, the I/O thread, and the
//! [`EngineCommand`]/[`EngineEvent`] bus.

use crate::cookies::CookieStoreHandle;
use crate::engine::events::{EngineCommand, EngineEvent, ResourceUpdate};
use crate::engine::internal_pages::InternalPages;
use crate::engine::types::{EventChannel, IoChannel, ResourceChannel};
use crate::engine::{DEFAULT_CHANNEL_CAPACITY, RESOURCE_CHANNEL_CAPACITY};
use crate::html::RenderConfiguration;
use crate::net::req_ref_tracker::RequestReferenceMap;
use crate::net::tab_identity::TabIdentityRegistry;
use crate::net::{spawn_io_thread, IoHandle};
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

/// The one switch over the five process settings; see
/// [`GosubEngine::set_process_isolation`].
pub const PROCESS_ISOLATION_SWITCH: &str = "security.process_isolation";

/// The run-only switch's three states; see `GosubEngine::process_isolation_this_run`.
const RUN_SWITCH_UNSET: u8 = 0;
const RUN_SWITCH_OFF: u8 = 1;
const RUN_SWITCH_ON: u8 = 2;

/// The settings that each run one of the engine's components in a sandboxed
/// process of its own.
pub const PROCESS_SETTINGS: [&str; 5] = [
    "security.network_process",
    "security.image_decoder_process",
    "security.renderer_process",
    "security.cookie_vault",
    "security.storage_service",
];

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
    /// The storage service each zone's localStorage was routed through: the
    /// one reference `close_zone` gives back. A zone that stayed in-process
    /// has none, whatever directory its store names.
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    local_storage_routes: HashMap<ZoneId, std::path::PathBuf>,
    /// `security.process_isolation` chosen for this run only
    /// ([`Self::set_process_isolation_for_this_run`]): a command-line flag's
    /// choice, which must not become the persisted setting. Outranks the
    /// stored value while set. Three states (unset, off, on) in an atomic,
    /// so the engine stays `Sync` for embedders that share it across threads.
    process_isolation_this_run: std::sync::atomic::AtomicU8,
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
    /// Event sender for the low-volume control bus.
    pub event_tx: EventChannel,
    /// Event sender for the high-volume resource stream.
    pub resource_tx: ResourceChannel,
    /// Global engine configuration
    pub config: Arc<EngineConfig>,
    /// Per-engine settings store (key/value config with persistence and change subscriptions).
    /// A clone of this handle is threaded down to each zone and tab.
    pub config_store: Config,
    /// I/O submission channel, installed once when the engine starts (`start()`), read by each
    /// zone at creation. A `OnceLock` rather than `Arc<RwLock<Option<..>>>`: it is set exactly once
    /// and never swapped, and `EngineContext` is already shared behind an `Arc`, so no inner lock
    /// or `Arc` is needed. Reading before `start()` yields `None` (`EngineError::IoNotStarted`).
    pub(crate) io_tx: OnceLock<IoChannel>,
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
    /// The cookie vault, when `security.cookie_vault` started one: zones the
    /// engine provisions jars for keep them there.
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    pub cookie_vault: OnceLock<Arc<crate::cookie_vault::client::CookieVault>>,
    /// The network process's end of its line to the vault, waiting for the I/O
    /// thread to spawn that process.
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    pub net_vault_link: Arc<Mutex<Option<crate::cookie_vault::client::NetVaultLink>>>,
    /// Storage service processes, one per directory, shared by the zones
    /// whose local store lives there.
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    pub storage_services: Arc<Mutex<StorageServices>>,
}

/// Storage service per directory, with how many open zones use it.
#[cfg(all(feature = "process-isolation", target_os = "linux"))]
pub type StorageServices =
    std::collections::HashMap<std::path::PathBuf, (Arc<crate::storage_service::client::ServiceLocalStore>, usize)>;

impl Default for EngineContext {
    fn default() -> Self {
        Self {
            event_tx: broadcast::channel::<EngineEvent>(DEFAULT_CHANNEL_CAPACITY).0,
            resource_tx: broadcast::channel::<ResourceUpdate>(RESOURCE_CHANNEL_CAPACITY).0,
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
            #[cfg(all(feature = "process-isolation", target_os = "linux"))]
            cookie_vault: OnceLock::new(),
            #[cfg(all(feature = "process-isolation", target_os = "linux"))]
            net_vault_link: Arc::new(Mutex::new(None)),
            #[cfg(all(feature = "process-isolation", target_os = "linux"))]
            storage_services: Arc::new(Mutex::new(std::collections::HashMap::new())),
        }
    }
}

/// The storage-service registry key for a directory: one process per directory,
/// so two spellings of the same path must land on one entry.
#[cfg(all(feature = "process-isolation", target_os = "linux"))]
fn storage_service_key(dir: std::path::PathBuf) -> std::path::PathBuf {
    std::fs::canonicalize(&dir).unwrap_or(dir)
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
        // Timed stages go on the telemetry bus from here on, for an embedder that shows
        // what the engine is doing live. Free until someone subscribes.
        crate::telemetry::forward_timing_stages();
        let resolved_config = config.unwrap_or_default();

        // Command channel on which to send and receive engine commands from the UA.
        let (cmd_tx, cmd_rx) = mpsc::channel::<EngineCommand>(DEFAULT_CHANNEL_CAPACITY);

        // Separate buses so per-chunk resource progress cannot evict control events.
        let (event_tx, _first_rx) = broadcast::channel::<EngineEvent>(DEFAULT_CHANNEL_CAPACITY);
        let (resource_tx, _first_res_rx) = broadcast::channel::<ResourceUpdate>(RESOURCE_CHANNEL_CAPACITY);

        Self {
            context: Arc::new(EngineContext {
                event_tx: event_tx.clone(),
                resource_tx,
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
                #[cfg(all(feature = "process-isolation", target_os = "linux"))]
                cookie_vault: OnceLock::new(),
                #[cfg(all(feature = "process-isolation", target_os = "linux"))]
                net_vault_link: Arc::new(Mutex::new(None)),
                #[cfg(all(feature = "process-isolation", target_os = "linux"))]
                storage_services: Arc::new(Mutex::new(std::collections::HashMap::new())),
            }),
            render_backend: backend,
            compositor,
            font_system: Arc::new(Mutex::new(C::FontSystem::default())),
            zones: HashMap::new(),
            cookie_stores: HashMap::new(),
            #[cfg(all(feature = "process-isolation", target_os = "linux"))]
            local_storage_routes: HashMap::new(),
            process_isolation_this_run: std::sync::atomic::AtomicU8::new(RUN_SWITCH_UNSET),
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

        // Which `security.*` process settings survive into this run. Decided
        // before the I/O thread, which is what spawns the network process.
        self.resolve_isolation_settings();

        // The vault before the I/O thread: the network process, spawned there,
        // inherits its line to the vault.
        #[cfg(all(feature = "process-isolation", target_os = "linux"))]
        self.start_cookie_vault();

        // Start the I/O thread. The fetcher config is read from the settings store
        // when a zone first fetches, not here, so `net.*` set after `start()` applies.
        let io_handle = spawn_io_thread(self.context.clone());
        // Set once; `start()` already refuses to run twice, so this never races or overwrites.
        let _ = self.context.io_tx.set(io_handle.subscribe());
        self.io_handle = Some(io_handle);

        // Start metrics HTTP server (GET http://127.0.0.1:9090/metrics)
        #[cfg(feature = "metrics")]
        if self.context.config_store.get_bool("telemetry.metrics_enabled") {
            crate::metrics::start(9090, Arc::clone(&self.context));
        }

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

    /// Whether `key` still holds its schema default, i.e. the embedder never
    /// chose it. A default that cannot apply here is dropped quietly; an
    /// explicit choice that cannot apply gets a warning.
    #[cfg_attr(not(feature = "process-isolation"), allow(dead_code))]
    fn setting_at_default(&self, key: &str) -> bool {
        // Not "equals the default": an embedder that sets the default value
        // explicitly (turning the network process on where it is off by
        // default) has still made a choice. The one switch, set, is that
        // choice for all five.
        let store = &self.context.config_store;
        !store.is_overridden(key) && self.process_isolation_switch().is_none()
    }

    /// The switch as chosen: for this run, else as stored; `None` when unset.
    fn process_isolation_switch(&self) -> Option<bool> {
        let store = &self.context.config_store;
        match self
            .process_isolation_this_run
            .load(std::sync::atomic::Ordering::Acquire)
        {
            RUN_SWITCH_OFF => Some(false),
            RUN_SWITCH_ON => Some(true),
            _ => store
                .is_overridden(PROCESS_ISOLATION_SWITCH)
                .then(|| store.get_bool(PROCESS_ISOLATION_SWITCH)),
        }
    }

    /// `security.process_isolation`, when the embedder set it, decides the five
    /// process settings for this run: `false` is the single-process engine,
    /// `true` asks for every component process. The five keep their own
    /// values only while the switch is unset. Transient, like every value the
    /// engine resolves itself: the user's persisted per-component choices stay
    /// what they were for a run with the switch unset.
    fn apply_process_isolation_switch(&self) {
        let store = &self.context.config_store;
        let Some(on) = self.process_isolation_switch() else {
            return;
        };
        for key in PROCESS_SETTINGS {
            let _ = store.set_transient(key, gosub_config::settings::Setting::Bool(on));
        }
        log::info!(
            "{PROCESS_ISOLATION_SWITCH} is {}: {}",
            if on { "on" } else { "off" },
            if on {
                "every component process requested"
            } else {
                "single process, no component processes"
            }
        );
    }

    /// For this run only: what this process cannot do must not be written back as
    /// the embedder's persisted choice, or a run that can would inherit the `false`.
    fn turn_off(&self, key: &str) {
        let _ = self
            .context
            .config_store
            .set_transient(key, gosub_config::settings::Setting::Bool(false));
    }

    /// The `security.*` process settings default to on; here the defaults meet
    /// this process, platform and configuration. Without the embedder's
    /// `child_process::dispatch()` nothing may spawn (a child is this binary
    /// re-exec'd, and would run the embedder's own `main()` - for a GUI
    /// embedder, a phantom window per spawn); the network and decoder
    /// processes are on by default on Linux only, until the macOS and Windows
    /// backends have run in CI; the renderer tier has conditions of its own,
    /// checked in `start_renderer_process`.
    #[allow(clippy::needless_return)] // the cfg arms need explicit returns
    fn resolve_isolation_settings(&self) {
        self.apply_process_isolation_switch();

        #[cfg(not(feature = "process-isolation"))]
        {
            for key in PROCESS_SETTINGS {
                self.turn_off(key);
            }
        }

        #[cfg(feature = "process-isolation")]
        {
            let store = &self.context.config_store;
            if !crate::child_process::was_dispatched() {
                let requested: Vec<&str> = PROCESS_SETTINGS.into_iter().filter(|key| store.get_bool(key)).collect();
                if requested.is_empty() {
                    return;
                }
                if requested.iter().any(|key| !self.setting_at_default(key)) {
                    log::warn!(
                        "{} requested, but gosub_engine::child_process::dispatch() was not called at the \
                         top of main(); running without process isolation",
                        requested.join(", ")
                    );
                } else {
                    log::info!(
                        "process isolation is off: this embedder does not call \
                         gosub_engine::child_process::dispatch() at the top of main()"
                    );
                }
                for key in requested {
                    self.turn_off(key);
                }
                return;
            }

            if !gosub_sandbox::CONFINES_CHILDREN {
                self.refuse_unconfined_children();
                return;
            }

            if !cfg!(target_os = "linux") {
                for key in PROCESS_SETTINGS {
                    if store.get_bool(key) && self.setting_at_default(key) {
                        self.turn_off(key);
                    }
                }
                if PROCESS_SETTINGS.iter().any(|key| store.get_bool(key)) {
                    log::info!(
                        "process isolation was requested explicitly on a platform where it is not on by default"
                    );
                }
            }
        }
    }

    /// A platform with no sandbox backend (Android, the BSDs) starts no component
    /// process, asked for or not: unconfined, a child parsing page content holds
    /// every right this process has, which is worse than parsing it here.
    #[cfg(feature = "process-isolation")]
    fn refuse_unconfined_children(&self) {
        let store = &self.context.config_store;
        let requested: Vec<&str> = PROCESS_SETTINGS.into_iter().filter(|key| store.get_bool(key)).collect();
        if requested.iter().any(|key| !self.setting_at_default(key)) {
            log::warn!(
                "{} requested, but this platform has no sandbox for component processes and they \
                 would run unconfined; running without process isolation",
                requested.join(", ")
            );
        }
        for key in requested {
            self.turn_off(key);
        }
    }

    /// Spawn the cookie vault when `security.cookie_vault` asks for it. With
    /// the network process also on, the two get a direct line so cookie values
    /// on requests bypass this process entirely.
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    fn start_cookie_vault(&self) {
        use crate::cookie_vault::client::CookieVault;
        let store = &self.context.config_store;
        if !store.get_bool("security.cookie_vault") {
            return;
        }
        let with_net = store.get_bool("security.network_process");
        match CookieVault::spawn(with_net) {
            Ok((vault, net_link)) => {
                log::info!(
                    "cookie jars live in a separate, sandboxed vault process{}",
                    if with_net {
                        " (the network process talks to it directly)"
                    } else {
                        ""
                    }
                );
                *self.context.net_vault_link.lock() = net_link;
                let _ = self.context.cookie_vault.set(Arc::new(vault));
            }
            Err(e) => {
                log::error!("security.cookie_vault is on but the vault could not start ({e}); cookies stay in-process");
                self.turn_off("security.cookie_vault");
            }
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

        let by_default = self.setting_at_default("security.renderer_process");

        // The configured font system's (static) tier decides the mechanism:
        // only `Full` systems benefit from a warmed fork server.
        // `FontPathsReadable` renders in throwaway exec'd processes spawned
        // per render (see `render_process`) - nothing to start here, and not
        // by default either: that tier has no resident renderers (every scroll
        // and hover is a full render in a fresh process), so an embedder opts
        // into it knowingly.
        let exec_per_render = {
            use gosub_interface::font_system::{Confinement, FontSystem as _};
            match C::FontSystem::confinement() {
                Confinement::Full => false,
                Confinement::FontPathsReadable if by_default => {
                    log::info!(
                        "renderer isolation is off by default for this font system (it reads font \
                         files while operating, so renderers would be exec'd per render); set \
                         security.renderer_process explicitly to opt in"
                    );
                    self.turn_off("security.renderer_process");
                    return;
                }
                // Explicitly asked for: exec-per-render, once the rasterizer check
                // below has passed - that mode ships pixels as much as the fork
                // server's does.
                Confinement::FontPathsReadable => true,
                Confinement::Unsupported(reason) => {
                    if by_default {
                        log::info!(
                            "renderer isolation is off: the configured font system cannot run isolated ({reason})"
                        );
                    } else {
                        log::warn!(
                            "security.renderer_process is on, but the configured font system cannot run \
                             isolated ({reason}); rendering stays in-process"
                        );
                    }
                    self.turn_off("security.renderer_process");
                    return;
                }
            }
        };

        // A renderer that cannot rasterize would ship geometry and no pixels:
        // blank tabs with no way back. Better to say so and stay in-process.
        {
            let fonts: Arc<Mutex<dyn gosub_interface::font_system::FontSystem>> =
                Arc::new(Mutex::new(C::FontSystem::default()));
            if C::forked_tile_rasterizer(fonts).is_none() {
                if by_default {
                    log::info!(
                        "renderer isolation is off: this RenderConfiguration provides no \
                         forked_tile_rasterizer (enable the engine's `cairo-tiles`/`skia-tiles` feature)"
                    );
                } else {
                    log::warn!(
                        "security.renderer_process is on, but this RenderConfiguration provides no \
                         forked_tile_rasterizer (enable the engine's `cairo-tiles`/`skia-tiles` feature, \
                         or implement it); rendering stays in-process"
                    );
                }
                self.turn_off("security.renderer_process");
                return;
            }
        }

        if exec_per_render {
            log::info!(
                "renderer isolation active in exec-per-render mode \
                 (the configured font system reads font files while operating)"
            );
            return;
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
                        self.turn_off("security.renderer_process");
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

    /// The escape audit in the network process; `None` when networking is
    /// in-process (tools and tests).
    #[cfg(feature = "process-isolation")]
    pub async fn audit_net_process(&self) -> Option<gosub_sandbox::audit::AuditReport> {
        self.io_handle.as_ref()?.audit_net().await
    }

    /// The network process's pid; `None` when networking is in-process
    /// (tools and tests).
    #[cfg(feature = "process-isolation")]
    pub async fn net_process_pid(&self) -> Option<u32> {
        self.io_handle.as_ref()?.net_pid().await
    }

    /// The cookie vault, when one runs (tools and tests).
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    pub fn cookie_vault(&self) -> Option<&Arc<crate::cookie_vault::client::CookieVault>> {
        self.context.cookie_vault.get()
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
    ///
    /// The control bus: navigation, tab and zone lifecycle, downloads, input feedback,
    /// crashes. Per-resource detail is on its own stream - see
    /// [`subscribe_resource_events`](Self::subscribe_resource_events).
    ///
    /// Bounded: a receiver that falls behind gets
    /// [`RecvError::Lagged`](broadcast::error::RecvError::Lagged) and loses the oldest
    /// events, which can include [`TabCrashed`](EngineEvent::TabCrashed). Keep the receive
    /// loop short.
    pub fn subscribe_events(&self) -> broadcast::Receiver<EngineEvent> {
        self.context.event_tx.subscribe()
    }

    /// Return a receiver for the per-resource stream: one [`ResourceUpdate`] per fetch
    /// lifecycle step, including a `Progress` per chunk of every subresource.
    ///
    /// Separate from [`subscribe_events`](Self::subscribe_events) because these arrive at
    /// network rate and would otherwise evict control events from a bounded buffer. Opt-in:
    /// a shell that shows no per-resource detail never subscribes.
    pub fn subscribe_resource_events(&self) -> broadcast::Receiver<ResourceUpdate> {
        self.context.resource_tx.subscribe()
    }

    /// The `gosub://` page registry. Register a provider to add an internal page or override
    /// a built-in one (e.g. a branded `home`); see [`InternalPages`]. Registration works at
    /// any time - pages are resolved when navigated to.
    pub fn internal_pages(&self) -> &InternalPages {
        &self.context.internal_pages
    }

    /// The engine's settings store, for reading or overriding settings (e.g.
    /// `net.user_agent`). Usable before or after [`start`](Self::start). `net.*` settings
    /// are read when a zone makes its first request, so an override applies to zones that
    /// start fetching after it; zones already fetching keep the values they were built with.
    pub fn settings(&self) -> &Config {
        &self.context.config_store
    }

    /// Run the engine's components in separate sandboxed processes (`true`),
    /// or everything in this process as the engine always has (`false`). The
    /// same as setting `security.process_isolation`: it decides all five
    /// `security.*` process settings at [`start`](Self::start). **Persisted**
    /// like any setting: with a persistent settings adapter it holds for
    /// later runs too, until the key is removed - the user's preference. A
    /// command-line flag is a choice for this run and belongs to
    /// [`Self::set_process_isolation_for_this_run`]. The per-component
    /// settings are for finer choices while this one is unset. Needs
    /// `child_process::dispatch_with()` first in `main()` to take effect, like
    /// the settings it governs.
    pub fn set_process_isolation(&self, enabled: bool) -> Result<(), EngineError> {
        self.context
            .config_store
            .set(PROCESS_ISOLATION_SWITCH, gosub_config::settings::Setting::Bool(enabled))
            .map_err(|e| EngineError::InvalidConfiguration(e.to_string()))
    }

    /// [`Self::set_process_isolation`] for this run only: nothing is written
    /// to the settings storage, and the choice outranks whatever is stored.
    /// What an embedder's `--single-process` or `--isolated` flag maps to.
    pub fn set_process_isolation_for_this_run(&self, enabled: bool) -> Result<(), EngineError> {
        self.context
            .config_store
            .set_transient(PROCESS_ISOLATION_SWITCH, gosub_config::settings::Setting::Bool(enabled))
            .map_err(|e| EngineError::InvalidConfiguration(e.to_string()))?;
        self.process_isolation_this_run.store(
            if enabled { RUN_SWITCH_ON } else { RUN_SWITCH_OFF },
            std::sync::atomic::Ordering::Release,
        );
        Ok(())
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
            for (store, _) in self.context.storage_services.lock().values() {
                store.shutdown();
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

        // The vault after I/O, so a response still in flight stores its
        // cookies first; its shutdown waits for the snapshots it sent.
        #[cfg(all(feature = "process-isolation", target_os = "linux"))]
        if let Some(vault) = self.context.cookie_vault.get() {
            log::trace!("signal: shutting down the cookie vault");
            vault.shutdown();
        }

        // Persist cookie stores once nothing can change them any more.
        self.flush_persistence();

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
    /// Start building a zone. Everything is optional: the default is an ephemeral profile
    /// with the engine's default [`ZoneConfig`] and a fresh id.
    ///
    /// ```rust,no_run
    /// # use gosub_engine::GosubEngine;
    /// # fn f(engine: &mut GosubEngine) -> Result<(), gosub_engine::EngineError> {
    /// let zone = engine.zone_builder().create()?;
    /// # Ok(()) }
    /// ```
    pub fn zone_builder(&mut self) -> ZoneBuilder<'_, C> {
        ZoneBuilder {
            engine: self,
            config: None,
            id: None,
            services: ZoneServices::default(),
        }
    }

    /// `None` for `config` uses the engine's [`EngineConfig::default_zone_config`];
    /// `None` for `zone_id` generates a fresh id. Fails with
    /// [`EngineError::ZoneLimitExceeded`] once the engine holds
    /// [`EngineConfig::max_zones`] zones. The returned handle carries the [`ZoneId`]
    /// and a clone of the engine's command sender, so the caller can send zone
    /// commands without holding a reference to the engine.
    pub(crate) fn create_zone(
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

        #[cfg(all(feature = "process-isolation", target_os = "linux"))]
        let routed = self.route_local_storage(&services);

        // A zone whose jar the engine provisions keeps it in the vault, behind a
        // jar handle that forwards; an embedder-supplied jar is the embedder's.
        #[cfg(all(feature = "process-isolation", target_os = "linux"))]
        let services = match self.context.cookie_vault.get() {
            Some(vault) if services.cookie_jar.is_none() => {
                let vault = Arc::clone(vault);
                let id = zone_id.unwrap_or_default();
                vault.open_zone(id, cookie_store.clone());
                let jar = crate::cookie_vault::client::VaultCookieJar::new(Arc::clone(&vault), id).handle();
                let created = self.create_zone_with_services(
                    config,
                    ZoneServices {
                        cookie_jar: Some(jar),
                        ..services
                    },
                    Some(id),
                    cookie_store,
                );
                // No zone came of it: the vault must not keep (or respawn
                // with) a jar nothing will ever close, nor the storage
                // service a reference nothing will give back.
                if created.is_err() {
                    vault.close_zone(id);
                }
                self.settle_local_storage(&created, routed);
                return created;
            }
            _ => services,
        };
        let created = self.create_zone_with_services(config, services, zone_id, cookie_store);
        #[cfg(all(feature = "process-isolation", target_os = "linux"))]
        self.settle_local_storage(&created, routed);
        created
    }

    /// Record which storage service a new zone holds a reference to, for
    /// `close_zone`. A zone that never came to exist is never closed, so its
    /// reference is given back here instead.
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    fn settle_local_storage(&mut self, created: &Result<Zone<C>, EngineError>, routed: Option<std::path::PathBuf>) {
        match (created, routed) {
            (Ok(zone), Some(dir)) => {
                self.local_storage_routes.insert(zone.id, dir);
            }
            (Err(_), routed) => self.release_local_storage(routed),
            (Ok(_), None) => {}
        }
    }

    /// Route a zone's local storage through the storage service process when
    /// its store can be served from a directory. One process per directory.
    /// Returns the registry key it counted the zone under, for
    /// [`Self::release_local_storage`].
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    fn route_local_storage(&self, services: &ZoneServices) -> Option<std::path::PathBuf> {
        use crate::storage_service::client::ServiceLocalStore;
        if !self.context.config_store.get_bool("security.storage_service") {
            return None;
        }
        let dir = storage_service_key(services.storage.local_store().service_directory()?);
        let mut processes = self.context.storage_services.lock();
        let store = match processes.get_mut(&dir) {
            Some((store, zones)) => {
                *zones += 1;
                Arc::clone(store)
            }
            None => match ServiceLocalStore::new(&dir) {
                Ok(store) => {
                    let store = Arc::new(store);
                    processes.insert(dir.clone(), (Arc::clone(&store), 1));
                    store
                }
                Err(e) => {
                    log::warn!("localStorage stays in-process for {}: {e}", dir.display());
                    return None;
                }
            },
        };
        drop(processes);
        if !services.storage.route_local_through(store) {
            log::warn!(
                "localStorage stays in-process for {}: this zone's storage already handed out an area, \
                 and a second store over the same files would lose writes",
                dir.display()
            );
            self.release_local_storage(Some(dir));
            return None;
        }
        Some(dir)
    }

    /// Give back one zone's reference to a storage service process, ending the
    /// process with its last zone.
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    fn release_local_storage(&self, dir: Option<std::path::PathBuf>) {
        let Some(dir) = dir else {
            return;
        };
        let mut processes = self.context.storage_services.lock();
        let last = match processes.get_mut(&dir) {
            Some((_, zones)) => {
                *zones = zones.saturating_sub(1);
                *zones == 0
            }
            None => false,
        };
        if last {
            if let Some((store, _)) = processes.remove(&dir) {
                store.shutdown();
            }
        }
    }

    fn create_zone_with_services(
        &mut self,
        config: ZoneConfig,
        services: ZoneServices,
        zone_id: Option<ZoneId>,
        cookie_store: Option<crate::cookies::CookieStoreHandle>,
    ) -> Result<Zone<C>, EngineError> {
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

        // Nobody listening is not a reason to fail: the zone exists and is
        // registered, and an error here would hand back no zone to close it with.
        if self
            .context
            .event_tx
            .send(EngineEvent::ZoneCreated { zone_id })
            .is_err()
        {
            log::debug!("zone {zone_id} created with no event subscriber");
        }

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

        // The vault drops the zone's jar once its last snapshot is with the store.
        #[cfg(all(feature = "process-isolation", target_os = "linux"))]
        if let Some(vault) = self.context.cookie_vault.get() {
            vault.close_zone(zone_id);
        }

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

        // The storage service outlives its last zone by nothing.
        #[cfg(all(feature = "process-isolation", target_os = "linux"))]
        {
            let routed = self.local_storage_routes.remove(&zone_id);
            self.release_local_storage(routed);
        }

        self.zones.remove(&zone_id);

        let _ = self.context.event_tx.send(EngineEvent::ZoneClosed { zone_id });
    }
}

/// Builds a [`Zone`]; obtained from [`GosubEngine::zone_builder`].
///
/// Starts from [`ZoneServices::default`] (ephemeral, in-memory). Either replace the whole
/// services struct with [`services`](Self::services), or adjust one piece at a time.
pub struct ZoneBuilder<'a, C: RenderConfiguration> {
    engine: &'a mut GosubEngine<C>,
    config: Option<ZoneConfig>,
    id: Option<ZoneId>,
    services: ZoneServices,
}

impl<C: RenderConfiguration> ZoneBuilder<'_, C> {
    /// Limits and settings for the zone. Defaults to the engine's `default_zone_config`.
    pub fn config(mut self, config: ZoneConfig) -> Self {
        self.config = Some(config);
        self
    }

    /// A fixed id, to restore a persisted profile across runs. Defaults to a fresh UUID.
    pub fn id(mut self, id: ZoneId) -> Self {
        self.id = Some(id);
        self
    }

    /// Replace the whole services struct.
    pub fn services(mut self, services: ZoneServices) -> Self {
        self.services = services;
        self
    }

    /// Local and session storage.
    pub fn storage(mut self, storage: Arc<crate::storage::StorageService>) -> Self {
        self.services.storage = storage;
        self
    }

    /// The cookie jar. `None` means the zone sends and stores no cookies.
    pub fn cookie_jar(mut self, jar: Option<crate::cookies::CookieJarHandle>) -> Self {
        self.services.cookie_jar = jar;
        self
    }

    /// A persistent cookie store, for cookies that survive the process.
    pub fn cookie_store(mut self, store: Option<CookieStoreHandle>) -> Self {
        self.services.cookie_store = store;
        self
    }

    /// How storage is keyed (e.g. per top-level origin).
    pub fn partition_policy(mut self, policy: crate::storage::PartitionPolicy) -> Self {
        self.services.partition_policy = policy;
        self
    }

    /// Bookmarks and visited history. `None` records nothing, as a private profile would.
    pub fn places(mut self, places: Option<crate::places::PlacesHandle>) -> Self {
        self.services.places = places;
        self
    }

    /// Create the zone. Fails with [`EngineError::ZoneLimitExceeded`] past `max_zones`.
    pub fn create(self) -> Result<Zone<C>, EngineError> {
        self.engine.create_zone(self.config, self.services, self.id)
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

    /// A shared bus would drop the oldest event - possibly `TabCrashed` - once more than
    /// `DEFAULT_CHANNEL_CAPACITY` piled up behind a busy shell.
    #[tokio::test(flavor = "current_thread")]
    async fn resource_flood_does_not_evict_control_events() {
        use crate::engine::events::{ResourceEvent, ResourceUpdate};
        use crate::net::req_ref_tracker::RequestReference;

        let engine = engine_with_max_zones(1);
        let mut control = engine.subscribe_events();

        // A control event, then more resource traffic than the control bus could hold.
        let tab_id = crate::tab::TabId::new();
        let zone_id = ZoneId::new();
        let _ = engine.context.event_tx.send(EngineEvent::TabCrashed {
            tab_id,
            zone_id,
            error: "boom".into(),
        });
        for i in 0..(DEFAULT_CHANNEL_CAPACITY * 4) {
            let _ = engine.context.resource_tx.send(ResourceUpdate {
                tab_id,
                event: ResourceEvent::Progress {
                    request_id: crate::engine::types::RequestId::new(),
                    reference: RequestReference::Navigation(crate::engine::types::NavigationId::new()),
                    received_bytes: i as u64,
                    expected_length: None,
                    elapsed: Duration::from_millis(1),
                },
            });
        }

        match control.try_recv() {
            Ok(EngineEvent::TabCrashed { error, .. }) => assert_eq!(error, "boom"),
            other => panic!("control event was evicted by resource traffic: {other:?}"),
        }
    }

    /// A failing navigation must reach the embedder classified, not as a string it has to
    /// parse. Port 9 (discard) refuses connections, so this is a transport failure.
    #[tokio::test(flavor = "current_thread")]
    async fn failed_navigation_reports_a_typed_error() {
        use crate::events::NavigationEvent;
        use crate::LoadError;

        let mut engine = engine_with_max_zones(1);
        let mut events = engine.subscribe_events();
        let _join = tokio::spawn(engine.start().expect("start"));
        let mut zone = engine.zone_builder().services(services()).create().expect("zone");
        let tab = zone.tab_builder().create().await.expect("tab");

        tab.navigate("http://127.0.0.1:9/nope").await.expect("navigate");

        let error = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                match events.recv().await {
                    Ok(EngineEvent::Navigation {
                        event: NavigationEvent::Failed { error, .. },
                        ..
                    }) => return error,
                    Ok(_) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(e) => panic!("event stream closed: {e}"),
                }
            }
        })
        .await
        .expect("timed out waiting for NavigationEvent::Failed");

        assert!(
            matches!(error, LoadError::Connect { .. }),
            "a refused connection should classify as Connect, got {error:?}"
        );
        // And it still prints something a shell can show.
        assert!(!error.to_string().is_empty());

        engine.close_zone(zone).await;
        engine.shutdown().await.expect("shutdown");
    }

    /// An unparseable URL is a different kind of failure from a transport one.
    #[tokio::test(flavor = "current_thread")]
    async fn unparseable_url_reports_invalid_url() {
        use crate::events::NavigationEvent;
        use crate::LoadError;

        let mut engine = engine_with_max_zones(1);
        let mut events = engine.subscribe_events();
        let _join = tokio::spawn(engine.start().expect("start"));
        let mut zone = engine.zone_builder().services(services()).create().expect("zone");
        let tab = zone.tab_builder().create().await.expect("tab");

        tab.navigate("not a url at all").await.expect("navigate");

        let error = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                match events.recv().await {
                    Ok(EngineEvent::Navigation {
                        event: NavigationEvent::FailedUrl { error, .. },
                        ..
                    }) => return error,
                    Ok(_) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(e) => panic!("event stream closed: {e}"),
                }
            }
        })
        .await
        .expect("timed out waiting for NavigationEvent::FailedUrl");

        assert!(
            matches!(error, LoadError::InvalidUrl { .. }),
            "expected InvalidUrl, got {error:?}"
        );

        engine.close_zone(zone).await;
        engine.shutdown().await.expect("shutdown");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn zone_tab_looks_up_a_live_handle_by_id() {
        let mut engine = engine_with_max_zones(1);
        let _join = tokio::spawn(engine.start().expect("start"));
        let mut zone = engine.zone_builder().services(services()).create().expect("zone");
        let created = zone.tab_builder().create().await.expect("tab");

        let looked_up = zone.tab(created.tab_id).expect("tab exists");
        assert_eq!(looked_up.tab_id, created.tab_id);
        // Same sink, so state read through either handle agrees.
        looked_up.set_title("via lookup").await.expect("send");
        tokio::time::timeout(Duration::from_secs(5), async {
            while created.title() != "via lookup" {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("title should propagate");

        assert!(zone.tab(crate::tab::TabId::new()).is_none(), "unknown id yields None");
        assert!(zone.close_tab(created.tab_id).await);
        assert!(zone.tab(created.tab_id).is_none(), "closed tab is gone");

        engine.close_zone(zone).await;
        engine.shutdown().await.expect("shutdown");
    }

    /// A page served as an attachment is offered as a download; `RenderDownload` overrides
    /// that and loads the already-spooled body as the page. No second request is made.
    #[tokio::test(flavor = "current_thread")]
    async fn render_download_loads_a_misclassified_page() {
        use crate::events::NavigationEvent;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let hits_srv = hits.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let hits_srv = hits_srv.clone();
                tokio::spawn(async move {
                    hits_srv.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let mut buf = vec![0u8; 4096];
                    let _ = stream.read(&mut buf).await;
                    let body = "<html><head><title>Actually a page</title></head><body>hi</body></html>";
                    let head = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\n\
                         Content-Disposition: attachment; filename=\"page.bin\"\r\n\
                         Content-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(head.as_bytes()).await;
                    let _ = stream.write_all(body.as_bytes()).await;
                });
            }
        });

        let mut engine = engine_with_max_zones(1);
        let mut events = engine.subscribe_events();
        let _join = tokio::spawn(engine.start().expect("start"));
        let mut zone = engine.zone_builder().services(services()).create().expect("zone");
        let tab = zone.tab_builder().create().await.expect("tab");

        tab.navigate(format!("http://127.0.0.1:{port}/page"))
            .await
            .expect("navigate");
        let (offered_url, offer) = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                match events.recv().await {
                    Ok(EngineEvent::DownloadRequested { url, offer, .. }) => return (url, offer),
                    Ok(_) => continue,
                    Err(e) => panic!("event stream closed: {e}"),
                }
            }
        })
        .await
        .expect("timed out waiting for DownloadRequested");
        let served = hits.load(std::sync::atomic::Ordering::SeqCst);
        // The offer is readable off the handle - the recovery path for a lagged receiver.
        assert_eq!(
            tab.pending_downloads().iter().map(|p| p.offer).collect::<Vec<_>>(),
            vec![offer]
        );

        tab.render_download(offer).await.expect("render");
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                match events.recv().await {
                    Ok(EngineEvent::Navigation {
                        event: NavigationEvent::Finished { .. },
                        ..
                    }) => return,
                    Ok(EngineEvent::Navigation {
                        event: NavigationEvent::Failed { error, .. },
                        ..
                    }) => panic!("render failed: {error}"),
                    Ok(_) => continue,
                    Err(e) => panic!("event stream closed: {e}"),
                }
            }
        })
        .await
        .expect("timed out waiting for the rendered page");

        assert_eq!(tab.title(), "Actually a page");
        assert!(tab.pending_downloads().is_empty(), "rendering consumes the offer");
        assert_eq!(tab.url().map(|u| u.to_string()), Some(offered_url.to_string()));
        assert_eq!(
            hits.load(std::sync::atomic::Ordering::SeqCst),
            served,
            "must not re-fetch"
        );

        // The offer is consumed: a second override has nothing to render.
        tab.render_download(offer).await.expect("send");
        let error = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Ok(EngineEvent::Navigation {
                    event: NavigationEvent::Failed { error, .. },
                    ..
                }) = events.recv().await
                {
                    return error;
                }
            }
        })
        .await
        .expect("expected a Failed for a consumed offer");
        assert!(matches!(error, crate::LoadError::Content { .. }), "got {error:?}");

        engine.close_zone(zone).await;
        engine.shutdown().await.expect("shutdown");
    }

    /// `net.*` settings are read when a zone first fetches, so an override made after
    /// `start()` must reach the wire for a zone created afterwards.
    #[tokio::test(flavor = "current_thread")]
    async fn user_agent_set_after_start_is_used_by_new_zones() {
        use crate::events::NavigationEvent;
        use crate::Setting;
        use cow_utils::CowUtils;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let seen = std::sync::Arc::new(parking_lot::Mutex::new(String::new()));
        let seen_srv = seen.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let seen_srv = seen_srv.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 8192];
                    let n = stream.read(&mut buf).await.unwrap_or(0);
                    *seen_srv.lock() = String::from_utf8_lossy(&buf[..n]).into_owned();
                    let body = "<html><body>ok</body></html>";
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
        let mut events = engine.subscribe_events();
        let _join = tokio::spawn(engine.start().expect("start"));

        // After start, before any zone exists.
        engine
            .settings()
            .set("net.user_agent", Setting::String("GosubTest/9.0".into()))
            .expect("set user agent");

        let mut zone = engine.zone_builder().services(services()).create().expect("zone");
        let tab = zone.tab_builder().create().await.expect("tab");
        tab.navigate(format!("http://127.0.0.1:{port}/"))
            .await
            .expect("navigate");
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                match events.recv().await {
                    Ok(EngineEvent::Navigation {
                        event: NavigationEvent::Finished { .. } | NavigationEvent::Failed { .. },
                        ..
                    }) => return,
                    Ok(_) => continue,
                    Err(e) => panic!("event stream closed: {e}"),
                }
            }
        })
        .await
        .expect("timed out waiting for navigation");

        let request = seen.lock().clone();
        assert!(
            request.cow_to_ascii_lowercase().contains("user-agent: gosubtest/9.0"),
            "override did not reach the wire; request was:\n{request}"
        );

        engine.close_zone(zone).await;
        engine.shutdown().await.expect("shutdown");
    }

    /// The builder with nothing set must yield a working zone: an embedder's first program
    /// should not need to know what `ZoneServices` is.
    #[tokio::test(flavor = "current_thread")]
    async fn zone_builder_defaults_produce_a_usable_zone() {
        use crate::events::{NavigationEvent, TabCommand};

        let mut engine = engine_with_max_zones(2);
        let mut events = engine.subscribe_events();
        let _join = tokio::spawn(engine.start().expect("start"));

        let mut zone = engine.zone_builder().create().expect("default zone");
        let tab = zone.tab_builder().create().await.expect("tab");
        tab.send(TabCommand::LoadHtml {
            html: "<html><head><title>Default</title></head><body></body></html>".into(),
            base_url: "https://example.test/".into(),
        })
        .await
        .expect("load");
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Ok(EngineEvent::Navigation {
                    event: NavigationEvent::Finished { .. },
                    ..
                }) = events.recv().await
                {
                    return;
                }
            }
        })
        .await
        .expect("navigation should finish");
        assert_eq!(tab.title(), "Default");

        // A fixed id is honoured; a per-field override replaces just that field.
        let wanted = ZoneId::new();
        let custom = engine
            .zone_builder()
            .id(wanted)
            .cookie_jar(None)
            .create()
            .expect("custom zone");
        assert_eq!(custom.id, wanted);

        engine.close_zone(custom).await;
        engine.close_zone(zone).await;
        engine.shutdown().await.expect("shutdown");
    }

    /// A per-tab `Accept-Language` set through the builder must reach the wire.
    #[tokio::test(flavor = "current_thread")]
    async fn tab_builder_accept_language_reaches_the_wire() {
        use crate::events::NavigationEvent;
        use cow_utils::CowUtils;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let seen = std::sync::Arc::new(parking_lot::Mutex::new(String::new()));
        let seen_srv = seen.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let seen_srv = seen_srv.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 8192];
                    let n = stream.read(&mut buf).await.unwrap_or(0);
                    *seen_srv.lock() = String::from_utf8_lossy(&buf[..n]).into_owned();
                    let body = "<html><body>ok</body></html>";
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
        let mut events = engine.subscribe_events();
        let _join = tokio::spawn(engine.start().expect("start"));
        let mut zone = engine.zone_builder().create().expect("zone");
        let tab = zone
            .tab_builder()
            .accept_language("xx-TEST,xx;q=0.5")
            .url(format!("http://127.0.0.1:{port}/"))
            .create()
            .await
            .expect("tab");

        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Ok(EngineEvent::Navigation {
                    event: NavigationEvent::Finished { .. } | NavigationEvent::Failed { .. },
                    ..
                }) = events.recv().await
                {
                    return;
                }
            }
        })
        .await
        .expect("navigation should complete");

        let request = seen.lock().clone();
        assert!(
            request
                .cow_to_ascii_lowercase()
                .contains("accept-language: xx-test,xx;q=0.5"),
            "per-tab Accept-Language missing; request was:\n{request}"
        );
        let _ = tab;
        engine.close_zone(zone).await;
        engine.shutdown().await.expect("shutdown");
    }

    /// A download body is spooled before the embedder answers, so it is capped: past
    /// `net.download.max_spool_bytes` the navigation fails instead of filling the temp dir.
    #[tokio::test(flavor = "current_thread")]
    async fn oversized_download_fails_instead_of_spooling() {
        use crate::events::NavigationEvent;
        use crate::Setting;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let payload: Vec<u8> = vec![7u8; 64 * 1024];
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
                         Content-Disposition: attachment; filename=\"big.bin\"\r\n\
                         Content-Length: {}\r\nConnection: close\r\n\r\n",
                        payload.len()
                    );
                    let _ = stream.write_all(head.as_bytes()).await;
                    let _ = stream.write_all(&payload).await;
                });
            }
        });

        let mut engine = engine_with_max_zones(1);
        let mut events = engine.subscribe_events();
        let _join = tokio::spawn(engine.start().expect("start"));
        engine
            .settings()
            .set("net.download.max_spool_bytes", Setting::UInt(1024))
            .expect("set cap");
        let mut zone = engine.zone_builder().create().expect("zone");
        let tab = zone.tab_builder().create().await.expect("tab");
        tab.navigate(format!("http://127.0.0.1:{port}/big.bin"))
            .await
            .expect("navigate");

        let outcome = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                match events.recv().await {
                    Ok(EngineEvent::DownloadRequested { .. }) => return Err("offer was made past the cap"),
                    Ok(EngineEvent::Navigation {
                        event: NavigationEvent::Failed { error, .. },
                        ..
                    }) => return Ok(error),
                    Ok(_) => continue,
                    Err(e) => panic!("event stream closed: {e}"),
                }
            }
        })
        .await
        .expect("timed out");
        let error = outcome.expect("navigation should fail, not offer");
        assert!(error.to_string().contains("max_spool_bytes"), "got {error}");
        assert!(tab.pending_downloads().is_empty());

        engine.close_zone(zone).await;
        engine.shutdown().await.expect("shutdown");
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

    /// Embedders share the engine across threads; the run-only switch must
    /// not take that away.
    #[test]
    fn the_engine_stays_sync() {
        fn assert_sync<T: Sync>() {}
        assert_sync::<GosubEngine>();
    }

    /// The one switch decides all five for the run: off is the single-process
    /// engine whatever the per-component settings say, on requests every one,
    /// and unset leaves them alone.
    #[test]
    fn the_process_isolation_switch_decides_all_five() {
        use gosub_config::settings::Setting;
        let engine = engine_with_max_zones(1);
        let store = engine.settings();
        store.set("security.cookie_vault", Setting::Bool(true)).expect("set");
        store
            .set("security.renderer_process", Setting::Bool(true))
            .expect("set");

        engine.apply_process_isolation_switch();
        assert!(
            store.get_bool("security.cookie_vault"),
            "unset: the component setting stands"
        );

        engine.set_process_isolation(false).expect("switch off");
        engine.apply_process_isolation_switch();
        for key in PROCESS_SETTINGS {
            assert!(!store.get_bool(key), "{key} is on with the switch off");
        }
        assert!(
            !engine.setting_at_default("security.network_process"),
            "the switch is the embedder's choice for every component"
        );

        engine.set_process_isolation(true).expect("switch on");
        engine.apply_process_isolation_switch();
        for key in PROCESS_SETTINGS {
            assert!(store.get_bool(key), "{key} is off with the switch on");
        }

        // A flag's choice for this run outranks the stored one and is not
        // written back: the stored value stays `true`.
        engine
            .set_process_isolation_for_this_run(false)
            .expect("switch off for this run");
        engine.apply_process_isolation_switch();
        for key in PROCESS_SETTINGS {
            assert!(!store.get_bool(key), "{key} is on with the run's switch off");
        }
        assert!(
            store.is_overridden(PROCESS_ISOLATION_SWITCH),
            "the stored choice is still there"
        );
    }

    /// Where no backend confines a child, an explicit request is refused too:
    /// the setting goes off for the run, and the stored choice stays.
    #[cfg(feature = "process-isolation")]
    #[test]
    fn no_sandbox_backend_means_no_component_process() {
        use gosub_config::settings::Setting;
        let engine = engine_with_max_zones(1);
        let store = engine.settings();
        store.set("security.network_process", Setting::Bool(true)).expect("set");
        engine.set_process_isolation(true).expect("switch on");
        engine.apply_process_isolation_switch();

        engine.refuse_unconfined_children();
        for key in PROCESS_SETTINGS {
            assert!(!store.get_bool(key), "{key} is on with no sandbox to run it in");
        }
        assert!(
            store.is_overridden(PROCESS_ISOLATION_SWITCH),
            "the stored choice is still there"
        );
    }

    /// The defaults are on, but they too need `dispatch()`: an engine in a
    /// process that never dispatched ends up with all three off, quietly.
    #[cfg(feature = "process-isolation")]
    #[tokio::test]
    async fn process_settings_default_on_but_need_dispatch() {
        let mut engine = engine_with_max_zones(1);
        assert!(engine.settings().get_bool("security.network_process"));
        assert!(engine.settings().get_bool("security.image_decoder_process"));
        assert!(engine.settings().get_bool("security.renderer_process"));
        let _join = tokio::spawn(engine.start().expect("start"));
        assert!(!engine.settings().get_bool("security.network_process"));
        assert!(!engine.settings().get_bool("security.image_decoder_process"));
        assert!(!engine.settings().get_bool("security.renderer_process"));
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

        // Serves every connection: besides the two navigations the tab may fetch its
        // icon, and which connection comes second is not fixed. The request for
        // `/second` is the one the test is about, wherever it lands.
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                let mut buf = vec![0u8; 4096];
                let n = stream.read(&mut buf).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]).to_string();
                let first = request.starts_with("GET /first ");
                if request.starts_with("GET /second ") {
                    *captured.lock() = request;
                }

                let body = b"<html><title>hi</title></html>";
                let set_cookie = if first {
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
        let mut events = engine.subscribe_events();
        let _join = tokio::spawn(engine.start().expect("start"));

        let mut zone = engine.create_zone(None, services(), None).expect("zone");
        // One tab for both navigations: with no zone store or jar configured every
        // tab gets its own jar, so a second tab would start empty.
        let tab = zone.create_tab(Default::default(), None).await.expect("tab");

        tab.navigate(format!("http://127.0.0.1:{port}/first"))
            .await
            .expect("first navigation");
        // The store happens on the I/O side after the response arrives, so the
        // second navigation must not start until the first has been answered. The
        // cookie is stored before the reply is forwarded, so a finished navigation
        // is one whose cookie is already in the jar.
        assert!(
            wait_for(&mut events, |e| matches!(
                e,
                EngineEvent::Navigation {
                    event: crate::events::NavigationEvent::Finished { .. },
                    ..
                }
            ))
            .await,
            "the first navigation never finished"
        );

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

    /// A cookie a redirect sets reaches the jar before the redirect is
    /// followed, and rides on the next hop: the login shape, where a `302`
    /// hands out the session and sends the browser to the page that needs it.
    #[tokio::test]
    async fn a_cookie_set_on_a_redirect_rides_on_the_next_hop() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let home_request = Arc::new(Mutex::new(String::new()));
        let captured = home_request.clone();

        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                let mut buf = vec![0u8; 4096];
                let n = stream.read(&mut buf).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]).to_string();
                let head = if request.starts_with("GET /login ") {
                    "HTTP/1.1 302 Found\r\nLocation: /home\r\nSet-Cookie: sid=hop1; Path=/\r\n\
                     Content-Length: 0\r\nConnection: close\r\n\r\n"
                        .to_string()
                } else {
                    if request.starts_with("GET /home ") {
                        *captured.lock() = request;
                    }
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        .to_string()
                };
                let _ = stream.write_all(head.as_bytes()).await;
            }
        });

        let mut engine = engine_with_max_zones(1);
        let _events = engine.subscribe_events();
        let _join = tokio::spawn(engine.start().expect("start"));
        let mut zone = engine.create_zone(None, services(), None).expect("zone");
        let tab = zone.create_tab(Default::default(), None).await.expect("tab");

        tab.navigate(format!("http://127.0.0.1:{port}/login"))
            .await
            .expect("navigation");

        let mut request = String::new();
        for _ in 0..100 {
            request = home_request.lock().clone();
            if !request.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        use cow_utils::CowUtils;
        assert!(
            request.cow_to_ascii_lowercase().contains("cookie: sid=hop1"),
            "the redirect's cookie should ride on the next hop, got:\n{request}"
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
        let mut zone = engine.zone_builder().services(zone_services).create().expect("zone");

        // Tab creation resolves the persistent per-zone jar from the store.
        let _tab = zone.tab_builder().create().await.expect("tab");

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
        let mut zone = engine
            .zone_builder()
            .config(zone_cfg)
            .services(services())
            .create()
            .expect("zone");
        let tab = zone.tab_builder().create().await.expect("tab");
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

    /// A page served from the private network may load its neighbours, also when its host is
    /// a name: `localhost` is placed by the address its response came from, which the I/O
    /// side records before the page is parsed and asks for its stylesheet. (A literal host
    /// would be placed without the record; a name proves the record is there in time.)
    #[tokio::test]
    async fn a_private_page_by_name_loads_its_own_subresources() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let styles = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let styles_srv = styles.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let styles_srv = styles_srv.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 4096];
                    let n = stream.read(&mut buf).await.unwrap_or(0);
                    let (ctype, body) = if buf[..n].starts_with(b"GET /style.css ") {
                        styles_srv.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        ("text/css", "body { color: red; }")
                    } else {
                        (
                            "text/html",
                            "<html><head><link rel=\"stylesheet\" href=\"/style.css\"></head><body>hi</body></html>",
                        )
                    };
                    let head = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(head.as_bytes()).await;
                    let _ = stream.write_all(body.as_bytes()).await;
                });
            }
        });

        let mut engine = engine_with_max_zones(1);
        let _join = tokio::spawn(engine.start().expect("start"));
        let mut zone = engine.zone_builder().services(services()).create().expect("zone");
        let tab = zone.tab_builder().create().await.expect("tab");
        tab.navigate(format!("http://localhost:{port}/page"))
            .await
            .expect("navigate");

        tokio::time::timeout(Duration::from_secs(10), async {
            while styles.load(std::sync::atomic::Ordering::SeqCst) == 0 {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("the page's own stylesheet was refused");
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

        // Lets the test prove that accepting an offer does not re-request the URL.
        let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let hits_srv = hits.clone();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let payload = payload_srv.clone();
                let hits_srv = hits_srv.clone();
                tokio::spawn(async move {
                    hits_srv.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let mut buf = vec![0u8; 4096];
                    let n = stream.read(&mut buf).await.unwrap_or(0);
                    // The page save-link-as is used from; everything else is the binary.
                    if buf[..n].starts_with(b"GET /page ") {
                        let page = "<html><body><a href=\"/data/other.bin\">other</a></body></html>";
                        let head = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\
                             Content-Length: {}\r\nConnection: close\r\n\r\n",
                            page.len()
                        );
                        let _ = stream.write_all(head.as_bytes()).await;
                        let _ = stream.write_all(page.as_bytes()).await;
                        return;
                    }
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
        let mut zone = engine.zone_builder().services(services()).create().expect("zone");
        let tab = zone.tab_builder().create().await.expect("tab");

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
                        offer,
                        ..
                    }) => return (url, suggested_filename, total_bytes, nav_progress, offer),
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
        let served_before_accept = hits.load(std::sync::atomic::Ordering::SeqCst);
        assert_eq!(tab.pending_downloads().len(), 1, "the offer is listed while pending");
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("saved.bin");
        tab.send(TabCommand::StartDownload {
            id: DownloadId(7),
            url: offer.0.to_string(),
            target_path: target.clone(),
            offer: Some(offer.4),
        })
        .await
        .expect("start download");

        let (_progress_seen, received) = tokio::time::timeout(Duration::from_secs(10), async {
            let mut progress_seen = false;
            loop {
                match event_rx.recv().await {
                    Ok(EngineEvent::DownloadProgress { id: DownloadId(7), .. }) => progress_seen = true,
                    Ok(EngineEvent::DownloadFinished {
                        id: DownloadId(7),
                        received_bytes,
                        path,
                        ..
                    }) => {
                        assert_eq!(path, target);
                        return (progress_seen, received_bytes);
                    }
                    Ok(EngineEvent::DownloadFailed { error, .. }) => panic!("download failed: {error}"),
                    Ok(_) => continue,
                    Err(e) => panic!("event stream closed: {e}"),
                }
            }
        })
        .await
        .expect("timed out waiting for DownloadFinished");
        assert_eq!(received, payload.len() as u64);
        assert_eq!(std::fs::read(&target).unwrap(), payload, "file content must match");
        assert_eq!(
            hits.load(std::sync::atomic::Ordering::SeqCst),
            served_before_accept,
            "accepting a download offer must not re-fetch the URL"
        );

        // Save-link-as has no spooled body, so that path still fetches - and still reports
        // progress while it streams. From a page, as the context menu offers it: a tab
        // showing nothing has no document to judge a private-network load by, and is
        // refused one.
        assert!(tab.pending_downloads().is_empty(), "accepting consumes the offer");
        tab.navigate(format!("http://127.0.0.1:{port}/page"))
            .await
            .expect("navigate to the page");
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                match event_rx.recv().await {
                    Ok(EngineEvent::Navigation {
                        event: crate::events::NavigationEvent::Finished { .. },
                        ..
                    }) => return,
                    Ok(_) => continue,
                    Err(e) => panic!("event stream closed: {e}"),
                }
            }
        })
        .await
        .expect("timed out waiting for the page");
        let direct = dir.path().join("direct.bin");
        tab.send(TabCommand::StartDownload {
            id: DownloadId(9),
            url: format!("http://127.0.0.1:{port}/data/other.bin"),
            target_path: direct.clone(),
            offer: None,
        })
        .await
        .expect("start save-link-as");
        let direct_progress = tokio::time::timeout(Duration::from_secs(10), async {
            let mut progress_seen = false;
            loop {
                match event_rx.recv().await {
                    Ok(EngineEvent::DownloadProgress { id: DownloadId(9), .. }) => progress_seen = true,
                    Ok(EngineEvent::DownloadFinished { id: DownloadId(9), .. }) => return progress_seen,
                    Ok(EngineEvent::DownloadFailed {
                        id: DownloadId(9),
                        error,
                        ..
                    }) => {
                        panic!("save-link-as failed: {error}")
                    }
                    Ok(_) => continue,
                    Err(e) => panic!("event stream closed: {e}"),
                }
            }
        })
        .await
        .expect("timed out waiting for save-link-as to finish");
        assert!(direct_progress, "a fetched download still reports progress");
        assert_eq!(std::fs::read(&direct).unwrap(), payload);
        assert!(
            hits.load(std::sync::atomic::Ordering::SeqCst) > served_before_accept,
            "save-link-as must actually fetch"
        );

        // The failed-fetch path reports too (connection refused port).
        tab.send(TabCommand::StartDownload {
            id: DownloadId(8),
            url: "http://127.0.0.1:9/off".into(),
            target_path: dir.path().join("nope.bin"),
            offer: None,
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
        let mut zone = engine.zone_builder().services(zone_services).create().expect("zone");
        let tab = zone.tab_builder().create().await.expect("tab");

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

    /// An embedder that sets a process setting to its default value has still
    /// chosen it: on a platform where the default is turned off, that choice is
    /// what keeps it on.
    #[cfg(feature = "process-isolation")]
    #[test]
    fn an_explicit_process_setting_equal_to_its_default_is_not_at_default() {
        let engine = engine_with_max_zones(1);
        let key = "security.network_process";
        assert!(engine.setting_at_default(key), "untouched: at its default");

        let default = engine.settings().get_info(key).expect("known setting").default;
        engine.settings().set(key, default).expect("set to its own default");
        assert!(!engine.setting_at_default(key), "set explicitly, even to the default");
    }

    /// Crash containment: a panicking tab worker produces a TabCrashed event (instead of
    /// dying silently), and the tab's handle then reports closed on further commands.
    #[tokio::test]
    async fn worker_panic_emits_tab_crashed() {
        use crate::events::TabCommand;

        let mut engine = engine_with_max_zones(1);
        let mut event_rx = engine.subscribe_events();
        let _join = tokio::spawn(engine.start().expect("start"));
        let mut zone = engine.zone_builder().services(services()).create().expect("zone");
        let tab = zone.tab_builder().create().await.expect("tab");
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
        // The worker's own cleanup never ran, so the watchdog drops the tab's
        // identity: a fetch the dead tab left behind gets no cookies.
        assert!(
            engine.context.tab_identities.get(tab_id).is_none(),
            "a crashed tab must not resolve to its cookie jar"
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
        let mut zone = engine.zone_builder().services(services()).create().expect("zone");
        let tab = zone.tab_builder().create().await.expect("tab");
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

    /// Whether a page's icon served with `content_type` (none: no header at
    /// all) reaches the embedder as `FavIconChanged`.
    async fn favicon_delivered(content_type: Option<&'static str>) -> bool {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let icon_served = Arc::new(tokio::sync::Notify::new());
        let icon_served_srv = icon_served.clone();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let icon_served = icon_served_srv.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 4096];
                    let n = stream.read(&mut buf).await.unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]).to_string();
                    let Some(path) = req.split_whitespace().nth(1) else {
                        return;
                    };
                    let head = if path == "/icon" {
                        let ctype = content_type
                            .map(|t| format!("Content-Type: {t}\r\n"))
                            .unwrap_or_default();
                        format!("HTTP/1.1 200 OK\r\n{ctype}Content-Length: 8\r\nConnection: close\r\n\r\nPNGBYTES")
                    } else {
                        let body = "<html><head><link rel=\"icon\" href=\"/icon\"></head><body>p</body></html>";
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        )
                    };
                    let _ = stream.write_all(head.as_bytes()).await;
                    if path == "/icon" {
                        icon_served.notify_one();
                    }
                });
            }
        });

        let mut engine = engine_with_max_zones(1);
        let mut event_rx = engine.subscribe_events();
        let _join = tokio::spawn(engine.start().expect("start"));
        let mut zone = engine.create_zone(None, services(), None).expect("zone");
        let tab = zone.create_tab(Default::default(), None).await.expect("tab");
        tab.navigate(format!("http://127.0.0.1:{port}/"))
            .await
            .expect("navigate");

        tokio::time::timeout(Duration::from_secs(10), icon_served.notified())
            .await
            .expect("the page's icon was never requested");
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if let Ok(EngineEvent::FavIconChanged { .. }) = event_rx.recv().await {
                    return;
                }
            }
        })
        .await
        .is_ok()
    }

    /// A page's icon served without an image Content-Type never reaches the
    /// embedder: it would hand the bytes to an image decoder.
    #[tokio::test]
    async fn a_favicon_without_an_image_type_is_dropped() {
        assert!(
            !favicon_delivered(None).await,
            "an icon without a Content-Type reached the embedder"
        );
        assert!(
            !favicon_delivered(Some("text/html")).await,
            "an icon typed text/html reached the embedder"
        );
    }

    /// The image type is matched without regard to case, as media types are.
    #[tokio::test]
    async fn a_favicon_type_is_matched_without_case() {
        assert!(
            favicon_delivered(Some("Image/PNG")).await,
            "an icon typed Image/PNG was dropped"
        );
    }

    /// Enter on a focused link goes through the same scheme check as a click:
    /// a page's link never opens an internal page.
    #[tokio::test]
    async fn enter_on_a_link_does_not_open_an_internal_page() {
        use crate::events::{Modifiers, NavigationEvent, TabCommand};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 4096];
                    let _ = stream.read(&mut buf).await;
                    let body = "<html><body><a href=\"gosub://settings\">settings</a></body></html>";
                    let head = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(head.as_bytes()).await;
                });
            }
        });

        let mut engine = engine_with_max_zones(1);
        let mut event_rx = engine.subscribe_events();
        let _join = tokio::spawn(engine.start().expect("start"));
        let mut zone = engine.create_zone(None, services(), None).expect("zone");
        let tab = zone.create_tab(Default::default(), None).await.expect("tab");
        tab.navigate(format!("http://127.0.0.1:{port}/"))
            .await
            .expect("navigate");
        assert!(
            wait_for(&mut event_rx, |ev| matches!(
                ev,
                EngineEvent::Navigation {
                    event: NavigationEvent::Finished { .. },
                    ..
                }
            ))
            .await,
            "the page never loaded"
        );

        for key in ["Tab", "Enter"] {
            let _ = tab
                .send(TabCommand::KeyDown {
                    key: key.into(),
                    code: key.into(),
                    modifiers: Modifiers::empty(),
                })
                .await;
        }
        let opened = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if let Ok(EngineEvent::Navigation {
                    event: NavigationEvent::Started { url, .. },
                    ..
                }) = event_rx.recv().await
                {
                    return url;
                }
            }
        })
        .await;
        assert!(opened.is_err(), "Enter on a page's gosub: link navigated: {opened:?}");
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
                    // A connection that closes without a request line is a fetch the
                    // engine abandoned (the next navigation cancels the last one's icon),
                    // not a page load: it must not count as one.
                    let Some(path) = req.split_whitespace().nth(1).map(str::to_string) else {
                        return;
                    };
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
        let mut zone = engine.zone_builder().services(services()).create().expect("zone");
        let tab = zone.tab_builder().create().await.expect("tab");

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
        let mut zone = engine.zone_builder().services(zone_services).create().expect("zone");
        let zone_id = zone.id;
        let _tab = zone.tab_builder().create().await.expect("tab");

        // Store a cookie through the zone's persistent jar.
        let jar = store.jar_for(zone_id).expect("persistent jar");
        let url = url::Url::parse("https://example.com/").unwrap();
        let mut headers = http::HeaderMap::new();
        headers.append(http::header::SET_COOKIE, "sid=closed42; Path=/".parse().unwrap());
        jar.write().store_response_cookies(&url, &headers, None);

        // The single max_zones slot is taken.
        assert!(matches!(
            engine.zone_builder().services(services()).create(),
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
            .zone_builder()
            .services(services())
            .create()
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
        // Zones need the I/O runtime.
        let _join = tokio::spawn(engine.start().expect("start"));

        // `None` config also exercises the default_zone_config fallback.
        engine
            .zone_builder()
            .services(services())
            .create()
            .expect("first zone fits");

        let err = engine.zone_builder().services(services()).create().unwrap_err();
        assert!(matches!(err, EngineError::ZoneLimitExceeded));

        engine.shutdown().await.expect("shutdown");
    }
}
