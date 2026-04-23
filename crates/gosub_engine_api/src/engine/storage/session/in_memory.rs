use anyhow::Result;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::engine::storage::area::{SessionStore, StorageArea};
use crate::engine::storage::types::PartitionKey;
use crate::tab::TabId;
use crate::zone::ZoneId;

/// In memory storage
#[derive(Default)]
pub struct InMemorySessionStore {
    data: Arc<RwLock<HashMap<(ZoneId, TabId, String, String), HashMap<String, String>>>>,
}

impl InMemorySessionStore {
    /// Creates a new instance of the in-memory session store.
    pub fn new() -> Self {
        Self::default()
    }
}

impl SessionStore for InMemorySessionStore {
    fn area(&self, zone: ZoneId, tab: TabId, part: &PartitionKey, origin: &url::Origin) -> Arc<dyn StorageArea> {
        let k = (
            zone,
            tab,
            match part {
                PartitionKey::None => "".to_string(),
                PartitionKey::TopLevel(o) => format!("top:{}", o.ascii_serialization()),
                PartitionKey::Custom(s) => s.to_string(),
            },
            origin.ascii_serialization(),
        );

        {
            let mut guard = self.data.write().unwrap();
            guard.entry(k.clone()).or_default();
        }

        Arc::new(SessionArea {
            data: Arc::clone(&self.data),
            key: k,
        })
    }

    fn drop_tab(&self, zone: ZoneId, tab: TabId) {
        let mut guard = self.data.write().unwrap();
        guard.retain(|(z, t, _, _), _| *z != zone || *t != tab);
    }
}

struct SessionArea {
    data: Arc<RwLock<HashMap<(ZoneId, TabId, String, String), HashMap<String, String>>>>,
    key: (ZoneId, TabId, String, String),
}

impl StorageArea for SessionArea {
    fn get_item(&self, k: &str) -> Option<String> {
        self.data
            .read()
            .unwrap()
            .get(&self.key)
            .and_then(|m| m.get(k).cloned())
    }

    fn set_item(&self, k: &str, v: &str) -> Result<()> {
        self.data
            .write()
            .unwrap()
            .get_mut(&self.key)
            .unwrap()
            .insert(k.to_string(), v.to_string());
        Ok(())
    }

    fn remove_item(&self, k: &str) -> Result<()> {
        self.data
            .write()
            .unwrap()
            .get_mut(&self.key)
            .map(|m| m.remove(k));
        Ok(())
    }

    fn clear(&self) -> Result<()> {
        self.data
            .write()
            .unwrap()
            .insert(self.key.clone(), HashMap::new());
        Ok(())
    }

    fn len(&self) -> usize {
        self.data
            .read()
            .unwrap()
            .get(&self.key)
            .map(|m| m.len())
            .unwrap_or(0)
    }

    fn keys(&self) -> Vec<String> {
        self.data
            .read()
            .unwrap()
            .get(&self.key)
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default()
    }
}
