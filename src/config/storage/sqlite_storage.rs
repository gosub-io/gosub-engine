use std::collections::HashMap;
use crate::config::settings::Setting;
use crate::config::StorageAdapter;

struct SqlStorageAdapter {
    connection: sqlite::Connection
}

impl SqlStorageAdapter {
    fn new(path: String) -> Self {
        let conn = sqlite::open(path).expect("cannot open db file");

        let query = "CREATE TABLE IF NOT EXISTS settings (
            id INTEGER PRIMARY KEY,
            key TEXT NOT NULL,
            value TEXT NOT NULL,
        )";
        conn.execute(query).unwrap();

        SqlStorageAdapter {
            connection: conn
        }

    }
}

impl StorageAdapter for SqlStorageAdapter
{
    fn get_setting(&self, key: &str) -> Option<Setting> {
        let query = "SELECT * FROM settings WHERE key = :key";
        let mut statement = self.connection.prepare(query).unwrap();
        statement.bind((":key", key)).unwrap();

        Setting::from_string(key)
    }

    fn set_setting(&mut self, key: &str, value: Setting) {
        let query = "INSERT OR REPLACE INTO settings (key, value) VALUES (:key, :value)";
        let mut statement = self.connection.prepare(query).unwrap();
        statement.bind((":key", key)).unwrap();
        statement.bind((":value", value.to_string().as_str())).unwrap();

        statement.next().unwrap();
    }

    fn get_all_settings(&self) -> HashMap<String, Setting> {
        todo!()
    }
}
