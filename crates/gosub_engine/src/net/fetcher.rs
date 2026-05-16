// Re-export the net-layer Fetcher and supporting types
pub use gosub_net::net::fetcher::{FetchInflightMap, Fetcher, FetcherConfig};
pub use gosub_net::net::fetcher_context::FetcherContext;

use crate::engine::types::EventChannel;
use crate::net::emitter::engine_event_emitter::EngineEventEmitter;
use crate::net::emitter::null_emitter::NullEmitter;
use crate::net::req_ref_tracker::{RequestRefTracker, RequestReferenceMap};
use gosub_net::net::observer::NetObserver;
use gosub_net::net::request_ref::RequestReference;
use gosub_net::net::types::{Initiator, ResourceKind};
use gosub_net::types::RequestId;
use parking_lot::RwLock;
use std::sync::Arc;

/// Engine-side implementation of FetcherContext.
/// Bridges the net-layer Fetcher to engine events and tab tracking.
pub struct EngineNetContext {
    pub event_tx: EventChannel,
    pub request_reference_map: Arc<RwLock<RequestReferenceMap>>,
    pub request_ref_tracker: Arc<RequestRefTracker>,
}

impl FetcherContext for EngineNetContext {
    fn observer_for(
        &self,
        reference: RequestReference,
        req_id: RequestId,
        kind: ResourceKind,
        initiator: Initiator,
    ) -> Arc<dyn NetObserver + Send + Sync> {
        let guard = self.request_reference_map.read();
        match guard.get(&reference) {
            Some(&tab_id) => Arc::new(EngineEventEmitter::new(
                tab_id,
                req_id,
                reference,
                self.event_tx.clone(),
                kind,
                initiator,
            )),
            None => {
                log::trace!("Cannot find the request reference for reference {:?}", reference);
                Arc::new(NullEmitter) as Arc<dyn NetObserver + Send + Sync>
            }
        }
    }

    fn on_ref_active(&self, reference: RequestReference) {
        self.request_ref_tracker.inc(&reference);
    }

    fn on_ref_done(&self, reference: RequestReference) {
        self.request_ref_tracker
            .dec_and_maybe_cleanup(&reference, &self.request_reference_map);
    }
}
