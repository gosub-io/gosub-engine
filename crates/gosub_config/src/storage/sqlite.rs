use crate::settings::Setting;
use crate::StorageAdapter;
use gosub_shared::types::Result;
use log::warn;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Mutex;

pub struct SqliteStorageAdapter {
    connection: Mutex<sqlite::Connection>,
}

impl TryFrom<&String> for SqliteStorageAdapter {
    type Error = anyhow::Error;

    fn try_from(path: &String) -> Result<Self> {
        let conn = sqlite::open(path).expect("cannot open db file");

        let query = "CREATE TABLE IF NOT EXISTS settings (
            id INTEGER PRIMARY KEY,
            key TEXT NOT NULL,
            value TEXT NOT NULL
        )";
        conn.execute(query)?;

        Ok(SqliteStorageAdapter {
            connection: Mutex::new(conn),
        })
    }
}

impl StorageAdapter for SqliteStorageAdapter {
    fn get(&self, key: &str) -> Option<Setting> {
        let db_lock = match self.connection.lock() {
            Ok(l) => {l}
            Err(e) => {
                warn!("Poisoned mutex {e}");
                return None
            }
        };

        let query = "SELECT * FROM settings WHERE key = :key";
        // If any of these sqlite commands fail at any point,
        // Then we return a None
        let mut statement = match db_lock.prepare(query) {
            Ok(s) => {s}
            Err(e) => {
                warn!("problem preparing statement: {e}");
                return None
            }
        };
        match statement.bind((":key", key)) {
            Ok(_) => {}
            Err(e) => {
                warn!("problem binding statement: {e}");
                return None
            }
        };

        match Setting::from_str(key) {
            Ok(setting) => Some(setting),
            Err(e) => {
                warn!("problem reading from sqlite: {e}");
                None
            }
        }
    }

    fn set(&self, key: &str, value: Setting) {
        let db_lock = self.connection.lock().expect("Poisoned");

        let query = "INSERT OR REPLACE INTO settings (key, value) VALUES (:key, :value)";
        let mut statement = db_lock.prepare(query).expect("Poisoned");
        statement.bind((":key", key)).expect("Failed to bind");
        statement.bind((":value", value.to_string().as_str())).expect("Failed to bind");

        statement.next().expect("Failed to execute the set");
    }

    fn all(&self) -> Result<HashMap<String, Setting>> {
        let db_lock = self.connection.lock().expect("Poisoned");

        let query = "SELECT * FROM settings";
        let mut statement = db_lock.prepare(query)?;

        let mut settings = HashMap::new();
        while let sqlite::State::Row = statement.next()? {
            let key = statement.read::<String, _>(1)?;
            let value = statement.read::<String, _>(2)?;
            settings.insert(key, Setting::from_str(&value)?);
        }

        Ok(settings)
    }
}
