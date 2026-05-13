use crate::errors::Error;
use crate::settings::Setting;
use crate::{Result, StorageAdapter};
use log::warn;
use parking_lot::Mutex;
use rusqlite::{named_params, Connection};
use std::collections::HashMap;
use std::str::FromStr;

pub struct SqliteStorageAdapter {
    connection: Mutex<Connection>,
}

impl TryFrom<&String> for SqliteStorageAdapter {
    type Error = Error;

    fn try_from(path: &String) -> Result<Self> {
        let conn = Connection::open(path)?;

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
    fn get(&self, key: &str) -> Result<Option<Setting>> {
        let db_lock = self.connection.lock();
        let query = "SELECT value FROM settings WHERE key = :key";
        let mut statement = db_lock.prepare(query)?;

        match statement.query_row(named_params! { ":key": key }, |row| row.get::<_, String>(0)) {
            Ok(val) => match Setting::from_str(&val) {
                Ok(setting) => Ok(Some(setting)),
                Err(err) => {
                    warn!("problem reading from sqlite: {err}");
                    Ok(None)
                }
            },
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(err) => Err(Error::Sqlite(err)),
        }
    }

    fn set(&self, key: &str, value: Setting) -> Result<()> {
        let db_lock = self.connection.lock();
        let query = "INSERT OR REPLACE INTO settings (key, value) VALUES (:key, :value)";
        let mut statement = db_lock.prepare(query)?;
        statement.execute(named_params! {
            ":key": key,
            ":value": value.to_string(),
        })?;
        Ok(())
    }

    fn all(&self) -> Result<HashMap<String, Setting>> {
        let mut settings = HashMap::new();

        let db_lock = self.connection.lock();
        let query = "SELECT id,key,value FROM settings";
        let mut statement = db_lock.prepare(query)?;

        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let key: String = row.get(1)?;
            let val: String = row.get(2)?;
            settings.insert(key, Setting::from_str(&val)?);
        }

        Ok(settings)
    }
}
