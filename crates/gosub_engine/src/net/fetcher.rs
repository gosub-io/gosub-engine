// Re-export the net-layer Fetcher and supporting types from the external gosub-sonar crate
pub use gosub_sonar::net::fetcher::{Fetcher, FetcherConfig};
pub use gosub_sonar::net::fetcher_context::FetcherContext;

use crate::engine::types::EventChannel;
use crate::net::emitter::engine_event_emitter::EngineEventEmitter;
use crate::net::emitter::null_emitter::NullEmitter;
use crate::net::req_ref_tracker::{RequestRefTracker, RequestReferenceMap, REF_REGISTRY};
use crate::net::types::{Initiator as EngineInitiator, ResourceKind as EngineResourceKind};
use gosub_sonar::net::observer::NetObserver;
use gosub_sonar::net::types::{Initiator, ResourceKind};
use gosub_sonar::types::RequestId;
use parking_lot::RwLock;
use std::sync::Arc;

/// Engine-side implementation of FetcherContext.
/// Bridges the net-layer Fetcher to engine events and tab tracking.
///
/// The fetcher hands us the opaque sonar-side reference tags; we resolve them back to the
/// engine's rich [`RequestReference`](crate::net::req_ref_tracker::RequestReference) via
/// [`REF_REGISTRY`] before touching engine state.
pub struct EngineNetContext {
    pub event_tx: EventChannel,
    pub request_reference_map: Arc<RwLock<RequestReferenceMap>>,
    pub request_ref_tracker: Arc<RequestRefTracker>,
}

impl FetcherContext for EngineNetContext {
    fn observer_for(
        &self,
        reference: gosub_sonar::RequestReference,
        req_id: RequestId,
        kind: ResourceKind,
        initiator: Initiator,
    ) -> Arc<dyn NetObserver + Send + Sync> {
        let Some(reference) = REF_REGISTRY.from_net(reference) else {
            log::trace!("Cannot resolve net reference {:?} to an engine reference", reference);
            return Arc::new(NullEmitter) as Arc<dyn NetObserver + Send + Sync>;
        };

        // Recover the rich (kind, initiator) pair registered when the request was built;
        // sonar only carries its own coarse classification through the pipeline.
        let (kind, initiator) = REF_REGISTRY
            .request_meta(req_id)
            .unwrap_or_else(|| (EngineResourceKind::from_net(kind), EngineInitiator::from_net(initiator)));

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

    fn on_ref_active(&self, reference: gosub_sonar::RequestReference) {
        if let Some(reference) = REF_REGISTRY.from_net(reference) {
            self.request_ref_tracker.inc(&reference);
        }
    }

    fn on_ref_done(&self, reference: gosub_sonar::RequestReference) {
        if let Some(reference) = REF_REGISTRY.from_net(reference) {
            self.request_ref_tracker
                .dec_and_maybe_cleanup(&reference, &self.request_reference_map);
        }
    }
}
