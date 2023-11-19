use crate::config::settings::Setting;
use crate::config::StorageAdapter;
use crate::types::Result;
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Default)]
pub struct MemoryStorageAdapter {
    settings: Mutex<HashMap<String, Setting>>,
}

impl MemoryStorageAdapter {
    pub fn new() -> Self {
        MemoryStorageAdapter {
            settings: Mutex::new(HashMap::new()),
        }
    }
}

impl StorageAdapter for MemoryStorageAdapter {
    fn get(&self, key: &str) -> Option<Setting> {
        let lock = self.settings.lock().unwrap();
        let v = lock.get(key);
        v.cloned()
    }

    fn set(&self, key: &str, value: Setting) {
        let mut lock = self.settings.lock().unwrap();
        lock.insert(key.to_string(), value);
    }

    fn all(&self) -> Result<HashMap<String, Setting>> {
        let lock = self.settings.lock().unwrap();
        Ok(lock.clone())
    }
}
