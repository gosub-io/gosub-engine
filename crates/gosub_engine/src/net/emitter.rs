use crate::net::events::NetEvent;

/// Emitters are a way to send NetEvents triggered by the fetch() function to other parts of the engine.
/// Most likely, they are converted to EngineEvents, which are send over the event_tx channel to the UA.
/// It's possible to ignore any events by using the NullEmitter.
/// It could also be possible to send events to multiple emitters, for instance, both the EngineEvent and
/// a json log emitter.
pub mod engine_event_emitter;
pub mod null_emitter;

/// A NetObserver allows to send NetEvents to emitters
pub trait NetObserver: Send + Sync {
    fn on_event(&self, ev: NetEvent);
}
