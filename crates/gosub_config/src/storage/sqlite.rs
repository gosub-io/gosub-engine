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
        let conn = Connection::open(path)?;

        let query = "CREATE TABLE IF NOT EXISTS settings (
            id INTEGER PRIMARY KEY,
            key TEXT NOT NULL,
            value TEXT NOT NULL
        )";
        conn.execute(query, [])?;

        Ok(Self {
            connection: Mutex::new(conn),
        })
    }
}

#[allow(clippy::significant_drop_tightening)]
impl StorageAdapter for SqliteStorageAdapter {
    fn get(&self, key: &str) -> Option<Setting> {
        let result = {
            let db_lock = self.connection.lock().expect("sqlite connection lock poisoned");
            let query = "SELECT value FROM settings WHERE key = :key";
            let mut statement = db_lock.prepare(query).expect("failed to prepare sqlite query");
            let val: String = statement
                .query_row(named_params! { ":key": key }, |row| row.get(0))
                .ok()?;

            match Setting::from_str(&val) {
                Ok(setting) => Some(setting),
                Err(err) => {
                    warn!("problem reading from sqlite: {err}");
                    None
                }
            }
        };
        result
    }

    fn set(&self, key: &str, value: Setting) {
        let db_lock = self.connection.lock().expect("sqlite connection lock poisoned");
        let query = "INSERT OR REPLACE INTO settings (key, value) VALUES (:key, :value)";
        let Ok(mut statement) = db_lock.prepare(query) else {
            warn!("failed to prepare sqlite insert statement");
            return;
        };
        if let Err(err) = statement.execute(named_params! {
            ":key": &key.to_string(),
            ":value": &value.to_string(),
        }) {
            warn!("failed to insert setting into sqlite: {err}");
        }
    }

    fn all(&self) -> Result<HashMap<String, Setting>> {
        let result = {
            let db_lock = self.connection.lock().expect("sqlite connection lock poisoned");
            let query = "SELECT id,key,value FROM settings";
            let mut statement = db_lock.prepare(query).expect("failed to prepare sqlite query");

            let mut rows = statement.query([]).expect("failed to execute sqlite query");
            let mut res = HashMap::new();
            while let Some(row) = rows.next()? {
                let key: String = row.get(1).expect("failed to get key from sqlite row");
                let val: String = row.get(2).expect("failed to get value from sqlite row");
                res.insert(key, Setting::from_str(&val)?);
            }
            res
        };

        Ok(result)
    }
}
