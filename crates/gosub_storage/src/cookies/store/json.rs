use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Read;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use crate::cookies::cookie_jar::DefaultCookieJar;
use crate::cookies::persistent_cookie_jar::PersistentCookieJar;
use crate::cookies::store::CookieStore;
use crate::cookies::{CookieJarHandle, CookieStoreHandle};
use gosub_net::types::ZoneId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
struct CookieStoreFile {
    zones: HashMap<ZoneId, DefaultCookieJar>,
}

pub struct JsonCookieStore {
    path: PathBuf,
    jars: RwLock<HashMap<ZoneId, CookieJarHandle>>,
    store_self: RwLock<Option<CookieStoreHandle>>,
}

impl JsonCookieStore {
    pub fn new(path: PathBuf) -> Arc<Self> {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if !path.exists() {
            let empty = CookieStoreFile { zones: HashMap::new() };
            fs::write(&path, serde_json::to_vec(&empty).unwrap()).expect("Failed to create cookie store file");
        }

        let store = Arc::new(Self {
            path,
            jars: RwLock::new(HashMap::new()),
            store_self: RwLock::new(None),
        });

        *store.store_self.write().unwrap() = Some(CookieStoreHandle::from(store.clone()));
        store
    }

    fn load_file(&self) -> CookieStoreFile {
        let mut file = File::open(&self.path).expect("Failed to open cookie store file");
        let mut contents = String::new();
        file.read_to_string(&mut contents)
            .expect("Failed to read cookie store file");
        serde_json::from_str(&contents).unwrap_or_else(|_| CookieStoreFile { zones: HashMap::new() })
    }

    fn save_file(&self, store_file: &CookieStoreFile) {
        let contents = serde_json::to_vec_pretty(store_file).expect("Failed to serialize cookies");
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, &contents).expect("Failed to write temp cookie store file");
        fs::rename(&tmp, &self.path).expect("Failed to replace cookie store file");
    }
}

impl CookieStore for JsonCookieStore {
    fn jar_for(&self, zone_id: ZoneId) -> Option<CookieJarHandle> {
        if let Some(jar) = self.jars.read().unwrap().get(&zone_id) {
            return Some(jar.clone());
        }

        let mut file = self.load_file();
        let jar = file.zones.remove(&zone_id).unwrap_or_default();
        let arc_jar: CookieJarHandle = jar.into();

        let store = self
            .store_self
            .read()
            .unwrap()
            .as_ref()
            .expect("store_self not initialized")
            .clone();

        let persistent = PersistentCookieJar::new(zone_id, arc_jar.clone(), store);
        let handle = CookieJarHandle::new(persistent);

        self.jars.write().unwrap().insert(zone_id, handle.clone());
        Some(handle)
    }

    fn persist_zone_from_snapshot(&self, zone_id: ZoneId, snapshot: &DefaultCookieJar) {
        let mut store_file = self.load_file();
        store_file.zones.insert(zone_id, snapshot.clone());
        self.save_file(&store_file);
    }

    fn remove_zone(&self, zone_id: ZoneId) {
        self.jars.write().unwrap().remove(&zone_id);
        let mut file = self.load_file();
        file.zones.remove(&zone_id);
        self.save_file(&file);
    }

    fn persist_all(&self) {
        let jars = self.jars.read().unwrap();
        let mut file = self.load_file();
        for (zone_id, jar_handle) in jars.iter() {
            let jar = jar_handle.read();
            if let Some(persist) = jar.as_any().downcast_ref::<PersistentCookieJar>() {
                let inner = persist.inner.read();
                if let Some(default) = inner.as_any().downcast_ref::<DefaultCookieJar>() {
                    file.zones.insert(*zone_id, default.clone());
                }
            }
        }
        self.save_file(&file);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::HeaderMap;
    use tempfile::tempdir;
    use url::Url;

    fn mk_headers(set_cookie_lines: &[&str]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for sc in set_cookie_lines {
            h.append(http::header::SET_COOKIE, (*sc).parse().unwrap());
        }
        h
    }

    #[test]
    fn jar_for_memoizes_and_wraps_persistent() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("cookies.json");
        let store = JsonCookieStore::new(path);

        let z = ZoneId::new();
        let a = store.jar_for(z).unwrap();
        let b = store.jar_for(z).unwrap();
        assert!(CookieJarHandle::ptr_eq(&a, &b), "same zone should return same Arc");

        assert!(a.read().as_any().downcast_ref::<PersistentCookieJar>().is_some());
    }

    #[test]
    fn persist_all_writes_file_and_reload_restores_jar() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("cookies.json");
        let store = JsonCookieStore::new(path.clone());

        let zone = ZoneId::new();
        let handle = store.jar_for(zone).unwrap();

        {
            let binding = handle.read();
            let persist = binding
                .as_any()
                .downcast_ref::<PersistentCookieJar>()
                .expect("persistent wrapper expected");
            let mut inner = persist.inner.write();
            let url: Url = "https://example.com/".parse().unwrap();
            let headers = mk_headers(&["id=123; Path=/; HttpOnly"]);
            inner.store_response_cookies(&url, &headers);
        }

        store.persist_all();

        let mut f = File::open(&path).unwrap();
        let mut s = String::new();
        f.read_to_string(&mut s).unwrap();
        let parsed: CookieStoreFile = serde_json::from_str(&s).unwrap();
        assert!(
            parsed.zones.contains_key(&zone),
            "zone entry must exist after persist_all"
        );

        let store2 = JsonCookieStore::new(path.clone());
        let h2 = store2.jar_for(zone).unwrap();
        assert!(h2.read().as_any().downcast_ref::<PersistentCookieJar>().is_some());
    }

    #[test]
    fn remove_zone_evicts_cache_and_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("cookies.json");
        let store = JsonCookieStore::new(path.clone());

        let z1 = ZoneId::new();
        let z2 = ZoneId::new();

        let _ = store.jar_for(z1).unwrap();
        let _ = store.jar_for(z2).unwrap();
        store.persist_all();

        store.remove_zone(z1);

        let mut s = String::new();
        File::open(&path).unwrap().read_to_string(&mut s).unwrap();
        let parsed: CookieStoreFile = serde_json::from_str(&s).unwrap();
        assert!(!parsed.zones.contains_key(&z1));
        assert!(parsed.zones.contains_key(&z2));

        let _ = store.jar_for(z1).unwrap();
        store.persist_all();
        let mut s2 = String::new();
        File::open(&path).unwrap().read_to_string(&mut s2).unwrap();
        let parsed2: CookieStoreFile = serde_json::from_str(&s2).unwrap();
        assert!(parsed2.zones.contains_key(&z1));
    }
}
