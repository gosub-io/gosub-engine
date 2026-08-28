//! `localStorage` as one JSON file per `(zone, partition, origin)` area.
//!
//! Plain files rather than SQLite because the storage service's filter allows
//! `openat` and not much else (no locks, renames or directory listing).
//! Filenames are hex-encoded tuples, so page-controlled strings never reach a
//! path; keys live inside the file. Per-value and per-area size caps.

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

#[derive(Debug, Clone)]
pub struct FileLocalStore {
    dir: PathBuf,
    /// Loaded areas, so handles to the same area share state.
    areas: Arc<Mutex<HashMap<PathBuf, Arc<FileArea>>>>,
}

impl FileLocalStore {
    /// Creates `dir`. Only the broker can: the service has no `mkdir`.
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir)?;
        Ok(Self {
            dir,
            areas: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Uses an existing `dir` without touching it.
    pub fn attach(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            areas: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn area_for(&self, zone: &str, partition: &str, origin: &str) -> Arc<FileArea> {
        let path = self.dir.join(area_file_name(zone, partition, origin));
        let mut areas = self.areas.lock();
        Arc::clone(
            areas
                .entry(path.clone())
                .or_insert_with(|| Arc::new(FileArea::load(path))),
        )
    }
}

/// Wire/file form of a `PartitionKey`.
pub fn partition_name(part: &PartitionKey) -> String {
    match part {
        PartitionKey::None => String::new(),
        PartitionKey::TopLevel(origin) => format!("top:{}", origin.ascii_serialization()),
        PartitionKey::Custom(s) => s.clone(),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `<hex zone>-<hex partition>-<hex origin>.json`; `-` cannot occur inside hex,
/// so the mapping is injective.
fn area_file_name(zone: &str, partition: &str, origin: &str) -> String {
    format!(
        "{}-{}-{}.json",
        hex(zone.as_bytes()),
        hex(partition.as_bytes()),
        hex(origin.as_bytes())
    )
}

impl LocalStore for FileLocalStore {
    fn area(&self, zone: ZoneId, part: &PartitionKey, origin: &url::Origin) -> Result<Arc<dyn StorageArea>> {
        Ok(self.area_for(&zone.to_string(), &partition_name(part), &origin.ascii_serialization()))
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
    fn load(path: PathBuf) -> Self {
        let items = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<HashMap<String, String>>(&bytes).ok())
            .unwrap_or_default();
        Self {
            path,
            items: Mutex::new(items),
        }
    }

    /// In place: no `rename` under the service's filter. A crash mid-write
    /// costs this area's last change.
    fn persist(&self, items: &HashMap<String, String>) -> Result<()> {
        let bytes = serde_json::to_vec(items)?;
        std::fs::write(&self.path, bytes)?;
        Ok(())
    }

    fn serialized_size(items: &HashMap<String, String>) -> usize {
        items.iter().map(|(k, v)| k.len() + v.len() + 6).sum()
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
        let mut items = self.items.lock();
        let previous = items.insert(key.to_string(), value.to_string());
        if Self::serialized_size(&items) > MAX_AREA_BYTES {
            match previous {
                Some(old) => items.insert(key.to_string(), old),
                None => items.remove(key),
            };
            return Err(anyhow!("storage quota of {MAX_AREA_BYTES} bytes exceeded"));
        }
        self.persist(&items)
    }

    fn remove_item(&self, key: &str) -> Result<()> {
        let mut items = self.items.lock();
        if items.remove(key).is_some() {
            self.persist(&items)?;
        }
        Ok(())
    }

    fn clear(&self) -> Result<()> {
        let mut items = self.items.lock();
        items.clear();
        self.persist(&items)
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

    #[test]
    fn areas_are_isolated_and_persist_across_reopen() {
        let dir = std::env::temp_dir().join(format!("gosub-file-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
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
        let dir = std::env::temp_dir().join(format!("gosub-file-store-quota-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
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
