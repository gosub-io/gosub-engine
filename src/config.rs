pub mod settings;
pub mod storage;

use crate::config::settings::{Setting, SettingInfo};
use crate::types::Result;
use serde_derive::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::mem;
use std::str::FromStr;
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

/// StorageAdapter is the interface for storing and retrieving settings
/// This can be used to store settings in a database, json file, etc
pub trait Store {
    /// Retrieves a setting from the storage
    fn get_setting(&self, key: &str) -> Option<Setting>;
    /// Stores a given setting to the storage
    fn set_setting(&mut self, key: &str, value: Setting);
    /// Retrieves all the settings in the storage in one go. This is used for preloading the settings
    /// into the ConfigStore and is more performant normally than calling get_setting manually for each
    /// setting.
    fn get_all_settings(&self) -> Result<HashMap<String, Setting>>;
}

/// Configuration store is the place where the gosub engine can find all configurable options
pub struct ConfigStore {
    /// A hashmap of all settings so we can search o(1) time
    settings: HashMap<String, Setting>,
    /// A hashmap of all setting descriptions, default values and type information
    settings_info: HashMap<String, SettingInfo>,
    /// Keys of all settings so we can iterate keys easily
    setting_keys: Vec<String>,
    /// The storage adapter used for persisting and loading keys
    storage: Box<dyn Store>,
}

impl ConfigStore {
    /// Creates a new store with the given storage adapter and preloads the store if needed
    pub fn from_storage(storage: Box<dyn Store>, preload: bool) -> Result<Self> {
        let mut store = ConfigStore {
            settings: HashMap::new(),
            settings_info: HashMap::new(),
            setting_keys: Vec::new(),
            storage,
        };

        // Populate the settings from the json file
        store.populate_settings()?;

        // preload the settings if requested
        if preload {
            let all_settings = store.storage.get_all_settings()?;
            for (key, value) in all_settings {
                store.settings.insert(key, value);
            }
        }

        Ok(store)
    }

    /// Returns true when the store knows about the given key
    pub fn has(&self, key: &str) -> bool {
        self.settings.contains_key(key)
    }

    /// Returns a list of keys that matches the given search string (can use ? and *) for search
    /// wildcards.
    pub fn find(&self, search: &str) -> Vec<String> {
        let search = WildMatch::new(search);

        let mut keys = Vec::new();
        for key in &self.setting_keys {
            if search.matches(key) {
                let key = key.clone();
                keys.push(key);
            }
        }

        keys
    }

    /// Retrieves information about the given key, or returns None when key is unknown
    pub fn get_info(&self, key: &str) -> Option<SettingInfo> {
        self.settings_info.get(key).cloned()
    }

    /// Returns the setting with the given key. If the setting is not found in the current
    /// store, it will load the key from the storage. If the key is still not found, it will
    /// return the default value for the given key. Note that if the key is not found and no
    /// default value is specified, this function will panic.
    pub fn get(&mut self, key: &str) -> Setting {
        if !self.has(key) {
            panic!("Setting {} not found", key);
        }

        if let Some(setting) = self.settings.get(key) {
            return setting.clone();
        }

        // Setting not found, try and load it from the storage adapter
        if let Some(setting) = self.storage.get_setting(key) {
            self.settings.insert(key.to_string(), setting.clone());
            return setting.clone();
        }

        // Return the default value for the setting when nothing is found
        let info = self.settings_info.get(key).unwrap();
        info.default.clone()
    }

    /// Sets the given setting to the given value. Will persist the setting to the
    /// storage.
    pub fn set(&mut self, key: &str, value: Setting) {
        if !self.has(key) {
            panic!("key not found");
        }
        let info = self.settings_info.get(key).unwrap();
        if mem::discriminant(&info.default) != mem::discriminant(&value) {
            panic!("value is of different type than setting expects")
        }

        self.settings.insert(key.to_string(), value.clone());
        self.storage.set_setting(key, value);
    }

    /// Populates the settings in the store from the settings.json file
    fn populate_settings(&mut self) -> Result<()> {
        let json_data: Value =
            serde_json::from_str(SETTINGS_JSON).expect("Failed to parse settings.json");

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
                        default: Setting::from_str(&entry.default)?,
                        last_accessed: 0,
                    };

                    self.setting_keys.push(key.clone());
                    self.settings_info.insert(key.clone(), info.clone());
                    self.settings.insert(key.clone(), info.default.clone());
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::config::storage::memory_storage::MemoryStorageAdapter;

    #[test]
    fn config_store() {
        let mut store =
            ConfigStore::from_storage(Box::new(MemoryStorageAdapter::new()), true).unwrap();
        let setting = store.get("dns.local_resolver.enabled");
        assert_eq!(setting, Setting::Bool(false));

        store.set("dns.local_resolver.enabled", Setting::Bool(true));
        let setting = store.get("dns.local_resolver.enabled");
        assert_eq!(setting, Setting::Bool(true));
    }

    #[test]
    #[should_panic]
    fn invalid_setting() {
        let mut store =
            ConfigStore::from_storage(Box::new(MemoryStorageAdapter::new()), true).unwrap();
        store.set(
            "dns.local_resolver.enabled",
            Setting::String("wont accept strings".into()),
        );
    }
}
