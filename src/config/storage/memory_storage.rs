use crate::config::settings::Setting;
use crate::config::Store;
use crate::types::Result;
use std::collections::HashMap;

#[derive(Default)]
pub struct MemoryStorageAdapter {
    store: HashMap<String, Setting>,
}

impl MemoryStorageAdapter {
    pub fn new() -> Self {
        MemoryStorageAdapter {
            store: HashMap::new(),
        }
    }
}

impl Store for MemoryStorageAdapter {
    fn get_setting(&self, key: &str) -> Option<Setting> {
        let v = self.store.get(key);
        v.cloned()
    }

    fn set_setting(&mut self, key: &str, value: Setting) {
        self.store.insert(key.to_string(), value);
    }

    fn get_all_settings(&self) -> Result<HashMap<String, Setting>> {
        Ok(self.store.clone())
    }
}
