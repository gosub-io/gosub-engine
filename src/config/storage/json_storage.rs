use crate::config::settings::Setting;
use crate::config::StorageAdapter;
use std::collections::HashMap;

pub struct JsonStorageAdapter {
    path: String,
}

impl JsonStorageAdapter {
    pub fn new(path: &str) -> Self {
        JsonStorageAdapter {
            path: path.to_string(),
        }
    }
}

impl StorageAdapter for JsonStorageAdapter {
    fn get_setting(&self, _key: &str) -> Option<Setting> {
        todo!()
    }

    fn set_setting(&mut self, _key: &str, _value: Setting) {
        todo!()
    }

    fn get_all_settings(&self) -> HashMap<String, Setting> {
        todo!()
    }
}
