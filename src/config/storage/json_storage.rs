use std::collections::HashMap;
use crate::config::settings::Setting;
use crate::config::StorageAdapter;

pub struct JsonStorageAdapter {
    path: String
}

impl JsonStorageAdapter {
    fn new(path: String) -> Self {
        JsonStorageAdapter {
            path
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
