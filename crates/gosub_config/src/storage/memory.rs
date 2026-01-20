use crate::settings::Setting;
use crate::StorageAdapter;
use gosub_shared::types::Result;
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Default)]
pub struct MemoryStorageAdapter {
    settings: Mutex<HashMap<String, Setting>>,
}

impl MemoryStorageAdapter {
    #[must_use]
    pub fn new() -> Self {
        Self {
            settings: Mutex::new(HashMap::new()),
        }
    }
}

impl StorageAdapter for MemoryStorageAdapter {
    fn get(&self, key: &str) -> Option<Setting> {
        self.settings
            .lock()
            .expect("memory storage lock poisoned")
            .get(key)
            .cloned()
    }

    fn set(&self, key: &str, value: Setting) {
        let mut lock = self.settings.lock().expect("memory storage lock poisoned");
        lock.insert(key.to_owned(), value);
    }

    fn all(&self) -> Result<HashMap<String, Setting>> {
        let lock = self.settings.lock().expect("memory storage lock poisoned");
        Ok(lock.clone())
    }
}
