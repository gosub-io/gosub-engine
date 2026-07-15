use crate::engine::types::NavigationId;
use crate::net::types::{Initiator, ResourceKind};
use crate::tab::TabId;
use dashmap::{DashMap, Entry};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::fmt::Display;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock};

/// Opaque ID for a document sub-resource load group
pub type DocumentId = u64;
/// Opaque ID for a background prefetch task
pub type PrefetchId = u64;
/// Opaque ID for a miscellaneous background task
pub type TaskId = u64;

/// Request references indicate what initiated a request without the net layer needing to know
/// about higher-level engine concepts like tabs. This keeps the net module independent of the
/// engine's tab/navigation machinery while still allowing it to emit typed events.
///
/// The external `gosub-sonar` fetching crate only knows opaque `u64` correlation tags
/// ([`gosub_sonar::RequestReference`]); use [`REF_REGISTRY`] to intern an engine reference
/// into a sonar tag when building a `FetchRequest` and to resolve it back in fetcher
/// callbacks.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Copy)]
pub enum RequestReference {
    /// Main doc for a tab
    Navigation(NavigationId),
    /// Sub resources of a specific doc
    Document(DocumentId),
    /// Background prefetches
    Prefetch(PrefetchId),
    /// Misc/system
    Background(TaskId),
}

impl Display for RequestReference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RequestReference::Navigation(id) => write!(f, "Nav({})", id),
            RequestReference::Document(id) => write!(f, "Doc({})", id),
            RequestReference::Prefetch(id) => write!(f, "Prefetch({})", id),
            RequestReference::Background(id) => write!(f, "BG({})", id),
        }
    }
}

/// Process-wide interning registry between the engine's rich [`RequestReference`] and the
/// opaque `Tagged(u64)` references gosub-sonar carries through its fetch pipeline.
///
/// Entries currently live for the lifetime of the process (one small entry per navigation /
/// load group); if that ever matters, cleanup can be tied to `on_ref_done` finalization.
pub static REF_REGISTRY: LazyLock<RefRegistry> = LazyLock::new(RefRegistry::new);

pub struct RefRegistry {
    forward: DashMap<RequestReference, u64>,
    reverse: DashMap<u64, RequestReference>,
    next: AtomicU64,
    /// Rich per-request classification (kind, initiator) that gosub-sonar's coarse enums
    /// cannot carry through the fetch pipeline. Registered when a `FetchRequest` is built,
    /// looked up in fetcher callbacks, and dropped again on terminal fetch events.
    request_meta: DashMap<crate::engine::types::RequestId, (ResourceKind, Initiator)>,
}

impl RefRegistry {
    fn new() -> Self {
        Self {
            forward: DashMap::new(),
            reverse: DashMap::new(),
            next: AtomicU64::new(1),
            request_meta: DashMap::new(),
        }
    }

    /// Remember the rich (kind, initiator) pair for a request about to be submitted.
    pub fn register_request(&self, req_id: crate::engine::types::RequestId, kind: ResourceKind, initiator: Initiator) {
        self.request_meta.insert(req_id, (kind, initiator));
    }

    /// Look up the rich (kind, initiator) pair registered for a request.
    pub fn request_meta(&self, req_id: crate::engine::types::RequestId) -> Option<(ResourceKind, Initiator)> {
        self.request_meta.get(&req_id).map(|m| *m)
    }

    /// Drop the per-request metadata once the fetch has reached a terminal state.
    pub fn forget_request(&self, req_id: crate::engine::types::RequestId) {
        self.request_meta.remove(&req_id);
    }

    /// Intern an engine reference, returning the stable sonar-side tag for it.
    pub fn to_net(&self, reference: RequestReference) -> gosub_sonar::RequestReference {
        let id = match self.forward.entry(reference) {
            Entry::Occupied(e) => *e.get(),
            Entry::Vacant(v) => {
                let id = self.next.fetch_add(1, Ordering::Relaxed);
                self.reverse.insert(id, reference);
                v.insert(id);
                id
            }
        };
        gosub_sonar::RequestReference::Tagged(id)
    }

    /// Resolve a sonar-side tag back to the engine reference it was interned from.
    pub fn from_net(&self, reference: gosub_sonar::RequestReference) -> Option<RequestReference> {
        match reference {
            gosub_sonar::RequestReference::Tagged(id) => self.reverse.get(&id).map(|r| *r),
            gosub_sonar::RequestReference::Background(id) => Some(RequestReference::Background(id)),
        }
    }
}

pub type RequestReferenceMap = HashMap<RequestReference, TabId>;

#[derive(Default)]
pub struct RequestRefTracker {
    inner: DashMap<RequestReference, (AtomicUsize, AtomicBool)>,
}

impl RequestRefTracker {
    pub fn new() -> Self {
        Self { inner: DashMap::new() }
    }

    pub fn inc(&self, r: &RequestReference) {
        match self.inner.entry(*r) {
            Entry::Occupied(e) => {
                e.get().0.fetch_add(1, Ordering::Relaxed);
            }
            Entry::Vacant(v) => {
                v.insert((AtomicUsize::new(1), AtomicBool::new(false)));
            }
        };
    }

    pub fn dec_and_maybe_cleanup(&self, r: &RequestReference, map: &Arc<RwLock<RequestReferenceMap>>) {
        let Some(entry) = self.inner.get(r) else {
            return;
        };
        // fetch_sub returns the value *before* the decrement.
        let prev = entry.0.fetch_sub(1, Ordering::Relaxed);
        let fin = entry.1.load(Ordering::Relaxed);
        // Release the DashMap read guard before removing, or `remove` deadlocks.
        drop(entry);

        if prev <= 1 && fin {
            map.write().remove(r);
            self.inner.remove(r);
        }
    }

    pub fn finalize(&self, r: &RequestReference, map: &Arc<RwLock<RequestReferenceMap>>) {
        let Some(entry) = self.inner.get(r) else {
            map.write().remove(r);
            return;
        };
        entry.1.store(true, Ordering::Relaxed);

        let now = entry.0.load(Ordering::Relaxed);
        // Release the DashMap read guard before removing, or `remove` deadlocks.
        drop(entry);

        if now == 0 {
            map.write().remove(r);
            self.inner.remove(r);
        }
    }
}
