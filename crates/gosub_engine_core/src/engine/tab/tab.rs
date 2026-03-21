use crate::engine::DEFAULT_CHANNEL_CAPACITY;
use crate::events::TabCommand;
use crate::tab::services::EffectiveTabServices;
use crate::tab::sink::TabSink;
use crate::tab::worker::TabWorker;
use crate::tab::TabHandle;
use crate::zone::{ZoneContext, ZoneId};
use gosub_interface::config::HasDocument;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

pub use gosub_net::types::TabId;

/// Create a new tab without spawning the run() function. This allows callers to place the worker in
/// its own task or manage its lifecycle differently.
pub fn create_tab<C: HasDocument + Send + Sync + 'static>(
    zone_id: ZoneId,
    services: EffectiveTabServices,
    zone_context: Arc<ZoneContext>,
) -> anyhow::Result<(TabHandle, TabWorker<C>)>
where
    C::Document: Send + Sync,
{
    let (cmd_tx, cmd_rx) = mpsc::channel::<TabCommand>(DEFAULT_CHANNEL_CAPACITY);
    let tab_id = TabId::new();
    let sink = Arc::new(TabSink::new());

    let worker = TabWorker::<C>::new(
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
pub fn create_tab_and_spawn<C: HasDocument + Send + Sync + 'static>(
    zone_id: ZoneId,
    services: EffectiveTabServices,
    zone_context: Arc<ZoneContext>,
) -> anyhow::Result<(TabHandle, JoinHandle<()>)>
where
    C::Document: Send + Sync,
{
    let (tab_handle, worker) = create_tab::<C>(zone_id, services, zone_context)?;
    let join_handle = worker.spawn_worker()?;
    Ok((tab_handle, join_handle))
}
