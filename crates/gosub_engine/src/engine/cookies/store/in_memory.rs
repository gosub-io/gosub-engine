use parking_lot::RwLock;
use std::collections::HashMap;

use crate::engine::cookies::cookie_jar::DefaultCookieJar;
use crate::engine::cookies::store::CookieStore;
use crate::engine::cookies::CookieJarHandle;
use crate::engine::zone::ZoneId;

/// Represents a cookie store that keeps all the jars in memory. They jars are not persisted once
/// the store is dropped.
pub struct InMemoryCookieStore {
    /// Cookie jars per zone
    jars: RwLock<HashMap<ZoneId, CookieJarHandle>>,
}

impl Default for InMemoryCookieStore {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryCookieStore {
    pub fn new() -> Self {
        Self {
            jars: RwLock::new(HashMap::new()),
        }
    }
}

impl CookieStore for InMemoryCookieStore {
    fn jar_for(&self, zone_id: ZoneId) -> Option<CookieJarHandle> {
        use std::collections::hash_map::Entry;

        let mut jars = self.jars.write();
        let handle = match jars.entry(zone_id) {
            Entry::Occupied(o) => o.get().clone(),
            Entry::Vacant(v) => {
                let jar_handle: CookieJarHandle = DefaultCookieJar::new().into();
                v.insert(jar_handle.clone());
                jar_handle
            }
        };
        Some(handle)
    }

    fn persist_zone_from_snapshot(&self, _zone_id: ZoneId, _snapshot: &DefaultCookieJar) {}

    fn remove_zone(&self, zone_id: ZoneId) {
        self.jars.write().remove(&zone_id);
    }

    fn persist_all(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_zone_returns_same_handle() {
        let store = InMemoryCookieStore::new();
        let z = ZoneId::new();

        let a = store.jar_for(z).unwrap();
        let b = store.jar_for(z).unwrap();

        // Same Arc target
        assert!(CookieJarHandle::ptr_eq(&a, &b));

        // Can write a cookie and read it back via the other handle
        {
            a.write()
                .store_response_cookies(&"https://example.com/".parse().unwrap(), &http::HeaderMap::new());
            // (No actual Set-Cookie here; we’re just ensuring it is writable without panicking)
        }
        // second handle should still be valid
        drop(b.read());
    }

    #[test]
    fn different_zones_get_different_handles() {
        let store = InMemoryCookieStore::new();
        let z1 = ZoneId::new();
        let z2 = ZoneId::new();

        let a = store.jar_for(z1).unwrap();
        let b = store.jar_for(z2).unwrap();

        assert!(!CookieJarHandle::ptr_eq(&a, &b));
    }

    #[test]
    fn remove_zone_drops_only_that_zone() {
        let store = InMemoryCookieStore::new();
        let z1 = ZoneId::new();
        let z2 = ZoneId::new();

        let a = store.jar_for(z1).unwrap();
        let _b = store.jar_for(z2).unwrap();

        store.remove_zone(z1);

        // z1 should allocate a fresh jar now
        let a2 = store.jar_for(z1).unwrap();
        assert!(!CookieJarHandle::ptr_eq(&a, &a2));
    }
}
