pub mod errors;
pub mod settings;
pub mod storage;

pub use errors::Error;
pub(crate) type Result<T> = std::result::Result<T, Error>;

use crate::settings::{Constraint, Setting, SettingInfo};
use crate::storage::MemoryStorageAdapter;
use log::warn;
use parking_lot::RwLock;
use serde_derive::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::mem;
use std::str::FromStr;
use std::sync::Arc;
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

/// Identifies a registered subscription so it can later be removed via [`Config::unsubscribe`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SubscriptionId(u64);

/// Callback invoked when a watched setting changes. It receives the setting key and the new value.
///
/// The callback is invoked after the store's internal lock has been released, so it MAY read or
/// write the same [`Config`] without deadlocking. Beware infinite recursion if a callback sets a
/// key it also subscribes to.
pub type SubscriptionCallback = Arc<dyn Fn(&str, &Setting) + Send + Sync>;

struct Subscription {
    id: SubscriptionId,
    matcher: WildMatch,
    callback: SubscriptionCallback,
}

/// A shareable handle to a configuration store. Cloning is cheap (an `Arc` bump); all clones refer
/// to the same underlying store, so subscriptions and writes made through one clone are visible to
/// the others. This is the per-engine entry point to configuration — construct one and hand clones
/// to whichever components need it.
#[derive(Clone)]
pub struct Config(Arc<RwLock<ConfigStore>>);

impl Default for Config {
    fn default() -> Self {
        Self::in_memory()
    }
}

impl Config {
    /// Creates a config backed by a volatile in-memory store (settings are lost on drop).
    #[must_use]
    pub fn in_memory() -> Self {
        Config(Arc::new(RwLock::new(ConfigStore::default())))
    }

    /// Creates a config backed by the given storage adapter, preloading any persisted settings.
    #[must_use]
    pub fn with_storage(storage: Box<dyn StorageAdapter>) -> Self {
        let config = Self::in_memory();
        config.set_storage(storage);
        config
    }

    /// Swaps in a new storage adapter, loading its persisted settings over the current ones.
    pub fn set_storage(&self, storage: Box<dyn StorageAdapter>) {
        self.0.write().set_storage(storage);
    }

    /// Returns the setting for the given key, falling back to the default, or `Ok(None)` when the
    /// key is unknown. See [`ConfigStore::get`].
    pub fn get(&self, key: &str) -> Result<Option<Setting>> {
        self.0.read().get(key)
    }

    /// Sets a setting, persisting it and notifying any matching subscribers when the value changes.
    pub fn set(&self, key: &str, value: Setting) -> Result<()> {
        // Mutate under the lock, collect the callbacks to fire, then release the lock *before*
        // invoking them so callbacks can freely re-enter the store.
        let fire = {
            let store = self.0.write();
            store.set(key, value)?.map(|value| {
                let callbacks = store.matching_callbacks(key);
                (value, callbacks)
            })
        };
        if let Some((value, callbacks)) = fire {
            for callback in callbacks {
                callback(key, &value);
            }
        }
        Ok(())
    }

    /// Removes the override for a key, reverting to its default and notifying matching subscribers
    /// when the value changes.
    pub fn remove(&self, key: &str) -> Result<()> {
        let fire = {
            let store = self.0.write();
            store.remove(key)?.map(|default| {
                let callbacks = store.matching_callbacks(key);
                (default, callbacks)
            })
        };
        if let Some((value, callbacks)) = fire {
            for callback in callbacks {
                callback(key, &value);
            }
        }
        Ok(())
    }

    /// Flushes any buffered writes in the underlying storage adapter.
    pub fn flush(&self) -> Result<()> {
        self.0.read().flush()
    }

    /// Returns true when the store knows about the given key.
    #[must_use]
    pub fn has(&self, key: &str) -> bool {
        self.0.read().has(key)
    }

    /// Returns the keys matching the given wildcard search (`*`/`?`).
    #[must_use]
    pub fn find(&self, search: &str) -> Vec<String> {
        self.0.read().find(search)
    }

    /// Returns metadata (description, default, constraint) for the given key.
    #[must_use]
    pub fn get_info(&self, key: &str) -> Option<SettingInfo> {
        self.0.read().get_info(key)
    }

    /// Subscribes to changes on settings whose key matches `pattern` (a [`WildMatch`] pattern, so
    /// `*`/`?` wildcards work, e.g. `dns.*` or `*`). The callback fires whenever a matching
    /// setting's value actually changes via `set` or `remove`. Returns an id used to unsubscribe.
    pub fn subscribe<F>(&self, pattern: &str, callback: F) -> SubscriptionId
    where
        F: Fn(&str, &Setting) + Send + Sync + 'static,
    {
        self.0.write().subscribe(pattern, callback)
    }

    /// Removes a previously registered subscription. Returns true when a subscription was removed.
    pub fn unsubscribe(&self, id: SubscriptionId) -> bool {
        self.0.write().unsubscribe(id)
    }

    /// Reads a boolean setting, returning `false` when the key is unknown or a storage error occurs.
    #[must_use]
    pub fn get_bool(&self, key: &str) -> bool {
        self.typed_get(key, false, Setting::to_bool)
    }

    /// Reads an unsigned integer setting, returning `0` on unknown key or storage error.
    #[must_use]
    pub fn get_uint(&self, key: &str) -> usize {
        self.typed_get(key, 0, Setting::to_uint)
    }

    /// Reads a signed integer setting, returning `0` on unknown key or storage error.
    #[must_use]
    pub fn get_sint(&self, key: &str) -> isize {
        self.typed_get(key, 0, Setting::to_sint)
    }

    /// Reads a float setting, returning `0.0` on unknown key or storage error.
    #[must_use]
    pub fn get_float(&self, key: &str) -> f64 {
        self.typed_get(key, 0.0, Setting::to_float)
    }

    /// Reads a string setting, returning an empty string on unknown key or storage error.
    #[must_use]
    pub fn get_string(&self, key: &str) -> String {
        self.typed_get(key, String::new(), Setting::to_string)
    }

    /// Reads a map setting, returning an empty vector on unknown key or storage error.
    #[must_use]
    pub fn get_map(&self, key: &str) -> Vec<String> {
        self.typed_get(key, Vec::new(), Setting::to_map)
    }

    /// Shared helper for the typed getters: reads the setting, applies `convert`, or returns
    /// `default` when the key is unknown (logging on a storage error).
    fn typed_get<T>(&self, key: &str, default: T, convert: impl Fn(&Setting) -> T) -> T {
        match self.get(key) {
            Ok(Some(setting)) => convert(&setting),
            Ok(None) => default,
            Err(err) => {
                warn!("config error: {err}");
                default
            }
        }
    }
}

/// Grants access to a [`Config`] handle. Subsystems that only need to read or watch settings
/// should bound on `T: HasConfig` rather than taking a concrete context type, so they stay
/// decoupled from how the engine is assembled.
///
/// A bare [`Config`] implements this (returning itself), and a runtime context that owns a
/// `Config` implements it by returning a reference to that field.
pub trait HasConfig {
    /// Returns the configuration handle.
    fn config(&self) -> &Config;
}

impl HasConfig for Config {
    fn config(&self) -> &Config {
        self
    }
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
    /// Registered change subscriptions, notified when a matching setting changes
    subscriptions: Vec<Subscription>,
    /// Monotonic counter used to hand out unique `SubscriptionId`s
    next_subscription_id: u64,
}

impl Default for ConfigStore {
    fn default() -> Self {
        let mut store = ConfigStore {
            settings: parking_lot::Mutex::new(HashMap::new()),
            settings_info: HashMap::new(),
            setting_keys: Vec::new(),
            storage: Box::new(MemoryStorageAdapter::new()),
            subscriptions: Vec::new(),
            next_subscription_id: 0,
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

    /// Sets the given setting to the given value and persists it. The setting MUST have a
    /// settings-info entry and satisfy its type and constraint, otherwise an error is returned.
    /// Returns `Ok(Some(value))` when the value actually changed (so the caller should notify
    /// subscribers), or `Ok(None)` when the value was already set to `value`.
    pub fn set(&self, key: &str, value: Setting) -> Result<Option<Setting>> {
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

        let changed = {
            let mut settings = self.settings.lock();
            let changed = settings.get(key) != Some(&value);
            settings.insert(key.to_owned(), value.clone());
            changed
        };
        self.storage.set(key, value.clone())?;

        Ok(changed.then_some(value))
    }

    /// Removes the stored override for the given key, reverting it back to its default value. The key
    /// MUST have a settings-info entry, otherwise this function returns an error and does nothing.
    /// Returns `Ok(Some(default))` when the value actually changed, or `Ok(None)` otherwise.
    pub fn remove(&self, key: &str) -> Result<Option<Setting>> {
        let info = if let Some(info) = self.settings_info.get(key) {
            info
        } else {
            warn!("config: Setting {key} is not known");
            return Err(Error::Config(format!("Setting {key} is not known")));
        };

        self.storage.remove(key)?;
        // Revert the in-memory value back to the default so subsequent reads return the default.
        let default = info.default.clone();
        let changed = {
            let mut settings = self.settings.lock();
            let changed = settings.get(key) != Some(&default);
            settings.insert(key.to_owned(), default.clone());
            changed
        };

        Ok(changed.then_some(default))
    }

    /// Flushes any buffered writes in the underlying storage adapter to its backing store.
    pub fn flush(&self) -> Result<()> {
        self.storage.flush()
    }

    /// Subscribes to changes on settings whose key matches `pattern` (a [`WildMatch`] pattern, so
    /// `*`/`?` wildcards work). Returns an id that can be passed to [`ConfigStore::unsubscribe`].
    /// See [`SubscriptionCallback`] for the constraints that apply to the callback.
    pub fn subscribe<F>(&mut self, pattern: &str, callback: F) -> SubscriptionId
    where
        F: Fn(&str, &Setting) + Send + Sync + 'static,
    {
        let id = SubscriptionId(self.next_subscription_id);
        self.next_subscription_id += 1;
        self.subscriptions.push(Subscription {
            id,
            matcher: WildMatch::new(pattern),
            callback: Arc::new(callback),
        });
        id
    }

    /// Removes a previously registered subscription. Returns true when a subscription was removed.
    pub fn unsubscribe(&mut self, id: SubscriptionId) -> bool {
        let before = self.subscriptions.len();
        self.subscriptions.retain(|sub| sub.id != id);
        self.subscriptions.len() != before
    }

    /// Returns clones of the callbacks for every subscription whose pattern matches `key`. The
    /// caller invokes these after releasing the store lock so callbacks may re-enter the store.
    fn matching_callbacks(&self, key: &str) -> Vec<SubscriptionCallback> {
        self.subscriptions
            .iter()
            .filter(|sub| sub.matcher.matches(key))
            .map(|sub| sub.callback.clone())
            .collect()
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
    use parking_lot::Mutex;
    use std::sync::Arc;

    #[test]
    fn get_and_set() {
        let cfg = Config::in_memory();

        let setting = cfg.get("dns.local.enabled").unwrap().unwrap();
        assert_eq!(setting, Setting::Bool(true));

        cfg.set("dns.local.enabled", Setting::Bool(false)).unwrap();
        assert_eq!(cfg.get("dns.local.enabled").unwrap().unwrap(), Setting::Bool(false));
    }

    #[test]
    fn invalid_setting() {
        testing_logger::setup();

        testing_logger::validate(|captured_logs| {
            assert_eq!(captured_logs.len(), 0);
        });

        let cfg = Config::in_memory();
        let result = cfg.set("dns.local.enabled", Setting::String("wont accept strings".into()));
        assert!(result.is_err());

        testing_logger::validate(|captured_logs| {
            assert_eq!(captured_logs.len(), 1);
            assert_eq!(captured_logs[0].level, log::Level::Warn);
        });
    }

    #[test]
    fn typed_accessors() {
        let cfg = Config::in_memory();

        cfg.set("dns.cache.max_entries", Setting::UInt(9432)).unwrap();
        assert_eq!(cfg.get_uint("dns.cache.max_entries"), 9432);

        assert!(cfg.get_bool("dns.local.enabled"));
        assert_eq!(cfg.get_string("useragent.default_page"), "about:blank");
    }

    #[test]
    fn typed_getter_unknown_returns_default() {
        let cfg = Config::in_memory();
        assert_eq!(cfg.get_string("this.key.doesnt.exist"), "");
        assert_eq!(cfg.get_uint("this.key.doesnt.exist"), 0);
        assert!(!cfg.get_bool("this.key.doesnt.exist"));
    }

    #[test]
    fn remove_reverts_to_default() {
        let cfg = Config::in_memory();

        cfg.set("dns.remote.retries", Setting::UInt(42)).unwrap();
        assert_eq!(cfg.get("dns.remote.retries").unwrap().unwrap(), Setting::UInt(42));

        cfg.remove("dns.remote.retries").unwrap();
        assert_eq!(cfg.get("dns.remote.retries").unwrap().unwrap(), Setting::UInt(3));
    }

    #[test]
    fn remove_unknown_key_errors() {
        let cfg = Config::in_memory();
        assert!(cfg.remove("this.key.doesnt.exist").is_err());
    }

    #[test]
    fn flush_is_ok() {
        let cfg = Config::in_memory();
        assert!(cfg.flush().is_ok());
    }

    #[test]
    fn unknown_key_returns_none() {
        let cfg = Config::in_memory();
        assert!(cfg.get("this.key.doesnt.exist").unwrap().is_none());
    }

    #[test]
    fn has_config_accessor() {
        // A subsystem bounded on `HasConfig` can read settings without knowing the concrete type.
        fn retries<T: HasConfig>(ctx: &T) -> usize {
            ctx.config().get_uint("dns.remote.retries")
        }

        let cfg = Config::in_memory();
        cfg.set("dns.remote.retries", Setting::UInt(11)).unwrap();
        assert_eq!(retries(&cfg), 11);
    }

    #[test]
    fn separate_configs_are_isolated() {
        let a = Config::in_memory();
        let b = Config::in_memory();

        a.set("dns.local.enabled", Setting::Bool(false)).unwrap();

        // `b` is a wholly separate store and keeps the default.
        assert!(!a.get_bool("dns.local.enabled"));
        assert!(b.get_bool("dns.local.enabled"));
    }

    /// Captures `(key, value)` pairs delivered to a subscription callback.
    #[allow(clippy::type_complexity)]
    fn capturing_callback() -> (Arc<Mutex<Vec<(String, Setting)>>>, impl Fn(&str, &Setting) + Send + Sync) {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let sink = captured.clone();
        let cb = move |key: &str, value: &Setting| {
            sink.lock().push((key.to_string(), value.clone()));
        };
        (captured, cb)
    }

    #[test]
    fn subscribe_fires_on_change() {
        let cfg = Config::in_memory();
        let (captured, cb) = capturing_callback();
        cfg.subscribe("dns.remote.doh.enabled", cb);

        // default is false -> setting true is a real change
        cfg.set("dns.remote.doh.enabled", Setting::Bool(true)).unwrap();

        assert_eq!(
            *captured.lock(),
            vec![("dns.remote.doh.enabled".to_string(), Setting::Bool(true))]
        );
    }

    #[test]
    fn subscribe_only_fires_on_actual_change() {
        let cfg = Config::in_memory();
        let (captured, cb) = capturing_callback();
        cfg.subscribe("dns.remote.timeout", cb);

        // default is u:5 -> setting 5 again is not a change, 7 is
        cfg.set("dns.remote.timeout", Setting::UInt(5)).unwrap();
        cfg.set("dns.remote.timeout", Setting::UInt(7)).unwrap();

        assert_eq!(*captured.lock(), vec![("dns.remote.timeout".to_string(), Setting::UInt(7))]);
    }

    #[test]
    fn subscribe_wildcard_matches() {
        let cfg = Config::in_memory();
        let (captured, cb) = capturing_callback();
        cfg.subscribe("*", cb);

        cfg.set("renderer.opengl.enabled", Setting::Bool(false)).unwrap();
        cfg.set("dns.remote.retries", Setting::UInt(9)).unwrap();

        assert_eq!(
            *captured.lock(),
            vec![
                ("renderer.opengl.enabled".to_string(), Setting::Bool(false)),
                ("dns.remote.retries".to_string(), Setting::UInt(9)),
            ]
        );
    }

    #[test]
    fn remove_notifies_with_default() {
        let cfg = Config::in_memory();
        let (captured, cb) = capturing_callback();
        cfg.subscribe("dns.cache.ttl.override.seconds", cb);

        // default u:0 -> set 99 (change), then remove (revert to default 0, also a change)
        cfg.set("dns.cache.ttl.override.seconds", Setting::UInt(99)).unwrap();
        cfg.remove("dns.cache.ttl.override.seconds").unwrap();

        assert_eq!(
            *captured.lock(),
            vec![
                ("dns.cache.ttl.override.seconds".to_string(), Setting::UInt(99)),
                ("dns.cache.ttl.override.seconds".to_string(), Setting::UInt(0)),
            ]
        );
    }

    #[test]
    fn unsubscribe_stops_notifications() {
        let cfg = Config::in_memory();
        let (captured, cb) = capturing_callback();
        let id = cfg.subscribe("useragent.default_page", cb);
        assert!(cfg.unsubscribe(id));

        cfg.set("useragent.default_page", Setting::String("about:config".into())).unwrap();

        assert!(captured.lock().is_empty());
    }

    #[test]
    fn callback_can_reenter_the_store() {
        // The callback writes a *different* key from within the notification. This only works
        // because notifications fire after the store lock is released.
        let cfg = Config::in_memory();
        let inner = cfg.clone();
        cfg.subscribe("dns.remote.doh.enabled", move |_key, _value| {
            inner.set("dns.remote.dot.enabled", Setting::Bool(true)).unwrap();
        });

        cfg.set("dns.remote.doh.enabled", Setting::Bool(true)).unwrap();

        assert!(cfg.get_bool("dns.remote.dot.enabled"));
    }

    #[test]
    fn constraint_enum_enforced() {
        let cfg = Config::in_memory();

        // `useragent.tab.close_button` is constrained to `left,right`.
        assert!(cfg.set("useragent.tab.close_button", Setting::Map(vec!["right".into()])).is_ok());
        assert!(cfg.set("useragent.tab.close_button", Setting::Map(vec!["middle".into()])).is_err());
    }

    #[test]
    fn constraint_range_enforced() {
        let cfg = Config::in_memory();

        // `useragent.tab.max_opened` is constrained to `-1,0-9999`.
        assert!(cfg.set("useragent.tab.max_opened", Setting::SInt(100)).is_ok());
        assert!(cfg.set("useragent.tab.max_opened", Setting::SInt(-1)).is_ok());
        assert!(cfg.set("useragent.tab.max_opened", Setting::SInt(10_000)).is_err());
        assert!(cfg.set("useragent.tab.max_opened", Setting::SInt(-5)).is_err());
    }

    #[test]
    fn defaults_satisfy_their_constraints() {
        let cfg = Config::in_memory();
        let store = cfg.0.read();
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
}
