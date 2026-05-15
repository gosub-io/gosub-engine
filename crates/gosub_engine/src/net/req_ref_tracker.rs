pub use gosub_net::net::request_ref::RequestReference;

use crate::tab::TabId;
use dashmap::{DashMap, Entry};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

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
