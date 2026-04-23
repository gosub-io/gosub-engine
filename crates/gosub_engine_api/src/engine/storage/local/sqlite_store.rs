use anyhow::Result;
use r2d2::{Pool, PooledConnection};
use r2d2_sqlite::rusqlite::{params, OpenFlags};
use r2d2_sqlite::SqliteConnectionManager;
use std::sync::Arc;

use crate::engine::storage::area::{LocalStore, StorageArea};
use crate::engine::storage::types::PartitionKey;
use crate::zone::ZoneId;

/// SQLite-based local storage implementation
pub struct SqliteLocalStore {
    pool: Pool<SqliteConnectionManager>,
}

impl SqliteLocalStore {
    /// Creates a new SQLite local store with the specified database file path.
    pub fn new(path: &str) -> Result<Self> {
        let manager = SqliteConnectionManager::file(path)
            .with_flags(OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE | OpenFlags::SQLITE_OPEN_URI)
            .with_init(|c| {
                c.busy_timeout(std::time::Duration::from_millis(500))?;
                c.pragma_update(None, "journal_mode", "WAL")?;
                c.pragma_update(None, "foreign_keys", "ON")?;
                c.execute_batch(
                    "CREATE TABLE IF NOT EXISTS local_storage (
                        zone TEXT NOT NULL,
                        partition TEXT NOT NULL,
                        origin TEXT NOT NULL,
                        key TEXT NOT NULL,
                        value TEXT NOT NULL,
                        updated_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
                        PRIMARY KEY(zone, partition, origin, key)
                    );",
                )?;
                Ok(())
            });

        let pool = Pool::builder()
            .max_size(16)
            .connection_timeout(std::time::Duration::from_secs(5))
            .build(manager)?;

        Ok(Self { pool })
    }

    #[allow(unused)]
    fn conn(&self) -> Result<PooledConnection<SqliteConnectionManager>> {
        Ok(self.pool.get()?)
    }
}

impl LocalStore for SqliteLocalStore {
    fn area(&self, zone: ZoneId, part: &PartitionKey, origin: &url::Origin) -> Result<Arc<dyn StorageArea>> {
        Ok(Arc::new(SqliteLocalArea {
            pool: self.pool.clone(),
            zone,
            partition: match part {
                PartitionKey::None => "".to_string(),
                PartitionKey::TopLevel(o) => format!("top:{}", o.ascii_serialization()),
                PartitionKey::Custom(s) => s.to_string(),
            },
            origin: origin.ascii_serialization(),
        }))
    }
}

struct SqliteLocalArea {
    pool: Pool<SqliteConnectionManager>,
    zone: ZoneId,
    partition: String,
    origin: String,
}

impl SqliteLocalArea {
    fn conn(&self) -> Result<PooledConnection<SqliteConnectionManager>> {
        Ok(self.pool.get()?)
    }
}

impl StorageArea for SqliteLocalArea {
    fn get_item(&self, key: &str) -> Option<String> {
        let conn = self.conn().ok()?;
        conn.query_row(
            "SELECT value FROM local_storage WHERE zone=?1 AND partition=?2 AND origin=?3 AND key=?4",
            params![self.zone.to_string(), self.partition, self.origin, key],
            |row| row.get::<_, String>(0),
        )
        .ok()
    }

    fn set_item(&self, key: &str, value: &str) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO local_storage(zone,partition,origin,key,value) VALUES (?1,?2,?3,?4,?5)
             ON CONFLICT(zone,partition,origin,key) DO UPDATE
             SET value=excluded.value, updated_at=strftime('%s','now')",
            params![self.zone.to_string(), self.partition, self.origin, key, value],
        )?;
        Ok(())
    }

    fn remove_item(&self, key: &str) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "DELETE FROM local_storage WHERE zone=?1 AND partition=?2 AND origin=?3 AND key=?4",
            params![self.zone.to_string(), self.partition, self.origin, key],
        )?;
        Ok(())
    }

    fn clear(&self) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "DELETE FROM local_storage WHERE zone=?1 AND partition=?2 AND origin=?3",
            params![self.zone.to_string(), self.partition, self.origin],
        )?;
        Ok(())
    }

    fn len(&self) -> usize {
        let conn = match self.conn() {
            Ok(c) => c,
            Err(_) => return 0,
        };
        conn.query_row::<u32, _, _>(
            "SELECT COUNT(*) FROM local_storage WHERE zone=?1 AND partition=?2 AND origin=?3",
            params![self.zone.to_string(), self.partition, self.origin],
            |row| row.get(0),
        )
        .unwrap_or(0) as usize
    }

    fn keys(&self) -> Vec<String> {
        let conn = match self.conn() {
            Ok(c) => c,
            Err(_) => return vec![],
        };
        let mut stmt = match conn
            .prepare("SELECT key FROM local_storage WHERE zone=?1 AND partition=?2 AND origin=?3 ORDER BY key")
        {
            Ok(s) => s,
            Err(_) => return vec![],
        };

        let rows = match stmt.query_map(params![self.zone.to_string(), self.partition, self.origin], |row| {
            row.get::<_, String>(0)
        }) {
            Ok(r) => r,
            Err(_) => return vec![],
        };

        rows.filter_map(Result::ok).collect()
    }
}
