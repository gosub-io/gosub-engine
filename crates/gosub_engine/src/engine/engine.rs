//! Engine core implementation.
//!
//! This module defines the [`GosubEngine`] struct, which is the main entry point for
//! creating and managing the engine, zones, and event bus. It also provides the
//! [`EngineContext`] struct for sharing resources and configuration across the engine.
//!
//! # Overview
//!
//! The engine is responsible for running zones and handling events. It provides a
//! command interface for starting, stopping, and configuring zones, as well as
//! subscribing to events from the engine and zones.
//!
//! # Main Types
//!
//! - [`GosubEngine`]: The main engine struct.
//! - [`EngineContext`]: Shared context for the engine, containing configuration and
//!   backend information.
//! - [`Zone`]: Represents a zone managed by the engine.
//! - [`EngineCommand`]: Commands that can be sent to the engine.
//! - [`EngineEvent`]: Events emitted by the engine, such as zone creation and
//!   destruction.

use crate::engine::events::{EngineCommand, EngineEvent};
use crate::engine::types::{EventChannel, IoChannel};
use crate::engine::DEFAULT_CHANNEL_CAPACITY;
use crate::html::RenderConfiguration;
use crate::net::req_ref_tracker::RequestReferenceMap;
use crate::net::{fetcher_config_from, spawn_io_thread, IoHandle};
use crate::zone::{Zone, ZoneConfig, ZoneId, ZoneServices, ZoneSink};
use crate::{EngineError, EngineSettings};
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
    pub config: Arc<EngineSettings>,
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
}

impl Default for EngineContext {
    fn default() -> Self {
        Self {
            event_tx: broadcast::channel::<EngineEvent>(DEFAULT_CHANNEL_CAPACITY).0,
            config: Arc::new(EngineSettings::default()),
            config_store: crate::engine::settings_store::default_config(),
            io_tx: OnceLock::new(),
            request_reference_map: Arc::new(RwLock::new(RequestReferenceMap::new())),
        }
    }
}

impl<C: RenderConfiguration> GosubEngine<C> {
    /// Create a new engine.
    ///
    /// If `config` is `None`, [`EngineSettings::default`] is used.
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
        config: Option<EngineSettings>,
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
            }),
            render_backend: backend,
            compositor,
            font_system: Arc::new(Mutex::new(C::FontSystem::default())),
            zones: HashMap::new(),
            cmd_tx,
            cmd_rx: Some(cmd_rx),
            io_handle: None,
            running: false,
        }
    }

    /// Starts the engine's I/O runtime and returns the main run-loop future.
    ///
    /// The returned future is intentionally **not** spawned: the caller decides how to drive it —
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

        // Hand the run-loop future to the caller to drive (spawn / await / select!) rather than
        // spawning it ourselves. `run()` yields `None` only if the loop was already taken, which
        // cannot happen here since `self.running` was false above.
        self.run().ok_or(EngineError::AlreadyRunning)
    }

    /// Return a receiver for engine events.
    pub fn subscribe_events(&self) -> broadcast::Receiver<EngineEvent> {
        self.context.event_tx.subscribe()
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
            while let Some(cmd) = cmd_rx.recv().await {
                match cmd {
                    EngineCommand::Shutdown { reply } => {
                        log::trace!("Engine received shutdown command. Shutting down main engine::run() loop");
                        let _ = reply.send(Ok(()));
                        break;
                    }
                    _ => {
                        log::warn!("Unhandled engine command: {:?}", cmd);
                    }
                }
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

    #[allow(unused)]
    fn flush_persistence(&mut self) {
        // if let Ok(zones) = self.zones.read() {
        //     for zone in zones.values() {
        //         if let Some(store) = zone.cookie_store_handle() {
        //             store.persist_all();
        //         }
        //     }
        // }
    }

    /// Create and register a new zone, returning a [`ZoneHandle`] for userland code.
    ///
    /// - `config`: zone configuration (features, limits, identity)
    /// - `services`: storage, cookie store/jar, partition policy, etc.
    /// - `zone_id`: optional id; if `None`, a fresh one is generated
    /// - `event_tx`: channel where the zone (and its tabs) will emit [`EngineEvent`]s
    ///
    /// The returned handle contains the [`ZoneId`] and a clone of the engine’s
    /// command sender, allowing the caller to send zone commands without holding
    /// a reference to the engine.
    pub fn create_zone(
        &mut self,
        config: ZoneConfig,
        services: ZoneServices,
        zone_id: Option<ZoneId>,
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

        self.context
            .event_tx
            .send(EngineEvent::ZoneCreated { zone_id })
            .map_err(|e| EngineError::Internal(e.into()))?;

        Ok(zone)
    }
}
