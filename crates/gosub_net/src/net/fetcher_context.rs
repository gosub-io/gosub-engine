use crate::net::observer::NetObserver;
use crate::net::request_ref::RequestReference;
use crate::net::types::{Initiator, ResourceKind};
use crate::types::RequestId;
use std::sync::Arc;

/// Abstracts the engine-side plumbing the Fetcher needs: observer creation and reference lifecycle.
/// Implement this in the engine to wire up event routing without the net crate depending on
/// engine-specific types like TabId or EventChannel.
pub trait FetcherContext: Send + Sync {
    /// Return an observer to emit NetEvents for this specific request.
    fn observer_for(
        &self,
        reference: RequestReference,
        req_id: RequestId,
        kind: ResourceKind,
        initiator: Initiator,
    ) -> Arc<dyn NetObserver + Send + Sync>;

    /// Called once when the Fetcher becomes the leader for a new unique fetch.
    fn on_ref_active(&self, reference: RequestReference);

    /// Called once when all subscribers for a fetch are done and the entry can be cleaned up.
    fn on_ref_done(&self, reference: RequestReference);
}
