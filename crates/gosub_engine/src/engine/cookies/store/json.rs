//! JSON-backed cookie store.
//!
//! `JsonCookieStore` persists all zones' cookie jars in a single JSON file; jars are
//! cached in memory and returned wrapped in
//! [`PersistentCookieJar`](crate::cookies::PersistentCookieJar), so every jar mutation
//! snapshots back to this store.
//!
//! `persist_zone_from_snapshot` and `remove_zone` read then rewrite the entire JSON
//! file; for large datasets use the SQLite store. Writes go to a temp file renamed
//! over the target (atomic on POSIX filesystems). Persistence is best-effort: I/O and
//! serialization errors are logged, never panicked on. An unreadable or malformed
//! existing file is preserved for recovery; a missing file is recreated on the next write.
//!
//! ```ignore,no_run
//! let store = JsonCookieStore::new("cookies.json".into())?;
//!
//! // New zones will receive a PersistentCookieJar minted by this store.
//! let zone_id = engine.zone().cookie_store(store).create()?;
//! ```
use parking_lot::{Mutex, RwLock};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use crate::engine::cookies::cookie_jar::DefaultCookieJar;
use crate::engine::cookies::store::CookieStore;
use crate::engine::cookies::{CookieJarHandle, CookieStoreHandle};
use crate::engine::zone::ZoneId;
use crate::EngineError;
use serde::{Deserialize, Serialize};

/// On-disk representation of all zones' cookie jars: the JSON payload at `JsonCookieStore::path`.
#[derive(Debug, Serialize, Deserialize)]
struct CookieStoreFile {
    zones: HashMap<ZoneId, DefaultCookieJar>,
}

/// A JSON-based cookie store that persists cookies across sessions.
pub struct JsonCookieStore {
    path: PathBuf,

    jars: RwLock<HashMap<ZoneId, CookieJarHandle>>,

    /// Held across every read-modify-write of the file: each one rewrites all
    /// zones, so two at once would drop the other's zone (and share the temp
    /// file). Taken after `jars`, never before.
    file: Mutex<()>,

    /// Self handle, so `PersistentCookieJar` can call back into this store.
    /// Initialized in [`new`](Self::new) and read-only thereafter.
    store_self: RwLock<Option<CookieStoreHandle>>,
}

impl JsonCookieStore {
    /// Creates (or opens) a JSON cookie store at `path`.
    ///
    /// If the file does not exist, an empty structure is written to disk.
    ///
    /// # Errors
    /// Returns [`EngineError::CookieStore`] if checking for an existing file or
    /// writing the initial empty file fails.
    pub fn new(path: PathBuf) -> Result<Arc<Self>, EngineError> {
        if let Some(parent) = path.parent() {
            let _ = crate::storage::private_dir(parent);
        }
        if !path.try_exists().map_err(|e| EngineError::CookieStore(e.into()))? {
            let empty = CookieStoreFile { zones: HashMap::new() };
            let bytes = serde_json::to_vec(&empty).map_err(|e| EngineError::CookieStore(e.into()))?;
            write_private(&path, &bytes).map_err(|e| EngineError::CookieStore(e.into()))?;
        }

        let store = Arc::new(Self {
            path,
            jars: RwLock::new(HashMap::new()),
            file: Mutex::new(()),
            store_self: RwLock::new(None),
        });

        *store.store_self.write() = Some(CookieStoreHandle::from(store.clone()));
        Ok(store)
    }

    /// Loads and deserializes the full cookie store file.
    ///
    /// A missing file starts an empty store so persistence can recreate it. Other read or
    /// deserialization failures are logged and return `None`; callers must not replace an
    /// unreadable existing file with a partial store.
    fn load_file(&self) -> Option<CookieStoreFile> {
        let contents = match fs::read_to_string(&self.path) {
            Ok(contents) => contents,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Some(CookieStoreFile { zones: HashMap::new() });
            }
            Err(e) => {
                log::error!("Failed to read cookie store file {:?}: {e}", self.path);
                return None;
            }
        };

        serde_json::from_str(&contents)
            .map_err(|e| {
                log::error!("Failed to parse cookie store file {:?}: {e}", self.path);
            })
            .ok()
    }

    /// Serializes and writes the full cookie store file (pretty-printed).
    ///
    /// Best-effort: serialization or I/O errors are logged and the write is skipped.
    fn save_file(&self, store_file: &CookieStoreFile) {
        let contents = match serde_json::to_vec_pretty(store_file) {
            Ok(contents) => contents,
            Err(e) => {
                log::error!("Failed to serialize cookies: {e}");
                return;
            }
        };
        // atomic-ish: write to tmp then rename
        let tmp = self.path.with_extension("json.tmp");
        if let Err(e) = write_private(&tmp, &contents) {
            log::error!("Failed to write temp cookie store file {tmp:?}: {e}");
            return;
        }
        if let Err(e) = fs::rename(&tmp, &self.path) {
            log::error!("Failed to replace cookie store file {:?}: {e}", self.path);
        }
    }
}

impl CookieStore for JsonCookieStore {
    /// Returns `None` only on the defensive internal-error path (uninitialized
    /// `store_self`); for a healthy store this always provisions a jar.
    fn jar_for(&self, zone_id: ZoneId) -> Option<CookieJarHandle> {
        crate::cookies::store::provision_persistent_jar(&self.jars, &self.store_self, zone_id, || {
            self.load_file()
                .and_then(|mut file| file.zones.remove(&zone_id))
                .unwrap_or_default()
        })
    }

    /// Reads the current file, replaces the zone entry, and writes the file back.
    fn persist_zone_from_snapshot(&self, zone_id: ZoneId, snapshot: &DefaultCookieJar) {
        let _file = self.file.lock();
        let Some(mut store_file) = self.load_file() else {
            return;
        };
        store_file.zones.insert(zone_id, snapshot.clone());
        self.save_file(&store_file);
    }

    /// Persists a final snapshot for `zone_id` and evicts its cached jar; on-disk data stays.
    fn release_zone(&self, zone_id: ZoneId) {
        if let Some(snapshot) = crate::cookies::store::evict_and_snapshot(&self.jars, zone_id) {
            self.persist_zone_from_snapshot(zone_id, &snapshot);
        }
    }

    /// Removes `zone_id` from both the in-memory cache and the on-disk file (best-effort).
    fn remove_zone(&self, zone_id: ZoneId) {
        self.jars.write().remove(&zone_id);

        let _file = self.file.lock();
        let Some(mut file) = self.load_file() else {
            return;
        };
        file.zones.remove(&zone_id);
        self.save_file(&file);
    }

    /// Only jars of type [`PersistentCookieJar`](crate::cookies::PersistentCookieJar)
    /// wrapping a [`DefaultCookieJar`] are snapshotted, keeping the format stable.
    fn persist_all(&self) {
        let jars = self.jars.read();

        // Every cached jar is read-locked before the file and held through the
        // write. A change then cannot land between its jar's snapshot and the
        // write (which would put the stale snapshot back), and the order is the
        // one a jar saving itself uses - jar, then file - so neither waits on
        // the other. In zone order, so two flushes cannot cross either.
        let mut cached: Vec<_> = jars.iter().collect();
        cached.sort_by_key(|(zone_id, _)| zone_id.to_string());
        let held: Vec<_> = cached
            .into_iter()
            .map(|(zone_id, jar)| (*zone_id, jar.read()))
            .collect();

        let _file = self.file.lock();
        let Some(mut file) = self.load_file() else {
            return;
        };
        for (zone_id, jar) in &held {
            if let Some(snapshot) = crate::cookies::store::persisted_snapshot(&***jar) {
                file.zones.insert(*zone_id, snapshot);
            }
        }
        self.save_file(&file);
    }
}

/// Write `bytes` to `path` readable by this user alone (`0600`): cookies are
/// credentials, and the profile directory may be traversable.
fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options.open(path)?.write_all(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::cookies::persistent_cookie_jar::PersistentCookieJar;
    use http::HeaderMap;
    use std::fs::File;
    use std::io::Read;
    use tempfile::tempdir;
    use url::Url;

    fn mk_headers(set_cookie_lines: &[&str]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for sc in set_cookie_lines {
            // `append` keeps repeated Set-Cookie headers as separate values.
            h.append(http::header::SET_COOKIE, (*sc).parse().unwrap());
        }
        h
    }

    type Mutation = fn(&JsonCookieStore, ZoneId);

    fn saving_mutations() -> [(&'static str, Mutation); 4] {
        [
            ("snapshot", |store, zone| {
                let url = Url::parse("https://example.com/").unwrap();
                store.jar_for(zone).unwrap().write().store_response_cookies(
                    &url,
                    &mk_headers(&["id=changed; Path=/"]),
                    None,
                );
            }),
            ("remove", JsonCookieStore::remove_zone),
            ("flush", |store, _| store.persist_all()),
            ("release", JsonCookieStore::release_zone),
        ]
    }

    /// Every saving path must preserve the original bytes when loading the
    /// existing store fails, including when a jar saves itself on mutation.
    #[test]
    fn failed_loads_do_not_overwrite_the_store() {
        // A malformed JSON suffix exercises parsing; an invalid UTF-8 byte
        // makes read_to_string fail before deserialization.
        for suffix in [b"\n{".as_slice(), b"\xff".as_slice()] {
            for (name, mutate) in saving_mutations() {
                let dir = tempdir().unwrap();
                let path = dir.path().join("cookies.json");
                let store = JsonCookieStore::new(path.clone()).unwrap();
                let zone = ZoneId::new();
                let other_zone = ZoneId::new();
                let _ = store.jar_for(zone).unwrap();
                store.persist_zone_from_snapshot(other_zone, &DefaultCookieJar::new());
                store.persist_all();

                let mut damaged = fs::read(&path).unwrap();
                damaged.extend_from_slice(suffix);
                fs::write(&path, &damaged).unwrap();

                mutate(&store, zone);

                assert_eq!(
                    fs::read(&path).unwrap(),
                    damaged,
                    "{name} replaced the unreadable store"
                );
                assert!(!path.with_extension("json.tmp").exists(), "{name} attempted a write");
            }
        }
    }

    #[test]
    fn a_removed_or_moved_store_is_recreated() {
        for move_aside in [false, true] {
            for (name, mutate) in saving_mutations() {
                let dir = tempdir().unwrap();
                let path = dir.path().join("cookies.json");
                let backup = dir.path().join("cookies.backup");
                let store = JsonCookieStore::new(path.clone()).unwrap();
                let zone = ZoneId::new();
                let url = Url::parse("https://example.com/").unwrap();
                store.jar_for(zone).unwrap().write().store_response_cookies(
                    &url,
                    &mk_headers(&["id=original; Path=/"]),
                    None,
                );
                let original = fs::read(&path).unwrap();
                if move_aside {
                    fs::rename(&path, &backup).unwrap();
                } else {
                    fs::remove_file(&path).unwrap();
                }

                mutate(&store, zone);

                assert!(path.is_file(), "{name} did not recreate the missing store");
                let saved: CookieStoreFile = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
                if name == "remove" {
                    assert!(saved.zones.is_empty());
                } else {
                    assert_eq!(saved.zones.len(), 1);
                    assert!(!saved.zones[&zone].entries.is_empty(), "{name} lost the cached cookies");
                }
                if move_aside {
                    assert_eq!(fs::read(&backup).unwrap(), original);
                }
            }
        }
    }

    #[test]
    fn other_read_errors_do_not_attempt_a_write() {
        for (name, mutate) in saving_mutations() {
            let dir = tempdir().unwrap();
            let path = dir.path().join("cookies.json");
            let store = JsonCookieStore::new(path.clone()).unwrap();
            let zone = ZoneId::new();
            let _ = store.jar_for(zone).unwrap();
            fs::remove_file(&path).unwrap();
            fs::create_dir(&path).unwrap();
            assert_ne!(
                fs::read_to_string(&path).unwrap_err().kind(),
                std::io::ErrorKind::NotFound
            );

            mutate(&store, zone);

            assert!(path.is_dir(), "{name} replaced the unreadable path");
            assert!(!path.with_extension("json.tmp").exists(), "{name} attempted a write");
        }
    }

    /// Creating a new file still works, and a later store must keep zones that
    /// it has not loaded into its own cache when it persists a new zone.
    #[test]
    fn a_new_store_preserves_uncached_zones() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("cookies.json");
        assert!(!path.exists());
        let first = JsonCookieStore::new(path.clone()).unwrap();
        let empty: CookieStoreFile = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert!(empty.zones.is_empty());

        let old_zone = ZoneId::new();
        first.persist_zone_from_snapshot(old_zone, &DefaultCookieJar::new());

        let reopened = JsonCookieStore::new(path.clone()).unwrap();
        let new_zone = ZoneId::new();
        let url = Url::parse("https://example.com/").unwrap();
        reopened.jar_for(new_zone).unwrap().write().store_response_cookies(
            &url,
            &mk_headers(&["id=123; Path=/"]),
            None,
        );
        reopened.persist_all();

        let saved: CookieStoreFile = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert!(saved.zones.contains_key(&old_zone));
        assert!(saved.zones.contains_key(&new_zone));
        assert!(!saved.zones[&new_zone].entries.is_empty());
    }

    /// Two zones persisting at once both end up in the file: each write is a
    /// read-modify-write of every zone, and they must not interleave.
    #[test]
    fn concurrent_zone_snapshots_both_reach_the_file() {
        let dir = tempdir().unwrap();
        let store = JsonCookieStore::new(dir.path().join("cookies.json")).unwrap();
        for _ in 0..50 {
            let (a, b) = (ZoneId::new(), ZoneId::new());
            let snapshot = DefaultCookieJar::new();
            std::thread::scope(|s| {
                for zone in [a, b] {
                    let (store, snapshot) = (&store, &snapshot);
                    s.spawn(move || store.persist_zone_from_snapshot(zone, snapshot));
                }
            });
            let zones = store.load_file().unwrap().zones;
            assert!(
                zones.contains_key(&a) && zones.contains_key(&b),
                "a concurrent snapshot was overwritten"
            );
        }
    }

    /// A flush and a jar saving its own change, side by side, both finish: the
    /// two take the jar and file locks in the same order.
    #[test]
    fn a_flush_and_a_jar_saving_itself_do_not_deadlock() {
        let dir = tempdir().unwrap();
        let store = JsonCookieStore::new(dir.path().join("cookies.json")).unwrap();
        let jar = store.jar_for(ZoneId::new()).unwrap();
        let url = Url::parse("https://example.com/").unwrap();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let writer_done = done_tx.clone();
        std::thread::spawn(move || {
            for i in 0..200 {
                jar.write()
                    .store_response_cookies(&url, &mk_headers(&[&format!("c{i}=1; Path=/")]), None);
            }
            let _ = writer_done.send(());
        });
        let flusher = store.clone();
        std::thread::spawn(move || {
            for _ in 0..200 {
                flusher.persist_all();
            }
            let _ = done_tx.send(());
        });
        for _ in 0..2 {
            done_rx
                .recv_timeout(std::time::Duration::from_secs(20))
                .expect("a flush and a jar's own save deadlocked");
        }
    }

    #[test]
    fn jar_for_memoizes_and_wraps_persistent() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("cookies.json");
        let store = JsonCookieStore::new(path).unwrap();

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
        let store = JsonCookieStore::new(path.clone()).unwrap();

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
            inner.store_response_cookies(&url, &headers, None);
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
        let store2 = JsonCookieStore::new(path.clone()).unwrap();
        let h2 = store2.jar_for(zone).unwrap();

        // Ensure it’s again a persistent wrapper
        assert!(h2.read().as_any().downcast_ref::<PersistentCookieJar>().is_some());
    }

    #[test]
    fn remove_zone_evicts_cache_and_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("cookies.json");
        let store = JsonCookieStore::new(path.clone()).unwrap();

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

    /// A file holding more than a jar may (an older build, another tool, a
    /// planted file) loads as a jar within its limits, so the snapshot the
    /// cookie vault is opened with fits its frame.
    #[test]
    fn an_oversized_file_loads_within_the_jars_limits() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("cookies.json");
        let zone = ZoneId::new();
        let mut jar = DefaultCookieJar::new();
        let value = "v".repeat(4000);
        // 5000 cookies of 4 KB: some 20 MB, past the vault's 16 MiB frame.
        for i in 0..5_000 {
            jar.entries.insert(
                format!("https://o{i}.test"),
                vec![crate::engine::cookies::Cookie {
                    name: "c".into(),
                    value: value.clone(),
                    path: Some("/".into()),
                    domain: None,
                    secure: false,
                    expires: None,
                    same_site: None,
                    http_only: false,
                    created_at: i,
                }],
            );
        }
        let file = CookieStoreFile {
            zones: HashMap::from([(zone, jar)]),
        };
        std::fs::write(&path, serde_json::to_vec(&file).unwrap()).unwrap();

        let store = JsonCookieStore::new(path).unwrap();
        let handle = store.jar_for(zone).unwrap();
        let binding = handle.read();
        let persist = binding
            .as_any()
            .downcast_ref::<PersistentCookieJar>()
            .expect("persistent wrapper expected");
        let inner = persist.inner.read();
        let loaded = inner
            .as_any()
            .downcast_ref::<DefaultCookieJar>()
            .expect("a default jar inside");
        // JSON is larger than the vault's wire form, so under the frame cap here
        // is under it there.
        let snapshot = serde_json::to_vec(loaded).unwrap();
        assert!(snapshot.len() < 16 * 1024 * 1024, "{} byte snapshot", snapshot.len());
        assert!(!loaded.entries.is_empty(), "the newest are kept");
    }
}
