use crate::settings::Setting;
use crate::StorageAdapter;
use gosub_shared::types::Result;
use log::warn;
use rusqlite::{named_params, Connection};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Mutex;

pub struct SqliteStorageAdapter {
    connection: Mutex<Connection>,
}

impl TryFrom<&String> for SqliteStorageAdapter {
    type Error = anyhow::Error;

    fn try_from(path: &String) -> Result<Self> {
        let conn = Connection::open(path).expect("cannot open db file");

        let query = "CREATE TABLE IF NOT EXISTS settings (
            id INTEGER PRIMARY KEY,
            key TEXT NOT NULL,
            value TEXT NOT NULL
        )";
        conn.execute(query, [])?;

        Ok(SqliteStorageAdapter {
            connection: Mutex::new(conn),
        })
    }
}

impl StorageAdapter for SqliteStorageAdapter {
    fn get(&self, key: &str) -> Option<Setting> {
        let db_lock = self.connection.lock().unwrap();

        let query = "SELECT value FROM settings WHERE key = :key";
        let mut statement = db_lock.prepare(query).unwrap();
        let val: String = statement
            .query_row(named_params! { ":key": key }, |row| row.get(0))
            .unwrap();

        match Setting::from_str(&val) {
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
        let _ = statement
            .execute(named_params! {
                ":key": &key.to_string(),
                ":value": &value.to_string(),
            })
            .unwrap();
    }

    fn all(&self) -> Result<HashMap<String, Setting>> {
        let mut settings = HashMap::new();

        let db_lock = self.connection.lock().unwrap();
        let query = "SELECT id,key,value FROM settings";
        let mut statement = db_lock.prepare(query).unwrap();

        let mut rows = statement.query([]).unwrap();
        while let Some(row) = rows.next()? {
            let key: String = row.get(1).unwrap();
            let val: String = row.get(2).unwrap();
            settings.insert(key, Setting::from_str(&val)?);
        }

        Ok(settings)
    }
}
