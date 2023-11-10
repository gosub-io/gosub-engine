pub mod settings;
pub mod storage;

use crate::config::settings::{Setting, SettingInfo};
use serde_derive::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use wildmatch::WildMatch;

const SETTINGS_JSON: &str = include_str!("./config/settings.json");

#[derive(Debug, Deserialize)]
struct JsonEntry {
    key: String,
    #[serde(rename = "type")]
    entry_type: String,
    default: String,
    description: String,
}

pub trait StorageAdapter {
    /// Retrieves a setting from the storage
    fn get_setting(&self, key: &str) -> Option<Setting>;
    /// Stores a given setting to the storage
    fn set_setting(&mut self, key: &str, value: Setting);
    /// Retrieves all the settings in the storage
    fn get_all_settings(&self) -> HashMap<String, Setting>;
}

/// Configuration store is the place where the gosub engine can find all configurable options
pub struct ConfigStore {
    /// A hashmap of all settings so we can search o(1) time
    settings: HashMap<String, Setting>,
    /// A hashmap of all setting descriptions
    settings_info: HashMap<String, SettingInfo>,
    /// Keys of all settings so we can iterate keys easily
    setting_keys: Vec<String>,
    /// The storage adapter used for persisting and loading keys
    storage: Box<dyn StorageAdapter>,
}

impl ConfigStore {
    /// Creates a new store with the given storage adapter and preloads the store if needed
    pub fn new(storage: Box<dyn StorageAdapter>, preload: bool) -> Self {
        let mut store = ConfigStore {
            settings: HashMap::new(),
            settings_info: HashMap::new(),
            setting_keys: Vec::new(),
            storage,
        };

        // Populate the settings from the json file
        store.populate_settings();

        // preload the settings if requested
        if preload {
            let all_settings = store.storage.get_all_settings();
            for (key, value) in all_settings {
                store.settings.insert(key, value);
            }
        }

        store
    }

    pub fn has(&self, key: &str) -> bool {
        self.settings.contains_key(key)
    }

    /// Returns a list of keys that mathces the given search string (can use *)
    pub fn find(&self, search: &str) -> Vec<String> {
        let search = WildMatch::new(search);

        let mut keys = Vec::new();
        for key in &self.setting_keys {
            if search.matches(key.as_str()) {
                let key = key.clone();
                keys.push(key);
            }
        }

        keys
    }

    pub fn get_info(&self, key: &str) -> Option<SettingInfo> {
        self.settings_info.get(key).cloned()
    }

    /// Returns the setting with the given key
    /// If the setting does not exist, it will try and load it from the storage adapter
    pub fn get(&mut self, key: &str, default: Option<Setting>) -> Setting {
        if let Some(setting) = self.settings.get(key) {
            return setting.clone();
        }

        // Setting not found, try and load it from the storage adapter
        if let Some(setting) = self.storage.get_setting(key) {
            self.settings.insert(key.to_string(), setting.clone());
            return setting.clone();
        }

        // Panic if we can't find the setting, and we don't have a default
        if default.is_none() {
            panic!("Setting {} not found", key);
        }

        default.unwrap()
    }

    pub fn set(&mut self, key: &str, value: Setting) {
        self.settings.insert(key.to_string(), value.clone());
        self.storage.set_setting(key, value);
    }

    /// Populates the settings in the store from the settings.json file
    fn populate_settings(&mut self) {
        let json_data = serde_json::from_str(SETTINGS_JSON);
        if json_data.is_err() {
            panic!("Failed to parse settings.json");
        }

        let json_data = json_data.unwrap();
        if let Value::Object(data) = json_data {
            for (section_prefix, section_entries) in data.iter() {
                let section_entries: Vec<JsonEntry> =
                    serde_json::from_value(section_entries.clone())
                        .expect("Failed to parse settings.json");

                for entry in section_entries {
                    let key = format!("{}.{}", section_prefix, entry.key);

                    let info = SettingInfo {
                        key: key.clone(),
                        description: entry.description,
                        default: Setting::from_string(entry.default.as_str())
                            .expect("cannot parse default setting"),
                        last_accessed: 0,
                    };

                    self.setting_keys.push(key.clone());
                    self.settings_info.insert(key.clone(), info.clone());
                    self.settings.insert(key.clone(), info.default.clone());
                }
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::config::storage::memory_storage::MemoryStorageAdapter;

    #[test]
    fn test_config_store() {
        let mut store = ConfigStore::new(Box::new(MemoryStorageAdapter::new()), true);
        let setting = store.get("dns.local_resolver.enabled", None);
        assert_eq!(setting, Setting::Bool(false));

        store.set("dns.local_resolver.enabled", Setting::Bool(true));
        let setting = store.get("dns.local_resolver.enabled", None);
        assert_eq!(setting, Setting::Bool(true));
    }
}
