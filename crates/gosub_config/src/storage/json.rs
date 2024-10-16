use crate::settings::Setting;
use crate::StorageAdapter;
use gosub_shared::types::Result;
use log::warn;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::fs::File;
use std::io::{Read, Seek, Write};
use std::sync::Mutex;

pub struct JsonStorageAdapter {
    path: String,
    elements: Mutex<HashMap<String, Setting>>,
}

impl TryFrom<&String> for JsonStorageAdapter {
    type Error = anyhow::Error;

    fn try_from(path: &String) -> Result<Self> {
        let _ = if let Ok(metadata) = fs::metadata(path) {
            assert!(metadata.is_file(), "json file is not a regular file");

            File::options()
                .read(true)
                .write(true)
                .open(path)
                .expect("failed to open json file")
        } else {
            let json = "{}";

            let mut file = File::create(path).expect("cannot create json file");
            file.write_all(json.as_bytes())?;

            file
        };

        let mut adapter = JsonStorageAdapter {
            path: path.to_string(),
            elements: Mutex::new(HashMap::new()),
        };

        adapter.read_file();

        Ok(adapter)
    }
}

impl StorageAdapter for JsonStorageAdapter {
    fn get(&self, key: &str) -> Option<Setting> {
        let lock = self.elements.lock().expect("Poisoned");
        lock.get(key).cloned()
    }

    fn set(&self, key: &str, value: Setting) {
        let mut lock = self.elements.lock().expect("Poisoned");
        lock.insert(key.to_owned(), value);

        // self.write_file()
    }

    fn all(&self) -> Result<HashMap<String, Setting>> {
        let lock = self.elements.lock().expect("Poisoned");

        Ok(lock.clone())
    }
}

impl JsonStorageAdapter {
    /// Read whole json file and stores the data into self.elements
    fn read_file(&mut self) {
        // @TODO: We should have some kind of OS file lock here
        let mut file = File::open(&self.path).expect("failed to open json file");

        let mut buf = String::new();
        let _ = file.read_to_string(&mut buf);

        let parsed_json: Value = serde_json::from_str(&buf).expect("Failed to parse json");

        if let Value::Object(settings) = parsed_json {
            self.elements = Mutex::new(HashMap::new());

            let mut lock = self.elements.lock().expect("Poisoned");
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
    }

    /// Write the self.elements hashmap back to the file by truncating the file and writing the
    /// data again.
    #[allow(dead_code)]
    fn write_file(&mut self) {
        // @TODO: We need some kind of OS lock file here. We should protect against concurrent threads but also
        // against concurrent processes.
        let mut file = File::open(&self.path).expect("failed to open json file");

        let json = serde_json::to_string_pretty(&self.elements).expect("failed to serialize");

        file.set_len(0).expect("failed to truncate file");
        file.seek(std::io::SeekFrom::Start(0)).expect("failed to seek");
        file.write_all(json.as_bytes()).expect("failed to write file");
    }
}
