//! `localStorage` as one JSON file per `(zone, partition, origin)` area.
//!
//! Plain files rather than SQLite because the storage service's filter allows
//! `openat`, the rename that replaces a file whole, and not much else (no
//! locks, no directory listing). Filenames are hex-encoded tuples, so
//! page-controlled strings never reach a path; keys live inside the file.
//! Per-value and per-area size caps, and a cap on areas kept loaded.

use crate::storage::{LocalStore, PartitionKey, StorageArea};
use crate::zone::ZoneId;
use anyhow::{anyhow, Result};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const MAX_VALUE_BYTES: usize = 5 * 1024 * 1024;
/// Per-origin quota, in the range browsers use.
pub const MAX_AREA_BYTES: usize = 10 * 1024 * 1024;
/// Areas a store keeps loaded with no handle out on them. Each holds its
/// items in memory, up to the quota; a page naming origins without end would
/// otherwise grow the service to its memory limit. One with a handle out is
/// never let go of (its handle is the live copy), and a let-go area reloads
/// from its file on next use.
pub const MAX_LOADED_AREAS: usize = 64;

/// Loaded areas by file path.
type AreaMap = Mutex<HashMap<PathBuf, Arc<FileArea>>>;
type Areas = Arc<AreaMap>;

#[derive(Debug, Clone)]
pub struct FileLocalStore {
    dir: PathBuf,
    /// Loaded areas, so handles to the same area share state (and, through
    /// [`live_area`], with every other store in the process).
    areas: Areas,
}

/// The one live [`FileArea`] of a file, process-wide, or a fresh load. Each
/// area keeps its items in memory and rewrites its file whole, so two copies
/// of one file - two stores on a directory, or a handle that outlived its
/// store and a store reopened since - would write over each other. Weak, so
/// an area goes once no store caches it and no handle holds it.
fn live_area(path: &Path) -> Result<Arc<FileArea>> {
    static LIVE: std::sync::LazyLock<Mutex<HashMap<PathBuf, std::sync::Weak<FileArea>>>> =
        std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));
    let mut live = LIVE.lock();
    if let Some(area) = live.get(path).and_then(std::sync::Weak::upgrade) {
        return Ok(area);
    }
    live.retain(|_, area| area.strong_count() > 0);
    let area = Arc::new(FileArea::load(path.to_path_buf())?);
    live.insert(path.to_path_buf(), Arc::downgrade(&area));
    Ok(area)
}

impl FileLocalStore {
    /// Creates `dir`. Only the broker can: the service has no `mkdir`. The
    /// directory is keyed by its canonical path, so two spellings of it share
    /// one set of areas.
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir)?;
        let dir = std::fs::canonicalize(&dir).unwrap_or(dir);
        Ok(Self {
            dir,
            areas: Areas::default(),
        })
    }

    /// Uses an existing `dir` without touching it - not even to canonicalize
    /// it, which the service's filter may not allow; pass the canonical path
    /// to share areas with an [`Self::open`] store on the same directory.
    pub fn attach(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            areas: Areas::default(),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Fails when the area's file exists but cannot be read or parsed; that
    /// is not cached, so the next use tries again.
    pub fn area_for(&self, zone: &str, partition: &str, origin: &str) -> Result<Arc<FileArea>> {
        let path = self.dir.join(area_file_name(zone, partition, origin));
        let mut areas = self.areas.lock();
        if let Some(area) = areas.get(&path) {
            return Ok(Arc::clone(area));
        }
        let area = live_area(&path)?;
        areas.insert(path, Arc::clone(&area));
        if areas.len() > MAX_LOADED_AREAS {
            // Only what nobody holds: a handle is the one live copy of its
            // area, and this store's clone is what shares it. The one just
            // loaded is held by `area` and stays.
            areas.retain(|_, cached| Arc::strong_count(cached) > 1);
        }
        Ok(area)
    }
}

/// Wire/file form of a `PartitionKey`. Every variant has its own prefix, so
/// no custom string can name another variant's area (`Custom("")` is not
/// `None`, `Custom("top:...")` is not a `TopLevel`).
pub fn partition_name(part: &PartitionKey) -> String {
    match part {
        PartitionKey::None => "none".to_string(),
        PartitionKey::TopLevel(origin) => format!("top:{}", origin.ascii_serialization()),
        PartitionKey::Custom(s) => format!("custom:{s}"),
    }
}

/// `<sha-256 of the tuple>.json`: a fixed 69 bytes whatever the inputs, since a
/// host alone can run to 253 bytes and a custom partition has no bound at all,
/// and a path component past `NAME_MAX` (255) cannot be created. Each field is
/// length-prefixed before hashing, so two different tuples never feed the hash
/// the same bytes.
fn area_file_name(zone: &str, partition: &str, origin: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    for field in [zone, partition, origin] {
        hasher.update((field.len() as u64).to_le_bytes());
        hasher.update(field.as_bytes());
    }
    let digest = hasher.finalize();
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("{hex}.json")
}

impl LocalStore for FileLocalStore {
    fn area(&self, zone: ZoneId, part: &PartitionKey, origin: &url::Origin) -> Result<Arc<dyn StorageArea>> {
        let area = self.area_for(&zone.to_string(), &partition_name(part), &origin.ascii_serialization())?;
        Ok(area)
    }

    fn service_directory(&self) -> Option<PathBuf> {
        Some(self.dir.clone())
    }
}

/// Items in memory, file rewritten on every change.
#[derive(Debug)]
pub struct FileArea {
    path: PathBuf,
    items: Mutex<HashMap<String, String>>,
}

impl FileArea {
    /// Only a missing file is a new, empty area. One that cannot be read or
    /// parsed is an error: handed out empty, the next write would replace it
    /// and erase whatever it held.
    fn load(path: PathBuf) -> Result<Self> {
        let items = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice::<HashMap<String, String>>(&bytes)
                .map_err(|e| anyhow!("area file {} is damaged: {e}", path.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
            Err(e) => return Err(anyhow!("area file {} cannot be read: {e}", path.display())),
        };
        Ok(Self {
            path,
            items: Mutex::new(items),
        })
    }

    /// Written beside the area and renamed over it, so a crash mid-write
    /// leaves the previous state, never a truncated file. The staged file is
    /// synced before the rename (or a power loss could leave the new name on
    /// empty contents), and the directory after it (the rename lives there).
    fn persist(&self, bytes: &[u8]) -> Result<()> {
        use std::io::Write as _;
        let staged = self.path.with_extension("json.new");
        let mut file = std::fs::File::create(&staged)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&staged, &self.path)?;
        // The new contents are in place and what reads see; a failure here
        // only leaves the rename undurable, so it is reported, not returned.
        #[cfg(unix)]
        if let Some(dir) = self.path.parent() {
            if let Err(e) = std::fs::File::open(dir).and_then(|dir| dir.sync_all()) {
                log::warn!("could not sync {}: {e}", dir.display());
            }
        }
        Ok(())
    }

    /// The area as it goes to disk, refused past the quota. Measured on the
    /// serialized bytes: JSON escapes quotes and backslashes, so the raw string
    /// lengths understate what is written.
    fn serialize_within_quota(items: &HashMap<String, String>) -> Result<Vec<u8>> {
        let bytes = serde_json::to_vec(items)?;
        if bytes.len() > MAX_AREA_BYTES {
            return Err(anyhow!("storage quota of {MAX_AREA_BYTES} bytes exceeded"));
        }
        Ok(bytes)
    }
}

impl StorageArea for FileArea {
    fn get_item(&self, key: &str) -> Option<String> {
        self.items.lock().get(key).cloned()
    }

    fn set_item(&self, key: &str, value: &str) -> Result<()> {
        if value.len() > MAX_VALUE_BYTES {
            return Err(anyhow!(
                "value of {} bytes exceeds the {MAX_VALUE_BYTES}-byte limit",
                value.len()
            ));
        }
        // Every change is undone when it cannot be written: what reads see is what
        // is on disk, so nothing a restart would lose is ever visible.
        let mut items = self.items.lock();
        let previous = items.insert(key.to_string(), value.to_string());
        let written = Self::serialize_within_quota(&items).and_then(|bytes| self.persist(&bytes));
        if written.is_err() {
            match previous {
                Some(old) => items.insert(key.to_string(), old),
                None => items.remove(key),
            };
        }
        written
    }

    fn remove_item(&self, key: &str) -> Result<()> {
        let mut items = self.items.lock();
        let Some(old) = items.remove(key) else {
            return Ok(());
        };
        let written = serde_json::to_vec(&*items)
            .map_err(Into::into)
            .and_then(|bytes| self.persist(&bytes));
        if written.is_err() {
            items.insert(key.to_string(), old);
        }
        written
    }

    fn clear(&self) -> Result<()> {
        let mut items = self.items.lock();
        let old = std::mem::take(&mut *items);
        let written = self.persist(b"{}");
        if written.is_err() {
            *items = old;
        }
        written
    }

    fn len(&self) -> usize {
        self.items.lock().len()
    }

    fn keys(&self) -> Vec<String> {
        self.items.lock().keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn origin(s: &str) -> url::Origin {
        url::Url::parse(s).expect("url").origin()
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("gosub-file-store-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// A 253-byte host is a valid origin; its area file name stays a fixed length.
    #[test]
    fn a_long_origin_still_gets_a_file() {
        let dir = scratch("long");
        let store = FileLocalStore::open(&dir).expect("open");
        let label = "a".repeat(63);
        let host = format!("{label}.{label}.{label}.test") + &".b".repeat(18);
        let area = store
            .area(ZoneId::new(), &PartitionKey::None, &origin(&format!("https://{host}")))
            .expect("area");
        area.set_item("k", "v").expect("a long origin must not fail to persist");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The quota counts what is written: escaped quotes take twice their raw size.
    #[test]
    fn the_quota_is_measured_on_the_serialized_bytes() {
        let dir = scratch("serialized-quota");
        let store = FileLocalStore::open(&dir).expect("open");
        let area = store
            .area(ZoneId::new(), &PartitionKey::None, &origin("https://q.test"))
            .expect("area");
        // Within the per-value limit raw, but twice that once each quote is escaped.
        let quotes = "\"".repeat(MAX_VALUE_BYTES);
        assert!(area.set_item("k", &quotes).is_err(), "escaped, this is past the quota");
        assert!(area.get_item("k").is_none(), "a refused value is not kept");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A write that fails leaves nothing behind in memory.
    #[test]
    fn a_failed_write_is_rolled_back() {
        let dir = scratch("rollback");
        let store = FileLocalStore::open(&dir).expect("open");
        let area = store.area_for("zone", "", "https://r.test").expect("area");
        area.set_item("kept", "1").expect("set");
        // The staging file's name taken by a directory: every later write fails.
        std::fs::create_dir(area.path.with_extension("json.new")).expect("block staging");

        assert!(area.set_item("new", "2").is_err());
        assert!(area.get_item("new").is_none());
        assert!(area.remove_item("kept").is_err());
        assert_eq!(area.get_item("kept").as_deref(), Some("1"));
        assert!(area.clear().is_err());
        assert_eq!(area.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn areas_are_isolated_and_persist_across_reopen() {
        let dir = scratch("reopen");
        let zone = ZoneId::new();
        {
            let store = FileLocalStore::open(&dir).expect("open");
            let a = store
                .area(zone, &PartitionKey::None, &origin("https://a.test"))
                .expect("area");
            let b = store
                .area(zone, &PartitionKey::None, &origin("https://b.test"))
                .expect("area");
            a.set_item("k", "1").expect("set");
            a.set_item("k2", "2").expect("set");
            assert_eq!(a.get_item("k").as_deref(), Some("1"));
            assert!(b.get_item("k").is_none());
            assert_eq!(a.len(), 2);
            a.remove_item("k2").expect("remove");
            assert_eq!(a.keys(), vec!["k".to_string()]);
            let again = store
                .area(zone, &PartitionKey::None, &origin("https://a.test"))
                .expect("area");
            assert_eq!(again.get_item("k").as_deref(), Some("1"));
        }
        let store = FileLocalStore::open(&dir).expect("reopen");
        let a = store
            .area(zone, &PartitionKey::None, &origin("https://a.test"))
            .expect("area");
        assert_eq!(a.get_item("k").as_deref(), Some("1"));
        assert_eq!(a.len(), 1);
        a.clear().expect("clear");
        assert!(a.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn quotas_are_enforced_and_leave_state_intact() {
        let dir = scratch("quota");
        let store = FileLocalStore::open(&dir).expect("open");
        let a = store
            .area(ZoneId::new(), &PartitionKey::None, &origin("https://q.test"))
            .expect("area");
        a.set_item("small", "x").expect("set");
        let huge = "v".repeat(MAX_VALUE_BYTES + 1);
        assert!(a.set_item("huge", &huge).is_err());
        let big = "v".repeat(MAX_VALUE_BYTES);
        assert!(a.set_item("b1", &big).is_ok());
        // Two maximum values exceed the area quota.
        assert!(
            a.set_item("b2", &big).is_err(),
            "second 5 MiB value must exceed the area quota"
        );
        assert_eq!(a.len(), 2);
        assert!(a.get_item("b2").is_none());
        assert_eq!(a.get_item("small").as_deref(), Some("x"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A damaged area file is refused, not handed out empty and overwritten.
    #[test]
    fn a_damaged_area_file_is_not_replaced() {
        let dir = scratch("damaged");
        let store = FileLocalStore::open(&dir).expect("open");
        let path = dir.join(area_file_name("zone", "none", "https://d.test"));
        std::fs::write(&path, b"{not json").expect("write");
        assert!(store.area_for("zone", "none", "https://d.test").is_err());
        assert!(
            store.area_for("zone", "none", "https://d.test").is_err(),
            "a failed load is not cached as an empty area"
        );
        assert_eq!(std::fs::read(&path).expect("read"), b"{not json");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Two stores on one directory, however it is spelled, write through the
    /// same areas instead of over each other.
    #[test]
    fn stores_on_one_directory_share_their_areas() {
        let dir = scratch("shared");
        let first = FileLocalStore::open(&dir).expect("open");
        let second = FileLocalStore::open(dir.join(".")).expect("open");
        let a = first.area_for("zone", "none", "https://s.test").expect("area");
        let b = second.area_for("zone", "none", "https://s.test").expect("area");
        a.set_item("a", "1").expect("set");
        b.set_item("b", "2").expect("set");
        drop((first, second, a, b));
        let reopened = FileLocalStore::open(&dir).expect("reopen");
        let area = reopened.area_for("zone", "none", "https://s.test").expect("area");
        assert_eq!(
            area.get_item("a").as_deref(),
            Some("1"),
            "the second store wrote over the first"
        );
        assert_eq!(area.get_item("b").as_deref(), Some("2"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A handle that outlives its store is the same area a reopened store
    /// hands out, not a second copy that writes over it.
    #[test]
    fn a_handle_that_outlives_its_store_meets_the_reopened_one() {
        let dir = scratch("outlived");
        let store = FileLocalStore::open(&dir).expect("open");
        let kept = store.area_for("zone", "none", "https://o.test").expect("area");
        drop(store);
        let reopened = FileLocalStore::open(&dir).expect("reopen");
        let fresh = reopened.area_for("zone", "none", "https://o.test").expect("area");
        kept.set_item("a", "1").expect("set");
        fresh.set_item("b", "2").expect("set");
        drop((kept, fresh, reopened));
        let area = FileLocalStore::open(&dir)
            .expect("open")
            .area_for("zone", "none", "https://o.test")
            .expect("area");
        assert_eq!(
            area.get_item("a").as_deref(),
            Some("1"),
            "the reopened store wrote over the kept handle"
        );
        assert_eq!(area.get_item("b").as_deref(), Some("2"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// No custom partition string reaches another variant's area.
    #[test]
    fn partition_variants_never_share_an_area() {
        let top = origin("https://top.test");
        let names = [
            partition_name(&PartitionKey::None),
            partition_name(&PartitionKey::Custom(String::new())),
            partition_name(&PartitionKey::TopLevel(top.clone())),
            partition_name(&PartitionKey::Custom(format!("top:{}", top.ascii_serialization()))),
            partition_name(&PartitionKey::Custom("none".into())),
        ];
        for (i, a) in names.iter().enumerate() {
            for b in &names[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }

    #[test]
    fn file_names_are_injective_and_safe() {
        let a = area_file_name("z", "ab", "c");
        let b = area_file_name("z", "a", "bc");
        assert_ne!(a, b);
        let name = area_file_name("z", "top:https://x", "https://../../etc");
        assert!(name.chars().all(|c| c.is_ascii_hexdigit()
            || c == '-'
            || c == '.'
            || c == 'j'
            || c == 's'
            || c == 'o'
            || c == 'n'));
        assert!(!name.contains("/"));
    }
}
