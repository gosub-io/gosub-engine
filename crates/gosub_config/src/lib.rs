mod errors;
pub mod settings;
pub mod storage;

use crate::settings::{Setting, SettingInfo};
use crate::storage::MemoryStorageAdapter;
use gosub_shared::types::Result;
use lazy_static::lazy_static;
use log::warn;
use serde_derive::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::mem;
use std::str::FromStr;
use std::sync::RwLock;
use wildmatch::WildMatch;

/// Settings are stored in a json file, but this is included in the binary for mostly easy editting.
const SETTINGS_JSON: &str = include_str!("./settings.json");

/// `StoreAdapter` is the interface for storing and retrieving settings
/// This can be used to storage settings in a database, json file, etc
/// Note that we need to implement Send so we can send the storage adapter
/// to other threads.
pub trait StorageAdapter: Send + Sync {
    /// Retrieves a setting from the storage
    fn get(&self, key: &str) -> Option<Setting>;

    /// Stores a given setting to the storage. Note that "self" is self and not "mut self". We need to be able
    /// to storage settings in a non-mutable way. That is mostly possible it seems with a mutex lock that we
    /// can get mutable.
    fn set(&self, key: &str, value: Setting);

    /// Retrieves all the settings in the storage in one go. This is used for preloading the settings
    /// into the `ConfigStore` and is more performant normally than calling `get_setting` manually for each
    /// setting.
    fn all(&self) -> Result<HashMap<String, Setting>>;
}

lazy_static! {
    // Initial config store will have a memory storage adapter. It will save within the session, but not
    // persist this on disk.
    static ref CONFIG_STORE: RwLock<ConfigStore> = RwLock::new(ConfigStore::default());
}

/// Returns a reference to the config store, which is locked by a mutex.
/// Any callers of the config store can just do  `config::config_store().get("dns.local.enabled`")
pub fn config_store() -> std::sync::RwLockReadGuard<'static, ConfigStore> {
    CONFIG_STORE.read().unwrap()
}

pub fn config_store_write() -> std::sync::RwLockWriteGuard<'static, ConfigStore> {
    CONFIG_STORE.write().unwrap()
}

/// These macro's can be used to simplify the calls to the config store. You can simply do:
///
/// let enabled = config!(bool "`dns.local.enabled").unwrap()`;
/// `config_set!(bool` "dns.local.enabled", false);
///
/// Note that when you cannot find the key, it will return a default value. This is not always
/// what you want, but you can test for existence of the key with `config_store().has("key`")
#[allow(clippy::crate_in_macro_def)]
#[macro_export]
macro_rules! config {
    (string $key:expr) => {
        match config_store().get($key) {
            Some(setting) => setting.to_string(),
            None => String::new(),
        }
    };
    (bool $key:expr) => {
        match config_store().get($key) {
            Some(setting) => setting.to_bool(),
            None => false,
        }
    };
    (uint $key:expr) => {
        match config_store().get($key) {
            Some(setting) => setting.to_uint(),
            None => 0,
        }
    };
    (sint $key:expr) => {
        match config_store().get($key) {
            Some(setting) => setting.to_sint(),
            None => 0,
        }
    };
    (map $key:expr) => {
        match config_store().get($key) {
            Some(setting) => setting.to_map(),
            None => Vec::new(),
        }
    };
}

#[allow(clippy::crate_in_macro_def)]
#[macro_export]
macro_rules! config_set {
    (string $key:expr, $val:expr) => {
        config_store().set($key, Setting::String($val))
    };
    (bool $key:expr, $val:expr) => {
        config_store().set($key, Setting::Bool($val))
    };
    (uint $key:expr, $val:expr) => {
        config_store().set($key, Setting::UInt($val))
    };
    (sint $key:expr, $val:expr) => {
        config_store().set($key, Setting::SInt($val))
    };
    (map $key:expr, $val:expr) => {
        config_store().set($key, Setting::Map($val))
    };
}

/// `JsonEntry` is used for parsing the settings.json file
#[derive(Debug, Deserialize)]
struct JsonEntry {
    key: String,
    #[serde(rename = "type")]
    _entry_type: String,
    default: String,
    description: String,
}

/// Configuration storage is the place where the gosub engine can find all configurable options
pub struct ConfigStore {
    /// A hashmap of all settings so we can search o(1) time
    /// The mutex allows to share between multiple threads,
    /// The refcell allows us to use mutable references in a non-mutable way (ie: settings can be
    /// stored while doing a immutable `get()`)
    settings: std::sync::Mutex<std::cell::RefCell<HashMap<String, Setting>>>,
    /// A hashmap of all setting descriptions, default values and type information
    settings_info: HashMap<String, SettingInfo>,
    /// Keys of all settings so we can iterate keys easily
    setting_keys: Vec<String>,
    /// The storage adapter used for persisting and loading keys
    storage: Box<dyn StorageAdapter>,
}

impl Default for ConfigStore {
    fn default() -> Self {
        let mut store = Self {
            settings: std::sync::Mutex::new(std::cell::RefCell::new(HashMap::new())),
            settings_info: HashMap::new(),
            setting_keys: Vec::new(),
            storage: Box::new(MemoryStorageAdapter::new()),
        };

        // Populate the store with the default settings. They may be overwritten by the storage
        // as soon as one is added with config::config_store()::set_storage()
        let _ = store.populate_default_settings();
        store
    }
}

impl ConfigStore {
    /// Sets a new storage engine and updates all settings in the config store according to what
    /// is written in the storage. Note that it will overwrite any current settings in the config
    /// store. Take this into consideration when using this function to switch storage engines.
    pub fn set_storage(&mut self, storage: Box<dyn StorageAdapter>) {
        self.storage = storage;

        // Find all keys, and add them to the configuration store
        if let Ok(all_settings) = self.storage.all() {
            for (key, value) in all_settings {
                self.settings
                    .lock()
                    .unwrap()
                    .borrow_mut()
                    .insert(key, value);
            }
        }
    }

    /// Returns true when the storage knows about the given key
    pub fn has(&self, key: &str) -> bool {
        self.settings.lock().unwrap().borrow().contains_key(key)
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
    /// storage, it will load the key from the storage. If the key is still not found, it will
    /// return the default value for the given key. Note that if the key is not found and no
    /// default value is specified, this function will panic.
    pub fn get(&self, key: &str) -> Option<Setting> {
        if let Some(setting) = self.settings.lock().unwrap().borrow().get(key) {
            return Some(setting.clone());
        }

        // Setting not found, try and load it from the storage adapter
        if let Some(setting) = self.storage.get(key) {
            self.settings
                .lock()
                .unwrap()
                .borrow_mut()
                .insert(key.to_string(), setting.clone());
            return Some(setting);
        }

        // Return the default value for the setting when nothing is found
        if let Some(info) = self.settings_info.get(key) {
            return Some(info.default.clone());
        }

        // At this point we haven't found the key in the store, we haven't found it in storage, and we
        // don't have a default value. This is a programming error, so we panic.
        panic!("config: Setting {key} is not known");
    }

    /// Sets the given setting to the given value. Will persist the setting to the
    /// storage. Note that the setting MUST have a settings-info entry, otherwise
    /// this function will not store the setting.
    pub fn set(&self, key: &str, value: Setting) {
        let info = if let Some(info) = self.settings_info.get(key) { info } else {
            warn!("config: Setting {key} is not known");
            return;
        };

        if mem::discriminant(&info.default) != mem::discriminant(&value) {
            warn!("config: Setting {key} is of different type than setting expects");
            return;
        }

        self.settings
            .lock()
            .unwrap()
            .borrow_mut()
            .insert(key.to_owned(), value.clone());

        self.storage.set(key, value);
    }

    /// Populates the settings in the storage from the settings.json file
    fn populate_default_settings(&mut self) -> Result<()> {
        let json_data: Value =
            serde_json::from_str(SETTINGS_JSON).expect("Failed to parse settings.json");

        if let Value::Object(data) = json_data {
            for (section_prefix, section_entries) in &data {
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
                    self.settings
                        .lock()
                        .unwrap()
                        .borrow_mut()
                        .insert(key.clone(), info.default.clone());
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use storage::MemoryStorageAdapter;

    #[test]
    fn test_config_store() {
        config_store_write().set_storage(Box::new(MemoryStorageAdapter::new()));

        let setting = config_store().get("dns.local.enabled").unwrap();
        assert_eq!(setting, Setting::Bool(true));

        config_store_write().set("dns.local.enabled", Setting::Bool(false));
        let setting = config_store().get("dns.local.enabled").unwrap();
        assert_eq!(setting, Setting::Bool(false));
    }

    #[test]
    fn invalid_setting() {
        testing_logger::setup();

        testing_logger::validate(|captured_logs| {
            assert_eq!(captured_logs.len(), 0);
        });

        config_store_write().set(
            "dns.local.enabled",
            Setting::String("wont accept strings".into()),
        );

        testing_logger::validate(|captured_logs| {
            assert_eq!(captured_logs.len(), 1);
            assert_eq!(captured_logs[0].level, log::Level::Warn);
        });
    }

    #[test]
    fn macro_usage() {
        config_store_write().set_storage(Box::new(MemoryStorageAdapter::new()));

        config_set!(uint "dns.cache.max_entries", 9432);
        let max_entries = config!(uint "dns.cache.max_entries");
        assert_eq!(max_entries, 9432);
    }

    #[test]
    #[should_panic]
    fn macro_usage_with_panic() {
        config_set!(string "this.key.doesnt.exist", "yesitdoes".into());
        let s = config!(string "this.key.doesnt.exist");
        assert_eq!(s, "");
    }
}
