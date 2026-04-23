//! JSON-backed cookie store.
//!
//! `JsonCookieStore` persists **all zones'** cookie jars in a single JSON file on disk.
//! It implements the [`CookieStore`] trait and returns per-zone jars wrapped in
//! [`PersistentCookieJar`], so that **every mutation** to a jar triggers a snapshot
//! write back to this store.
//!
//! ### Design
//! - One file for all zones (`CookieStoreFile { zones: HashMap<ZoneId, DefaultCookieJar> }`).
//! - In-memory cache: `jars: RwLock<HashMap<ZoneId, CookieJarHandle>>` for quick reuse.
//! - The store keeps a self handle (`store_self`) so the persistent jars can call
//!   back into `persist_zone_from_snapshot`.
//!
//! ### Concurrency
//! - This type is internally synchronized via `RwLock`s and is `Send + Sync` behind
//!   a `CookieStoreHandle = Arc<dyn CookieStore + Send + Sync>`.
//! - Returned jars are `Arc<RwLock<_>>` and safe to share across threads.
//!
//! ### I/O characteristics & caveats
//! - `persist_zone_from_snapshot` and `remove_zone` **read then rewrite** the entire
//!   JSON file. For large datasets, consider an SQLite-backed store.
//! - File writes are not atomic.
//! - Several helpers use `expect(...)` and will **panic** on I/O/serialization errors.
//!
//! ### Example
//! ```ignore,no_run
//! let store = JsonCookieStore::new("cookies.json".into());
//!
//! // New zones will receive a PersistentCookieJar minted by this store.
//! let zone_id = engine.zone().cookie_store(store).create()?;
//! ```
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Read;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use crate::engine::cookies::cookie_jar::DefaultCookieJar;
use crate::engine::cookies::persistent_cookie_jar::PersistentCookieJar;
use crate::engine::cookies::store::CookieStore;
use crate::engine::cookies::{CookieJarHandle, CookieStoreHandle};
use crate::engine::zone::ZoneId;
use serde::{Deserialize, Serialize};

/// On-disk representation of all zones' cookie jars.
///
/// This is the JSON payload stored at `JsonCookieStore::path`.
#[derive(Debug, Serialize, Deserialize)]
struct CookieStoreFile {
    zones: HashMap<ZoneId, DefaultCookieJar>,
}

/// A JSON-based cookie store that persists cookies across sessions.
///
/// The store caches per-zone jars in memory and loads/saves them to a single JSON file.
/// Jars returned by this store are wrapped in [`PersistentCookieJar`], so that writes
/// automatically trigger persistence to disk.
pub struct JsonCookieStore {
    /// Path to the JSON file where cookies are stored.
    path: PathBuf,

    /// Actual list of cookie jars per zone
    jars: RwLock<HashMap<ZoneId, CookieJarHandle>>,

    /// Self handle, so `PersistentCookieJar` can call back into this store.
    ///
    /// This is initialized in [`new`](Self::new) and then read-only thereafter.
    store_self: RwLock<Option<CookieStoreHandle>>,
}

impl JsonCookieStore {
    /// Creates (or opens) a JSON cookie store at `path`.
    ///
    /// If the file does not exist, an empty structure is written to disk.
    ///
    /// # Panics
    /// Panics if the initial write of an empty file fails.
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

    /// Loads and deserializes the full cookie store file.
    ///
    /// Returns an empty structure if deserialization fails.
    ///
    /// # Panics
    /// Panics if the file cannot be opened or read.
    fn load_file(&self) -> CookieStoreFile {
        let mut file = File::open(&self.path).expect("Failed to open cookie store file");
        let mut contents = String::new();
        file.read_to_string(&mut contents)
            .expect("Failed to read cookie store file");

        serde_json::from_str(&contents).unwrap_or_else(|_| CookieStoreFile { zones: HashMap::new() })
    }

    /// Serializes and writes the full cookie store file (pretty-printed).
    ///
    /// # Panics
    /// Panics if serialization or writing fails.
    fn save_file(&self, store_file: &CookieStoreFile) {
        let contents = serde_json::to_vec_pretty(store_file).expect("Failed to serialize cookies");
        // atomic-ish: write to tmp then rename
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, &contents).expect("Failed to write temp cookie store file");
        fs::rename(&tmp, &self.path).expect("Failed to replace cookie store file");
    }
}

impl CookieStore for JsonCookieStore {
    /// Returns the cookie jar handle for `zone_id`, creating it if needed.
    ///
    /// Behavior:
    /// - If a jar for `zone_id` exists in the in-memory cache, it is returned.
    /// - Otherwise, a serialized jar is loaded from disk (if present) or an empty
    ///   [`DefaultCookieJar`] is created.
    /// - That jar is wrapped in a [`PersistentCookieJar`] bound to this store
    ///   (via `store_self`) so that subsequent mutations persist automatically.
    ///
    /// Always returns `Some(_)` for valid inputs; `None` is reserved for stores
    /// that may intentionally refuse provisioning.
    fn jar_for(&self, zone_id: ZoneId) -> Option<CookieJarHandle> {
        // Fast path: already in memory
        if let Some(jar) = self.jars.read().unwrap().get(&zone_id) {
            return Some(jar.clone());
        }

        // load from disk (or empty)
        let mut file = self.load_file();
        let jar = file.zones.remove(&zone_id).unwrap_or_default();
        let arc_jar: CookieJarHandle = jar.into(); // assuming you have From<DefaultCookieJar> for CookieJarHandle

        let store = self
            .store_self
            .read()
            .unwrap()
            .as_ref()
            .expect("store_self not initialized")
            .clone();

        // Wrap in PersistentCookieJar and then into a CookieJarHandle
        let persistent = PersistentCookieJar::new(zone_id, arc_jar.clone(), store);
        let handle = CookieJarHandle::new(persistent);

        self.jars.write().unwrap().insert(zone_id, handle.clone());
        Some(handle)
    }

    /// Persists a snapshot of `zone_id`'s jar to disk.
    ///
    /// Called by [`PersistentCookieJar`] after each mutation. This method reads
    /// the current file, updates/replaces the zone entry, and writes the file back.
    ///
    /// # Panics
    /// Panics on I/O/serialization errors.
    fn persist_zone_from_snapshot(&self, zone_id: ZoneId, snapshot: &DefaultCookieJar) {
        let mut store_file = self.load_file();
        store_file.zones.insert(zone_id, snapshot.clone());
        self.save_file(&store_file);
    }

    /// Removes `zone_id` from both the in-memory cache and the on-disk file.
    ///
    /// # Panics
    /// Panics on I/O/serialization errors while updating the file.
    fn remove_zone(&self, zone_id: ZoneId) {
        self.jars.write().unwrap().remove(&zone_id);

        let mut file = self.load_file();
        file.zones.remove(&zone_id);
        self.save_file(&file);
    }

    /// Persists **all** in-memory jars to disk by snapshotting them.
    ///
    /// Only jars of type [`PersistentCookieJar`] that wrap a [`DefaultCookieJar`]
    /// are snapshotted here. This avoids double-wrapping and keeps the format stable.
    ///
    /// # Panics
    /// Panics on I/O/serialization errors while writing the file.
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
            // multiple Set-Cookie lines are allowed by repeated headers;
            // but HeaderMap overwrites by default; if your jar’s API accepts a single combined header
            // you can join them. For this smoke test, one is enough.
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

        // Downcast to persistent wrapper to ensure it’s wrapped
        assert!(a.read().as_any().downcast_ref::<PersistentCookieJar>().is_some());
    }

    #[test]
    fn persist_all_writes_file_and_reload_restores_jar() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("cookies.json");
        let store = JsonCookieStore::new(path.clone());

        let zone = ZoneId::new();
        let handle = store.jar_for(zone).unwrap();

        // write a cookie via the inner jar
        {
            let binding = handle.read();
            let persist = binding
                .as_any()
                .downcast_ref::<PersistentCookieJar>()
                .expect("persistent wrapper expected");
            let mut inner = persist.inner.write(); // inner: Arc<RwLock<DefaultCookieJar>>

            let url: Url = "https://example.com/".parse().unwrap();
            let headers = mk_headers(&["id=123; Path=/; HttpOnly"]);
            inner.store_response_cookies(&url, &headers);
        }

        // snapshot everything
        store.persist_all();

        // Verify on-disk file has the zone entry
        let mut f = File::open(&path).unwrap();
        let mut s = String::new();
        f.read_to_string(&mut s).unwrap();
        let parsed: CookieStoreFile = serde_json::from_str(&s).unwrap();
        assert!(
            parsed.zones.contains_key(&zone),
            "zone entry must exist after persist_all"
        );

        // New store instance should load the jar from disk
        let store2 = JsonCookieStore::new(path.clone());
        let h2 = store2.jar_for(zone).unwrap();

        // Ensure it’s again a persistent wrapper
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

        // Persist both
        store.persist_all();

        // Remove z1
        store.remove_zone(z1);

        // File should not contain z1 anymore
        let mut s = String::new();
        File::open(&path).unwrap().read_to_string(&mut s).unwrap();
        let parsed: CookieStoreFile = serde_json::from_str(&s).unwrap();
        assert!(!parsed.zones.contains_key(&z1));
        assert!(parsed.zones.contains_key(&z2));

        // Asking again should create a fresh jar for z1 (and persistable)
        let _ = store.jar_for(z1).unwrap();
        store.persist_all();
        let mut s2 = String::new();
        File::open(&path).unwrap().read_to_string(&mut s2).unwrap();
        let parsed2: CookieStoreFile = serde_json::from_str(&s2).unwrap();
        assert!(parsed2.zones.contains_key(&z1));
    }
}
