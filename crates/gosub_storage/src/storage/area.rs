use super::types::PartitionKey;
use anyhow::Result;
use gosub_net::types::TabId;
use gosub_net::types::ZoneId;
use std::sync::Arc;

pub trait StorageArea: Send + Sync {
    fn get_item(&self, key: &str) -> Option<String>;
    fn set_item(&self, key: &str, value: &str) -> Result<()>;
    fn remove_item(&self, key: &str) -> Result<()>;
    fn clear(&self) -> Result<()>;
    fn len(&self) -> usize;
    fn keys(&self) -> Vec<String>;
}

pub trait LocalStore: Send + Sync {
    fn area(&self, zone: ZoneId, part: &PartitionKey, origin: &url::Origin) -> Result<Arc<dyn StorageArea>>;
}

pub trait SessionStore: Send + Sync {
    fn area(&self, zone: ZoneId, tab: TabId, part: &PartitionKey, origin: &url::Origin) -> Arc<dyn StorageArea>;
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

        assert_eq!(area.len(), 0);
        assert!(area.get_item("missing").is_none());

        set(&area, "a", "1");
        set(&area, "b", "2");
        assert_eq!(area.len(), 2);
        assert_eq!(area.get_item("a").as_deref(), Some("1"));
        assert_eq!(area.get_item("b").as_deref(), Some("2"));

        set(&area, "a", "ONE");
        assert_eq!(area.len(), 2);
        assert_eq!(area.get_item("a").as_deref(), Some("ONE"));

        area.remove_item("b").unwrap();
        assert_eq!(area.len(), 1);
        assert!(area.get_item("b").is_none());

        area.clear().unwrap();
        assert_eq!(area.len(), 0);
    }
}
