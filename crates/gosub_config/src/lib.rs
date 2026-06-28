pub mod errors;
pub mod settings;
pub mod storage;

pub use errors::Error;
pub(crate) type Result<T> = std::result::Result<T, Error>;

use crate::settings::{Constraint, Setting, SettingInfo};
use crate::storage::MemoryStorageAdapter;
use lazy_static::lazy_static;
use log::warn;
use parking_lot::RwLock;
use serde_derive::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::mem;
use std::str::FromStr;
use wildmatch::WildMatch;

/// Settings are stored in a json file, but this is included in the binary for mostly easy editting.
const SETTINGS_JSON: &str = include_str!("./settings.json");

/// `StoreAdapter` is the interface for storing and retrieving settings
/// This can be used to storage settings in a database, json file, etc
/// Note that we need to implement Send so we can send the storage adapter
/// to other threads.
pub trait StorageAdapter: Send + Sync {
    /// Retrieves a setting from the storage. Returns `Ok(None)` when the key does not exist.
    fn get(&self, key: &str) -> Result<Option<Setting>>;

    /// Stores a given setting to the storage. Note that "self" is self and not "mut self". We need to be able
    /// to storage settings in a non-mutable way. That is mostly possible it seems with a mutex lock that we
    /// can get mutable.
    fn set(&self, key: &str, value: Setting) -> Result<()>;

    /// Removes a stored setting from the storage. Removing a key that does not exist is not an error
    /// (the operation is idempotent). This is used to revert a setting back to its default value.
    fn remove(&self, key: &str) -> Result<()>;

    /// Retrieves all the settings in the storage in one go. This is used for preloading the settings
    /// into the `ConfigStore` and is more performant normally than calling `get_setting` manually for each
    /// setting.
    fn all(&self) -> Result<HashMap<String, Setting>>;

    /// Flushes any buffered writes to the backing store. Adapters that persist eagerly on every `set`
    /// (or that do not persist at all, like the in-memory adapter) treat this as a no-op. It exists so
    /// callers can request an explicit durability point and so adapters can later batch writes without
    /// changing the trait.
    fn flush(&self) -> Result<()> {
        Ok(())
    }
}

lazy_static! {
    // Initial config store will have a memory storage adapter. It will save within the session, but not
    // persist this on disk.
    static ref CONFIG_STORE: RwLock<ConfigStore> = RwLock::new(ConfigStore::default());
}

/// Returns a reference to the config store, which is locked by a mutex.
/// Any callers of the config store can just do  `config::config_store().get("dns.local.enabled`")
pub fn config_store() -> parking_lot::RwLockReadGuard<'static, ConfigStore> {
    CONFIG_STORE.read()
}

pub fn config_store_write() -> parking_lot::RwLockWriteGuard<'static, ConfigStore> {
    CONFIG_STORE.write()
}

/// Reads a setting from the config store, returning a type-appropriate default when the key is
/// unknown or a storage error occurs.
///
/// ```ignore
/// let enabled = config!(bool "dns.local.enabled");
/// let max     = config!(uint "dns.cache.max_entries");
/// ```
#[allow(clippy::crate_in_macro_def)]
#[macro_export]
macro_rules! config {
    (string $key:expr) => {
        match config_store().get($key) {
            Ok(Some(setting)) => setting.to_string(),
            Ok(None) => String::new(),
            Err(err) => {
                log::warn!("config error: {err}");
                String::new()
            }
        }
    };
    (bool $key:expr) => {
        match config_store().get($key) {
            Ok(Some(setting)) => setting.to_bool(),
            Ok(None) => false,
            Err(err) => {
                log::warn!("config error: {err}");
                false
            }
        }
    };
    (uint $key:expr) => {
        match config_store().get($key) {
            Ok(Some(setting)) => setting.to_uint(),
            Ok(None) => 0,
            Err(err) => {
                log::warn!("config error: {err}");
                0
            }
        }
    };
    (sint $key:expr) => {
        match config_store().get($key) {
            Ok(Some(setting)) => setting.to_sint(),
            Ok(None) => 0,
            Err(err) => {
                log::warn!("config error: {err}");
                0
            }
        }
    };
    (float $key:expr) => {
        match config_store().get($key) {
            Ok(Some(setting)) => setting.to_float(),
            Ok(None) => 0.0,
            Err(err) => {
                log::warn!("config error: {err}");
                0.0
            }
        }
    };
    (map $key:expr) => {
        match config_store().get($key) {
            Ok(Some(setting)) => setting.to_map(),
            Ok(None) => Vec::new(),
            Err(err) => {
                log::warn!("config error: {err}");
                Vec::new()
            }
        }
    };
}

#[allow(clippy::crate_in_macro_def)]
#[macro_export]
macro_rules! config_set {
    (string $key:expr, $val:expr) => {{
        if let Err(err) = config_store().set($key, Setting::String($val)) {
            log::warn!("config error: {err}");
        }
    }};
    (bool $key:expr, $val:expr) => {{
        if let Err(err) = config_store().set($key, Setting::Bool($val)) {
            log::warn!("config error: {err}");
        }
    }};
    (uint $key:expr, $val:expr) => {{
        if let Err(err) = config_store().set($key, Setting::UInt($val)) {
            log::warn!("config error: {err}");
        }
    }};
    (sint $key:expr, $val:expr) => {{
        if let Err(err) = config_store().set($key, Setting::SInt($val)) {
            log::warn!("config error: {err}");
        }
    }};
    (float $key:expr, $val:expr) => {{
        if let Err(err) = config_store().set($key, Setting::Float($val)) {
            log::warn!("config error: {err}");
        }
    }};
    (map $key:expr, $val:expr) => {{
        if let Err(err) = config_store().set($key, Setting::Map($val)) {
            log::warn!("config error: {err}");
        }
    }};
}

/// `JsonEntry` is used for parsing the settings.json file
#[derive(Debug, Deserialize)]
struct JsonEntry {
    key: String,
    #[serde(rename = "type")]
    _entry_type: String,
    default: String,
    description: String,
    /// Optional comma-separated list of allowed values or ranges (e.g. `left,right` or `-1,0-9999`).
    #[serde(default)]
    values: Option<String>,
}

/// Configuration storage is the place where the gosub engine can find all configurable options
pub struct ConfigStore {
    settings: parking_lot::Mutex<HashMap<String, Setting>>,
    /// A hashmap of all setting descriptions, default values and type information
    settings_info: HashMap<String, SettingInfo>,
    /// Keys of all settings so we can iterate keys easily
    setting_keys: Vec<String>,
    /// The storage adapter used for persisting and loading keys
    storage: Box<dyn StorageAdapter>,
}

impl Default for ConfigStore {
    fn default() -> Self {
        let mut store = ConfigStore {
            settings: parking_lot::Mutex::new(HashMap::new()),
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
                self.settings.lock().insert(key, value);
            }
        }
    }

    /// Returns true when the storage knows about the given key
    pub fn has(&self, key: &str) -> bool {
        self.settings.lock().contains_key(key)
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
    /// return the default value for the given key. Returns `Ok(None)` when the key is unknown.
    pub fn get(&self, key: &str) -> Result<Option<Setting>> {
        if let Some(setting) = self.settings.lock().get(key) {
            return Ok(Some(setting.clone()));
        }

        // Setting not found, try and load it from the storage adapter
        if let Some(setting) = self.storage.get(key)? {
            self.settings.lock().insert(key.to_string(), setting.clone());
            return Ok(Some(setting.clone()));
        }

        // Return the default value for the setting when nothing is found
        if let Some(info) = self.settings_info.get(key) {
            return Ok(Some(info.default.clone()));
        }

        Ok(None)
    }

    /// Sets the given setting to the given value. Will persist the setting to the
    /// storage. Note that the setting MUST have a settings-info entry, otherwise
    /// this function will not store the setting.
    pub fn set(&self, key: &str, value: Setting) -> Result<()> {
        let info = if let Some(info) = self.settings_info.get(key) {
            info
        } else {
            warn!("config: Setting {key} is not known");
            return Err(Error::Config(format!("Setting {key} is not known")));
        };

        if mem::discriminant(&info.default) != mem::discriminant(&value) {
            warn!("config: Setting {key} is of different type than setting expects");
            return Err(Error::Config(format!(
                "Setting {key} is of different type than expected"
            )));
        }

        if let Some(constraint) = &info.constraint {
            if !constraint.allows(&value) {
                warn!("config: Setting {key} value {value} violates its constraint");
                return Err(Error::Config(format!(
                    "Setting {key} value is not allowed by its constraint"
                )));
            }
        }

        self.settings.lock().insert(key.to_owned(), value.clone());
        self.storage.set(key, value)?;
        Ok(())
    }

    /// Removes the stored override for the given key, reverting it back to its default value. The key
    /// MUST have a settings-info entry, otherwise this function returns an error and does nothing.
    pub fn remove(&self, key: &str) -> Result<()> {
        let info = if let Some(info) = self.settings_info.get(key) {
            info
        } else {
            warn!("config: Setting {key} is not known");
            return Err(Error::Config(format!("Setting {key} is not known")));
        };

        self.storage.remove(key)?;
        // Revert the in-memory value back to the default so subsequent reads return the default.
        self.settings.lock().insert(key.to_owned(), info.default.clone());
        Ok(())
    }

    /// Flushes any buffered writes in the underlying storage adapter to its backing store.
    pub fn flush(&self) -> Result<()> {
        self.storage.flush()
    }

    /// Populates the settings in the storage from the settings.json file
    fn populate_default_settings(&mut self) -> Result<()> {
        let json_data: Value = serde_json::from_str(SETTINGS_JSON)?;

        if let Value::Object(data) = json_data {
            for (section_prefix, section_entries) in &data {
                let section_entries: Vec<JsonEntry> = serde_json::from_value(section_entries.clone())?;

                for entry in section_entries {
                    let key = format!("{}.{}", section_prefix, entry.key);

                    let info = SettingInfo {
                        key: key.clone(),
                        description: entry.description,
                        default: Setting::from_str(&entry.default)?,
                        constraint: entry.values.as_deref().and_then(Constraint::parse),
                    };

                    self.setting_keys.push(key.clone());
                    self.settings_info.insert(key.clone(), info.clone());
                    self.settings.lock().insert(key.clone(), info.default.clone());
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

        let setting = config_store().get("dns.local.enabled").unwrap().unwrap();
        assert_eq!(setting, Setting::Bool(true));

        config_store_write()
            .set("dns.local.enabled", Setting::Bool(false))
            .unwrap();
        let setting = config_store().get("dns.local.enabled").unwrap().unwrap();
        assert_eq!(setting, Setting::Bool(false));
    }

    #[test]
    fn invalid_setting() {
        testing_logger::setup();

        testing_logger::validate(|captured_logs| {
            assert_eq!(captured_logs.len(), 0);
        });

        let result = config_store_write().set("dns.local.enabled", Setting::String("wont accept strings".into()));
        assert!(result.is_err());

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
    fn remove_reverts_to_default() {
        // Note: the config store is a global singleton shared across tests, so this test uses a key
        // (`dns.remote.retries`, default u:3) that no other test mutates to avoid cross-test races.
        config_store_write().set_storage(Box::new(MemoryStorageAdapter::new()));

        // Override the default, then remove it again.
        config_store_write().set("dns.remote.retries", Setting::UInt(42)).unwrap();
        assert_eq!(config_store().get("dns.remote.retries").unwrap().unwrap(), Setting::UInt(42));

        config_store_write().remove("dns.remote.retries").unwrap();

        // Back to the default value defined in settings.json.
        assert_eq!(config_store().get("dns.remote.retries").unwrap().unwrap(), Setting::UInt(3));
    }

    #[test]
    fn remove_unknown_key_errors() {
        let result = config_store_write().remove("this.key.doesnt.exist");
        assert!(result.is_err());
    }

    #[test]
    fn flush_is_ok() {
        config_store_write().set_storage(Box::new(MemoryStorageAdapter::new()));
        assert!(config_store().flush().is_ok());
    }

    #[test]
    fn constraint_enum_enforced() {
        config_store_write().set_storage(Box::new(MemoryStorageAdapter::new()));

        // `useragent.tab.close_button` is constrained to `left,right`.
        assert!(config_store().set("useragent.tab.close_button", Setting::Map(vec!["right".into()])).is_ok());
        assert!(config_store()
            .set("useragent.tab.close_button", Setting::Map(vec!["middle".into()]))
            .is_err());
    }

    #[test]
    fn constraint_range_enforced() {
        config_store_write().set_storage(Box::new(MemoryStorageAdapter::new()));

        // `useragent.tab.max_opened` is constrained to `-1,0-9999`.
        assert!(config_store().set("useragent.tab.max_opened", Setting::SInt(100)).is_ok());
        assert!(config_store().set("useragent.tab.max_opened", Setting::SInt(-1)).is_ok());
        assert!(config_store().set("useragent.tab.max_opened", Setting::SInt(10_000)).is_err());
        assert!(config_store().set("useragent.tab.max_opened", Setting::SInt(-5)).is_err());
    }

    #[test]
    fn defaults_satisfy_their_constraints() {
        let store = config_store();
        for (key, info) in &store.settings_info {
            if let Some(constraint) = &info.constraint {
                assert!(
                    constraint.allows(&info.default),
                    "default for {key} violates its own constraint: {:?} not in {:?}",
                    info.default,
                    constraint
                );
            }
        }
    }

    #[test]
    fn unknown_key_returns_none() {
        let result = config_store().get("this.key.doesnt.exist");
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn unknown_key_in_macro_returns_default() {
        config_set!(string "this.key.doesnt.exist", "yesitdoes".into());
        let s = config!(string "this.key.doesnt.exist");
        assert_eq!(s, "");
    }
}
