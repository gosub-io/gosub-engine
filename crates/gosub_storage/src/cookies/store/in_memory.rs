use std::collections::HashMap;
use std::sync::RwLock;

use crate::cookies::cookie_jar::DefaultCookieJar;
use crate::cookies::store::CookieStore;
use crate::cookies::CookieJarHandle;
use gosub_net::types::ZoneId;

pub struct InMemoryCookieStore {
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

        let mut jars = self.jars.write().unwrap();
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
        self.jars.write().unwrap().remove(&zone_id);
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

        assert!(CookieJarHandle::ptr_eq(&a, &b));

        {
            a.write()
                .store_response_cookies(&"https://example.com/".parse().unwrap(), &http::HeaderMap::new());
        }
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

        let a2 = store.jar_for(z1).unwrap();
        assert!(!CookieJarHandle::ptr_eq(&a, &a2));
    }
}
