use crate::engine::types::EventChannel;
use crate::net::emitter::engine_event_emitter::EngineEventEmitter;
use gosub_net::emitter::{NetObserver, ObserverFactory};
use gosub_net::net_types::{Initiator, ResourceKind};
use gosub_net::req_ref_tracker::{RequestReference, RequestReferenceMap};
use gosub_net::types::{RequestId, TabId};
use std::sync::{Arc, RwLock};

/// Engine-specific ObserverFactory that creates EngineEventEmitter instances.
/// It looks up the TabId from the RequestReferenceMap using the request reference,
/// then creates an EngineEventEmitter routing events to the correct tab.
pub struct EngineObserverFactory {
    pub event_tx: EventChannel,
    pub request_reference_map: Arc<RwLock<RequestReferenceMap>>,
}

impl ObserverFactory for EngineObserverFactory {
    fn create_observer(
        &self,
        reference: &RequestReference,
        req_id: RequestId,
        kind: ResourceKind,
        initiator: Initiator,
    ) -> Arc<dyn NetObserver> {
        // Look up the TabId from the RequestReferenceMap
        let tab_id: TabId = {
            let guard = self.request_reference_map.read().unwrap();
            guard.get(reference).copied().unwrap_or_default()
        };

        // Convert gosub_net::types::TabId to engine TabId
        let engine_tab_id = tab_id;

        Arc::new(EngineEventEmitter::new(
            engine_tab_id,
            req_id,
            *reference,
            self.event_tx.clone(),
            kind,
            initiator,
        ))
    }
}
