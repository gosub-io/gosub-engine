use crate::config::settings::Setting;
use crate::config::Store;
use crate::types::{Error, Result};
use log::warn;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::fs::File;
use std::io::{Read, Seek, Write};
use std::sync::{Arc, Mutex};

pub struct JsonStorageAdapter {
    path: String,
    file_mutex: Arc<Mutex<File>>,
    elements: HashMap<String, Setting>,
}

impl TryFrom<&String> for JsonStorageAdapter {
    type Error = Error;

    fn try_from(path: &String) -> Result<Self> {
        let file = match fs::metadata(path) {
            Ok(metadata) => {
                if !metadata.is_file() {
                    panic!("json file is not a regular file")
                }

                File::options()
                    .read(true)
                    .write(true)
                    .open(path)
                    .expect("failed to open json file")
            }
            Err(_) => {
                let json = "{}";

                let mut file = File::create(path).expect("cannot create json file");
                file.write_all(json.as_bytes())?;

                file
            }
        };

        let mut adapter = JsonStorageAdapter {
            path: path.to_string(),
            file_mutex: Arc::new(Mutex::new(file)),
            elements: HashMap::new(),
        };

        adapter.read_file();

        Ok(adapter)
    }
}

impl Store for JsonStorageAdapter {
    fn get_setting(&self, key: &str) -> Option<Setting> {
        self.elements.get(key).cloned()
    }

    fn set_setting(&mut self, key: &str, value: Setting) {
        self.elements.insert(key.to_string(), value);

        self.write_file()
    }

    fn get_all_settings(&self) -> Result<HashMap<String, Setting>> {
        Ok(self.elements.clone())
    }
}

impl JsonStorageAdapter {
    /// Read whole json file and stores the data into self.elements
    fn read_file(&mut self) {
        let mut file = self.file_mutex.lock().expect("Mutex lock failed");

        let mut buf = String::new();
        _ = file.read_to_string(&mut buf);

        let parsed_json: Value = serde_json::from_str(&buf).expect("Failed to parse json");

        if let Value::Object(settings) = parsed_json {
            self.elements = HashMap::new();
            for (key, value) in settings.iter() {
                match serde_json::from_value(value.clone()) {
                    Ok(setting) => {
                        self.elements.insert(key.clone(), setting);
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
    fn write_file(&mut self) {
        let mut file = self.file_mutex.lock().expect("Mutex lock failed");

        let json = serde_json::to_string_pretty(&self.elements).expect("failed to serialize");

        file.set_len(0).expect("failed to truncate file");
        file.seek(std::io::SeekFrom::Start(0))
            .expect("failed to seek");
        file.write_all(json.as_bytes())
            .expect("failed to write file");
    }
}
