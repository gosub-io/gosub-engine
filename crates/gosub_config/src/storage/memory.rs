use crate::settings::Setting;
use crate::{Result, StorageAdapter};
use parking_lot::Mutex;
use std::collections::HashMap;

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
    fn get(&self, key: &str) -> Result<Option<Setting>> {
        let lock = self.settings.lock();
        Ok(lock.get(key).cloned())
    }

    fn set(&self, key: &str, value: Setting) -> Result<()> {
        let mut lock = self.settings.lock();
        lock.insert(key.to_owned(), value);
        Ok(())
    }

    fn all(&self) -> Result<HashMap<String, Setting>> {
        let lock = self.settings.lock();
        Ok(lock.clone())
    }
}
