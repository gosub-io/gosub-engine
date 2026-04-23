use crate::engine::DEFAULT_CHANNEL_CAPACITY;
use crate::events::TabCommand;
use crate::tab::services::EffectiveTabServices;
use crate::tab::sink::TabSink;
use crate::tab::worker::TabWorker;
use crate::tab::TabHandle;
use crate::zone::{ZoneContext, ZoneId};
use std::fmt::Display;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use uuid::Uuid;

/// A unique identifier for a browser tab within a [`GosubEngine`](crate::engine::GosubEngine).
///
/// Internally, a `TabId` is a wrapper around a [`Uuid`], ensuring global
/// uniqueness for each tab opened in the engine. `TabId` implements
/// common traits such as `Copy`, `Clone`, `Eq`, `Hash`, and ordering traits,
/// so it can be freely duplicated, compared, sorted, or used as a key in
/// hash maps.
///
/// **Note:** The use of [`Uuid`] is an implementation detail and may change
/// in the future without notice. You should not depend on the internal
/// representation; always treat `TabId` as an opaque handle.
///
/// # Purpose
///
/// Tabs in Gosub are lightweight handles representing an open page
/// (or a rendering context) within a [`Zone`](crate::engine::zone::Zone). `TabId` allows the engine
/// and user code to unambiguously reference and operate on a specific tab,
/// even if tabs are opened or closed dynamically.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TabId(Uuid);

impl TabId {
    /// Create a new unique `TabId` using a random UUID.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Display for TabId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Create a new tab without spawning the run() function. This allows callers to place the worker in
/// its own task or manage its lifecycle differently.
pub fn create_tab(
    zone_id: ZoneId,
    services: EffectiveTabServices,
    zone_context: Arc<ZoneContext>,
) -> anyhow::Result<(TabHandle, TabWorker)> {
    let (cmd_tx, cmd_rx) = mpsc::channel::<TabCommand>(DEFAULT_CHANNEL_CAPACITY);
    let tab_id = TabId::new();
    let sink = Arc::new(TabSink::new());

    let worker = TabWorker::new(
        tab_id,
        zone_id,
        services,
        zone_context,
        sink.clone(),
        cmd_rx,
    );

    let handle = TabHandle { tab_id, cmd_tx, sink };
    Ok((handle, worker))
}

/// Creates a new tab and spawns the worker on the current tokio runtime.
pub fn create_tab_and_spawn(
    zone_id: ZoneId,
    services: EffectiveTabServices,
    zone_context: Arc<ZoneContext>,
) -> anyhow::Result<(TabHandle, JoinHandle<()>)> {
    let (tab_handle, worker) = create_tab(zone_id, services, zone_context)?;
    let join_handle = worker.spawn_worker()?;
    Ok((tab_handle, join_handle))
}
