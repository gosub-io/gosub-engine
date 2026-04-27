use crate::engine::storage::area::{LocalStore, StorageArea};
use crate::engine::storage::types::PartitionKey;
use crate::zone::ZoneId;
use anyhow::Result;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

type LocalAreaMap = HashMap<(ZoneId, PartitionKey, url::Origin), Arc<dyn StorageArea>>;

/// In‑memory local storage (no persistence). Used as a default when no storage is defined by the UA.
#[derive(Default)]
pub struct InMemoryLocalStore {
    areas: Mutex<LocalAreaMap>,
}

impl InMemoryLocalStore {
    /// Creates a new instance of the in-memory local store.
    pub fn new() -> Self {
        Self::default()
    }
}

impl LocalStore for InMemoryLocalStore {
    fn area(&self, zone: ZoneId, part: &PartitionKey, origin: &url::Origin) -> Result<Arc<dyn StorageArea>> {
        let key = (zone, part.clone(), origin.clone());
        let mut guard = self.areas.lock().unwrap();
        Ok(guard
            .entry(key)
            .or_insert_with(|| Arc::new(InMemoryLocalArea::default()) as Arc<dyn StorageArea>)
            .clone())
    }
}

#[derive(Default)]
struct InMemoryLocalArea {
    map: Mutex<HashMap<String, String>>,
}

impl StorageArea for InMemoryLocalArea {
    fn get_item(&self, key: &str) -> Option<String> {
        self.map.lock().ok()?.get(key).cloned()
    }

    fn set_item(&self, key: &str, value: &str) -> Result<()> {
        self.map.lock().unwrap().insert(key.to_string(), value.to_string());
        Ok(())
    }

    fn remove_item(&self, key: &str) -> Result<()> {
        self.map.lock().unwrap().remove(key);
        Ok(())
    }

    fn clear(&self) -> Result<()> {
        self.map.lock().unwrap().clear();
        Ok(())
    }

    fn len(&self) -> usize {
        self.map.lock().unwrap().len()
    }

    fn keys(&self) -> Vec<String> {
        let mut v: Vec<String> = self.map.lock().unwrap().keys().cloned().collect();
        v.sort_unstable(); // stable order if you want deterministic tests
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zone::ZoneId;

    fn o(s: &str) -> url::Origin {
        let url = url::Url::parse(s).expect("valid URL");
        url.origin()
    }

    #[test]
    fn area_contract() {
        let store = InMemoryLocalStore::new();
        let zone = ZoneId::new();
        let part = PartitionKey::TopLevel(o("https://example.com"));
        let origin = o("https://example.com");

        let area = store.area(zone, &part, &origin).unwrap();

        assert_eq!(area.len(), 0);
        assert!(area.get_item("missing").is_none());

        area.set_item("a", "1").unwrap();
        area.set_item("b", "2").unwrap();
        assert_eq!(area.len(), 2);
        assert_eq!(area.get_item("a").as_deref(), Some("1"));
        assert_eq!(area.get_item("b").as_deref(), Some("2"));

        // overwrite keeps len
        area.set_item("a", "ONE").unwrap();
        assert_eq!(area.len(), 2);
        assert_eq!(area.get_item("a").as_deref(), Some("ONE"));

        // remove
        area.remove_item("b").unwrap();
        assert_eq!(area.len(), 1);
        assert!(area.get_item("b").is_none());

        // clear
        area.clear().unwrap();
        assert_eq!(area.len(), 0);
        assert!(area.keys().is_empty());
    }

    #[test]
    fn same_tuple_shares_area_different_tuples_isolate() {
        let store = InMemoryLocalStore::new();
        let zone_a = ZoneId::new();
        let zone_b = ZoneId::new();
        let part_a = PartitionKey::TopLevel(o("https://a.test"));
        let part_b = PartitionKey::TopLevel(o("https://b.test"));
        let orig_a = o("https://a.test");
        let orig_b = o("https://b.test");

        let a1 = store.area(zone_a, &part_a, &orig_a).unwrap();
        let a2 = store.area(zone_a, &part_a, &orig_a).unwrap();
        a1.set_item("k", "v").unwrap();
        assert_eq!(a2.get_item("k").as_deref(), Some("v")); // shared

        // different origin
        let a_other_origin = store.area(zone_a, &part_a, &orig_b).unwrap();
        assert!(a_other_origin.get_item("k").is_none());

        // different partition
        let a_other_part = store.area(zone_a, &part_b, &orig_a).unwrap();
        assert!(a_other_part.get_item("k").is_none());

        // different zone
        let b_same_part_origin = store.area(zone_b, &part_a, &orig_a).unwrap();
        assert!(b_same_part_origin.get_item("k").is_none());
    }
}
