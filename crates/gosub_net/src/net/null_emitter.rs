use crate::net::events::NetEvent;
use crate::net::observer::NetObserver;
use tracing::instrument;

/// Emitter that drops all received events
pub struct NullEmitter;

impl NetObserver for NullEmitter {
    #[instrument(name = "net.observer", level = "debug", skip(self))]
    fn on_event(&self, _ev: NetEvent) {
        log::trace!("NullEmitter received an event, but will ignore it.");
    }
}
