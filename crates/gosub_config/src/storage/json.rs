use crate::settings::Setting;
use crate::{Result, StorageAdapter};
use log::warn;
use parking_lot::Mutex;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::fs::File;
use std::io::{Read, Write};

/// JSON file-backed storage adapter. All settings are held in memory and written to a JSON
/// file on the filesystem. Note: `set` currently only updates the in-memory cache; call
/// `write_file` explicitly to persist changes to disk.
pub struct JsonStorageAdapter {
    path: String,
    elements: Mutex<HashMap<String, Setting>>,
}

impl TryFrom<&String> for JsonStorageAdapter {
    type Error = crate::errors::Error;

    fn try_from(path: &String) -> Result<Self> {
        if let Ok(metadata) = fs::metadata(path) {
            if !metadata.is_file() {
                return Err(crate::errors::Error::Config(format!("{path} is not a regular file")));
            }
            File::options().read(true).write(true).open(path)?;
        } else {
            let mut file = File::create(path)?;
            file.write_all(b"{}")?;
        }

        let mut adapter = JsonStorageAdapter {
            path: path.to_string(),
            elements: Mutex::new(HashMap::new()),
        };

        adapter.read_file()?;

        Ok(adapter)
    }
}

impl StorageAdapter for JsonStorageAdapter {
    fn get(&self, key: &str) -> Result<Option<Setting>> {
        let lock = self.elements.lock();
        Ok(lock.get(key).cloned())
    }

    fn set(&self, key: &str, value: Setting) -> Result<()> {
        let mut lock = self.elements.lock();
        lock.insert(key.to_owned(), value);
        Ok(())
    }

    fn all(&self) -> Result<HashMap<String, Setting>> {
        let lock = self.elements.lock();
        Ok(lock.clone())
    }
}

impl JsonStorageAdapter {
    fn read_file(&mut self) -> Result<()> {
        let mut file = File::open(&self.path)?;

        let mut buf = String::new();
        file.read_to_string(&mut buf)?;

        let parsed_json: Value = serde_json::from_str(&buf)?;

        if let Value::Object(settings) = parsed_json {
            let mut lock = self.elements.lock();
            lock.clear();
            for (key, value) in &settings {
                match serde_json::from_value(value.clone()) {
                    Ok(setting) => {
                        lock.insert(key.clone(), setting);
                    }
                    Err(err) => {
                        warn!("problem reading setting from json: {err}");
                    }
                }
            }
        }

        Ok(())
    }

    #[allow(dead_code)]
    fn write_file(&mut self) -> Result<()> {
        let mut file = File::options().write(true).truncate(true).open(&self.path)?;
        let json = serde_json::to_string_pretty(&*self.elements.lock())?;
        file.write_all(json.as_bytes())?;
        Ok(())
    }
}
