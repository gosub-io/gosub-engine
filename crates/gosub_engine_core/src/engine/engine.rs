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
use crate::net::emitter::engine_observer_factory::EngineObserverFactory;
use crate::net::req_ref_tracker::RequestReferenceMap;
use crate::net::{spawn_io_thread, FetcherConfig, IoContext, IoHandle};
use crate::render::backend::{CompositorSink, RenderBackend};
use crate::render::DefaultCompositor;
use crate::util::spawn_named;
use crate::zone::{Zone, ZoneConfig, ZoneId, ZoneServices, ZoneSink};
use crate::{EngineConfig, EngineError};
use anyhow::Result;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tracing::instrument;

/// Main Gosub engine struct
pub struct GosubEngine {
    /// Context is what can be shared downstream
    context: Arc<EngineContext>,
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

// Engine context that is shared downwards to zones.
#[derive(Clone)]
pub struct EngineContext {
    /// Active render backend for the engine.
    pub render_backend: Arc<dyn RenderBackend + Send + Sync>,
    /// Compositor router sink to connect rendering output to the caller
    pub compositor: Arc<RwLock<dyn CompositorSink + Send + Sync>>,
    /// Event sender
    pub event_tx: EventChannel,
    /// Global engine configuration
    pub config: Arc<EngineConfig>,
    /// I/O thread handle
    pub io_tx: Arc<RwLock<Option<IoChannel>>>,
    /// Map for requests to tabs
    pub request_reference_map: Arc<RwLock<RequestReferenceMap>>,
}

impl Default for EngineContext {
    fn default() -> Self {
        Self {
            render_backend: Arc::new(crate::render::backends::null::NullBackend::new().unwrap()),
            compositor: Arc::new(RwLock::new(DefaultCompositor::new(|| {}))),
            event_tx: broadcast::channel::<EngineEvent>(DEFAULT_CHANNEL_CAPACITY).0,
            config: Arc::new(EngineConfig::default()),
            io_tx: Arc::new(RwLock::new(None)),
            request_reference_map: Arc::new(RwLock::new(RequestReferenceMap::new())),
        }
    }
}

impl GosubEngine {
    /// Create a new engine.
    ///
    /// If `config` is `None`, [`EngineConfig::default`] is used.
    ///
    /// ```
    /// # use gosub_engine as ge;
    /// let backend = ge::render::backends::null::NullBackend::new().unwrap();
    /// let engine = ge::GosubEngine::new(None, Box::new(backend));
    /// ```
    pub fn new(
        config: Option<EngineConfig>,
        backend: Arc<dyn RenderBackend + Send + Sync>,
        compositor: Arc<RwLock<dyn CompositorSink + Send + Sync>>,
    ) -> Self {
        let resolved_config = config.unwrap_or_default();

        // Command channel on which to send and receive engine commands from the UA.
        let (cmd_tx, cmd_rx) = mpsc::channel::<EngineCommand>(DEFAULT_CHANNEL_CAPACITY);

        // Broadcast event bus. Subscribe to receive engine events (including zone and tab events)
        let (event_tx, _first_rx) = broadcast::channel::<EngineEvent>(DEFAULT_CHANNEL_CAPACITY);

        Self {
            context: Arc::new(EngineContext {
                render_backend: backend,
                compositor,
                event_tx: event_tx.clone(),
                config: Arc::new(resolved_config),
                io_tx: Arc::new(RwLock::new(None)),
                request_reference_map: Arc::new(RwLock::new(RequestReferenceMap::new())),
            }),
            zones: HashMap::new(),
            cmd_tx,
            cmd_rx: Some(cmd_rx),
            io_handle: None,
            running: false,
        }
    }

    /// Starts the engine and returns the join handle of the main run loop task.
    pub fn start(&mut self) -> Result<Option<JoinHandle<()>>, EngineError> {
        if self.running {
            return Err(EngineError::AlreadyRunning);
        }

        // Start I/O thread
        let io_cfg = FetcherConfig::default();
        let io_ctx = Arc::new(IoContext {
            observer_factory: Arc::new(EngineObserverFactory {
                event_tx: self.context.event_tx.clone(),
                request_reference_map: self.context.request_reference_map.clone(),
            }),
            request_reference_map: self.context.request_reference_map.clone(),
        });
        let io_handle = spawn_io_thread(io_cfg, io_ctx);
        let io_tx = io_handle.subscribe();
        {
            let mut guard = self.context.io_tx.write().unwrap();
            *guard = Some(io_tx);
        }
        self.io_handle = Some(io_handle);

        // Start main engine run loop
        let join_handle = self.run().map(|task| spawn_named("Engine runner", task));

        Ok(join_handle)
    }

    /// Return a receiver for engine events.
    pub fn subscribe_events(&self) -> broadcast::Receiver<EngineEvent> {
        self.context.event_tx.subscribe()
    }

    pub fn backend(&self) -> Arc<dyn RenderBackend + Send + Sync> {
        Arc::clone(&self.context.render_backend)
    }

    /// Give this to zones/tabs when constructing them.
    pub fn compositor(&self) -> Arc<RwLock<dyn CompositorSink + Send + Sync>> {
        Arc::clone(&self.context.compositor)
    }

    /// Get a clone of the engine’s command sender (mainly for testing or
    /// custom handles).
    #[cfg(test)]
    #[allow(unused)]
    fn command_sender(&self) -> mpsc::Sender<EngineCommand> {
        self.cmd_tx.clone()
    }

    /// Run the engine’s inbound command loop in a dedicated thread/task.
    pub fn run<'b>(&mut self) -> Option<impl std::future::Future<Output = ()> + 'b> {
        self.running = true;

        let _ = self.context.event_tx.send(EngineEvent::EngineStarted);

        let mut cmd_rx = self.cmd_rx.take()?;

        Some(async move {
            #[allow(clippy::never_loop)]
            while let Some(cmd) = cmd_rx.recv().await {
                match cmd {
                    EngineCommand::Shutdown { reply } => {
                        log::trace!("Engine received shutdown command. Shutting down main engine::run() loop");
                        let _ = reply.send(Ok(()));
                        break;
                    }
                    _ => {
                        unimplemented!("unhandled engine command: {:?}", cmd);
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
        if let Some(io) = self.io_handle.take() {
            if let Err(e) = timeout(Duration::from_secs(10), io.shutdown()).await {
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
    ) -> Result<Zone, EngineError> {
        let zone = match zone_id {
            Some(zone_id) => Zone::new_with_id(zone_id, config, services, self.context.clone()),
            None => Zone::new(config, services, self.context.clone()),
        };

        let zone_id = zone.id;
        self.zones.insert(zone.id, zone.sink.clone());

        let _ = self.context.event_tx.send(EngineEvent::ZoneCreated { zone_id });

        Ok(zone)
    }
}
