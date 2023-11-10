use std::collections::HashMap;
use crate::config::settings::Setting;
use crate::config::StorageAdapter;

pub struct MemoryStorageAdapter {
    store: HashMap<String, Setting>
}

impl MemoryStorageAdapter {
    pub fn new() -> Self {
        MemoryStorageAdapter {
            store: HashMap::new()
        }
    }
}

impl StorageAdapter for MemoryStorageAdapter {
    fn get_setting(&self, key: &str) -> Option<Setting> {
        let v = self.store.get(key);
        match v {
            Some(v) => Some(v.clone()),
            None => None
        }
    }

    fn set_setting(&mut self, key: &str, value: Setting) {
        self.store.insert(key.to_string(), value);
    }

    fn get_all_settings(&self) -> HashMap<String, Setting> {
        self.store.clone()
    }
}
