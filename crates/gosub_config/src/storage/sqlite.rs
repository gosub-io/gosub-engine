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

        Ok(Self {
            connection: Mutex::new(conn),
        })
    }
}

impl StorageAdapter for SqliteStorageAdapter {
    fn get(&self, key: &str) -> Option<Setting> {
        let db_lock = self.connection.lock().unwrap();

        let query = "SELECT * FROM settings WHERE key = :key";
        let mut statement = db_lock.prepare(query).unwrap();
        statement.bind((":key", key)).unwrap();

        match Setting::from_str(key) {
            Ok(setting) => Some(setting),
            Err(err) => {
                warn!("problem reading from sqlite: {err}");
                None
            }
        }
    }

    fn set(&self, key: &str, value: Setting) {
        let db_lock = self.connection.lock().unwrap();

        let query = "INSERT OR REPLACE INTO settings (key, value) VALUES (:key, :value)";
        let mut statement = db_lock.prepare(query).unwrap();
        statement.bind((":key", key)).unwrap();
        statement
            .bind((":value", value.to_string().as_str()))
            .unwrap();

        statement.next().unwrap();
    }

    fn all(&self) -> Result<HashMap<String, Setting>> {
        let db_lock = self.connection.lock().unwrap();

        let query = "SELECT * FROM settings";
        let mut statement = db_lock.prepare(query).unwrap();

        let mut settings = HashMap::new();
        while statement.next().unwrap() == sqlite::State::Row {
            let key = statement.read::<String, _>(1).unwrap();
            let value = statement.read::<String, _>(2).unwrap();
            settings.insert(key, Setting::from_str(&value)?);
        }

        Ok(settings)
    }
}
