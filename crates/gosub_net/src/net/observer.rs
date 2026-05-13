use crate::net::events::NetEvent;

/// A NetObserver allows sending NetEvents to emitters.
/// Emitters bridge the net stack to other parts of the system (e.g. engine events, logging).
pub trait NetObserver: Send + Sync {
    fn on_event(&self, ev: NetEvent);
}
