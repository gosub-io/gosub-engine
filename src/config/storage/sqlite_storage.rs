use crate::config::settings::Setting;
use crate::config::StorageAdapter;
use std::collections::HashMap;

pub struct SqliteStorageAdapter {
    connection: sqlite::Connection,
}

impl SqliteStorageAdapter {
    pub fn new(path: &str) -> Self {
        let conn = sqlite::open(path).expect("cannot open db file");

        let query = "CREATE TABLE IF NOT EXISTS settings (
            id INTEGER PRIMARY KEY,
            key TEXT NOT NULL,
            value TEXT NOT NULL
        )";
        conn.execute(query).unwrap();

        SqliteStorageAdapter { connection: conn }
    }
}

impl StorageAdapter for SqliteStorageAdapter {
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
        statement
            .bind((":value", value.to_string().as_str()))
            .unwrap();

        statement.next().unwrap();
    }

    fn get_all_settings(&self) -> HashMap<String, Setting> {
        let query = "SELECT * FROM settings";
        let mut statement = self.connection.prepare(query).unwrap();

        let mut settings = HashMap::new();
        while let sqlite::State::Row = statement.next().unwrap() {
            let key = statement.read::<String, _>(1).unwrap();
            let value = statement.read::<String, _>(2).unwrap();
            settings.insert(key, Setting::from_string(value.as_str()).unwrap());
        }

        settings
    }
}
