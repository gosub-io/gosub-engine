use crate::tab::TabId;
use crate::NavigationId;
use dashmap::{DashMap, Entry};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::fmt::Display;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

pub type RequestReferenceMap = HashMap<RequestReference, TabId>;

/// Request references, indicate what initiated the request without the net functionality known
/// about its caller. This way, we can let the net module still emit events based on the request
/// reference. For instance, a request from a navigation (with a navigation_id) can emit at a
/// low level events to a tab, without the system known what a tab is. This leaves the engine
/// independent of higher level functionality.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Copy)]
pub enum RequestReference {
    /// Main doc for a tab
    Navigation(NavigationId),
    /// Sub resources of a specific doc
    Document(crate::net::types::DocumentId),
    /// Background prefetches
    Prefetch(crate::net::types::PrefetchId),
    /// Misc/system
    Background(crate::net::types::TaskId),
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
//
// /// RequestReferenceMap will map a request reference to a specific tab (for instance, to deliver the result)
// pub struct RequestReferenceMap {
//     tabs: HashMap<RequestReference, TabId>,
// }
//
// impl RequestReferenceMap {
//     pub fn new() -> Self {
//         Self { tabs: HashMap::new() }
//     }
//
//     /// Associates a request reference to a tab ID
//     pub fn insert(&mut self, reference: RequestReference, tab_id: TabId) {
//         self.tabs.insert(reference, tab_id);
//     }
//
//     /// Removes the mapping for a request reference
//     pub fn remove(&mut self, reference: &RequestReference) {
//         self.tabs.remove(reference);
//     }
//
//     /// Gets the tab ID for a request reference
//     pub fn get(&self, reference: &RequestReference) -> Option<TabId> {
//         self.tabs.get(reference).cloned()
//     }
// }

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
        if let Some(entry) = self.inner.get(r) {
            let new = entry.0.fetch_sub(1, Ordering::Relaxed);
            let fin = entry.1.load(Ordering::Relaxed);
            drop(entry);

            if new == 0 && fin {
                map.write().remove(r);
                self.inner.remove(r);
            }
        }
    }

    pub fn finalize(&self, r: &RequestReference, map: &Arc<RwLock<RequestReferenceMap>>) {
        if let Some(entry) = self.inner.get(r) {
            entry.1.store(true, Ordering::Relaxed);

            let now = entry.0.load(Ordering::Relaxed);
            drop(entry);

            if now == 0 {
                map.write().remove(r);
                self.inner.remove(r);
            }
        } else {
            map.write().remove(r);
        }
    }
}
