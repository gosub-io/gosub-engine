use tracing::instrument;
use crate::emitter::NetObserver;
use crate::events::NetEvent;

/// Emitter that will drop any events received
pub struct NullEmitter;

impl NetObserver for NullEmitter {
    #[instrument(
        name = "net.observer",
        level = "debug",
        skip(self),
    )]
    fn on_event(&self, _ev: NetEvent) {
        // Do nothing with the event
        log::trace!("NullEmitter received an event, but will ignore it.");
    }
}
