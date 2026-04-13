use crate::net_types::{DocumentId, PrefetchId, TaskId};
use crate::types::{NavigationId, TabId};
use dashmap::{DashMap, Entry};
use std::collections::HashMap;
use std::fmt::Display;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};

pub type RequestReferenceMap = HashMap<RequestReference, TabId>;

/// Request references, indicate what initiated the request without the net functionality knowing
/// about its caller.
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
                map.write().unwrap().remove(r);
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
                map.write().unwrap().remove(r);
                self.inner.remove(r);
            }
        } else {
            map.write().unwrap().remove(r);
        }
    }
}
