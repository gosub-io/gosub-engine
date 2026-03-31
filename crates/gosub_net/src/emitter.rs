use crate::events::NetEvent;
use crate::net_types::{Initiator, ResourceKind};
use crate::req_ref_tracker::RequestReference;
use crate::types::RequestId;
use std::sync::Arc;

pub mod null_emitter;

/// A NetObserver allows to send NetEvents to emitters
pub trait NetObserver: Send + Sync {
    fn on_event(&self, ev: NetEvent);
}

/// ObserverFactory creates NetObservers for requests.
/// This pattern decouples gosub_net from engine-level event types.
pub trait ObserverFactory: Send + Sync {
    fn create_observer(
        &self,
        reference: &RequestReference,
        req_id: RequestId,
        kind: ResourceKind,
        initiator: Initiator,
    ) -> Arc<dyn NetObserver>;
}

/// A NullObserverFactory that creates NullEmitters for all requests.
pub struct NullObserverFactory;

impl ObserverFactory for NullObserverFactory {
    fn create_observer(
        &self,
        _reference: &RequestReference,
        _req_id: RequestId,
        _kind: ResourceKind,
        _initiator: Initiator,
    ) -> Arc<dyn NetObserver> {
        Arc::new(null_emitter::NullEmitter)
    }
}
