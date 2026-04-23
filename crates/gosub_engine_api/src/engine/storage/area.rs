use super::types::PartitionKey;
use crate::tab::TabId;
use crate::zone::ZoneId;
use anyhow::Result;
use std::sync::Arc;

/// Object-safe key/value storage area (DOM’s Storage).
pub trait StorageArea: Send + Sync {
    /// Retrieves the value associated with the given key, or `None` if not found.
    fn get_item(&self, key: &str) -> Option<String>;

    /// Sets the value for the given key, overwriting any existing value.
    fn set_item(&self, key: &str, value: &str) -> Result<()>;

    /// Removes the item with the given key.
    fn remove_item(&self, key: &str) -> Result<()>;

    /// Clears all items in the storage area.
    fn clear(&self) -> Result<()>;

    /// Returns the number of items in the storage area.
    fn len(&self) -> usize;

    /// Returns a vector of all keys in the storage area.
    fn keys(&self) -> Vec<String>;
}

/// Store for localStorage-like areas (shared per (zone, partition, origin)).
pub trait LocalStore: Send + Sync {
    /// Retrieves a storage area for the given zone, partition, and origin.
    fn area(&self, zone: ZoneId, part: &PartitionKey, origin: &url::Origin) -> Result<Arc<dyn StorageArea>>;
}

/// Store for sessionStorage-like areas (isolated per (zone, tab, partition, origin)).
pub trait SessionStore: Send + Sync {
    /// Retrieves a storage area for the given zone, tab, partition, and origin.
    fn area(&self, zone: ZoneId, tab: TabId, part: &PartitionKey, origin: &url::Origin) -> Arc<dyn StorageArea>;

    /// Drops all session storage for the given tab in the specified zone.
    fn drop_tab(&self, zone: ZoneId, tab: TabId);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::InMemorySessionStore;

    fn set(area: &Arc<dyn StorageArea>, k: &str, v: &str) {
        area.set_item(k, v).unwrap();
    }

    fn o(s: &str) -> url::Origin {
        let url = url::Url::parse(s).expect("valid URL");
        url.origin()
    }

    #[test]
    fn storagearea_basic_contract() {
        let zone = ZoneId::new();
        let tab = TabId::new();
        let part = PartitionKey::None;
        let origin = o("https://example.com");

        let store = InMemorySessionStore::new();
        let area = store.area(zone, tab, &part, &origin);

        // starts empty
        assert_eq!(area.len(), 0);
        assert!(area.get_item("missing").is_none());

        // set + get
        set(&area, "a", "1");
        set(&area, "b", "2");
        assert_eq!(area.len(), 2);
        assert_eq!(area.get_item("a").as_deref(), Some("1"));
        assert_eq!(area.get_item("b").as_deref(), Some("2"));

        // overwrite keeps len()
        set(&area, "a", "ONE");
        assert_eq!(area.len(), 2);
        assert_eq!(area.get_item("a").as_deref(), Some("ONE"));

        // remove
        area.remove_item("b").unwrap();
        assert_eq!(area.len(), 1);
        assert!(area.get_item("b").is_none());

        // clear
        area.clear().unwrap();
        assert_eq!(area.len(), 0);
    }
}
