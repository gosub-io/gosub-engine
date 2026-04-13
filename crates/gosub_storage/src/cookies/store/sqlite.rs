use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use r2d2::{Pool, PooledConnection};
use r2d2_sqlite::rusqlite::params;
use r2d2_sqlite::SqliteConnectionManager;

use crate::cookies::cookie_jar::DefaultCookieJar;
use crate::cookies::persistent_cookie_jar::PersistentCookieJar;
use crate::cookies::store::CookieStore;
use crate::cookies::{Cookie, CookieJarHandle, CookieStoreHandle};
use gosub_net::types::ZoneId;

pub struct SqliteCookieStore {
    pool: Pool<SqliteConnectionManager>,
    jars: RwLock<HashMap<ZoneId, CookieJarHandle>>,
    store_self: RwLock<Option<CookieStoreHandle>>,
}

impl SqliteCookieStore {
    pub fn new(path: PathBuf) -> Arc<Self> {
        let manager = SqliteConnectionManager::file(path);
        let pool = Pool::new(manager).expect("Failed to create SQLite pool");

        {
            let conn = pool.get().expect("DB connection");
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS cookies (
                    zone_id TEXT NOT NULL,
                    origin TEXT NOT NULL,
                    name TEXT NOT NULL,
                    value TEXT NOT NULL,
                    path TEXT,
                    domain TEXT,
                    secure INTEGER NOT NULL,
                    expires TEXT,
                    same_site TEXT,
                    http_only INTEGER NOT NULL,
                    PRIMARY KEY (zone_id, origin, name)
                );",
            )
            .expect("Failed to create cookies table");
        }

        let store = Arc::new(Self {
            pool,
            jars: RwLock::new(HashMap::new()),
            store_self: RwLock::new(None),
        });

        {
            let mut self_ref = store.store_self.write().unwrap();
            *self_ref = Some(CookieStoreHandle::from(store.clone()));
        }

        store
    }

    fn conn(&self) -> PooledConnection<SqliteConnectionManager> {
        self.pool.get().expect("Failed to get DB connection")
    }

    fn load_zone(&self, zone_id: ZoneId) -> DefaultCookieJar {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT origin, name, value, path, domain, secure, expires, same_site, http_only
             FROM cookies WHERE zone_id = ?1",
            )
            .expect("Prepare failed");

        let rows = stmt
            .query_map([zone_id.to_string()], |row| {
                let origin: String = row.get(0)?;
                let entry = Cookie {
                    name: row.get(1)?,
                    value: row.get(2)?,
                    path: row.get(3)?,
                    domain: row.get(4)?,
                    secure: row.get::<_, i64>(5)? != 0,
                    expires: row.get(6)?,
                    same_site: row.get(7)?,
                    http_only: row.get::<_, i64>(8)? != 0,
                };
                Ok((origin, entry))
            })
            .expect("Query failed");

        let mut jar = DefaultCookieJar::new();
        for result in rows {
            if let Ok((origin, entry)) = result {
                jar.entries.entry(origin).or_default().push(entry);
            }
        }
        jar
    }

    fn save_zone(&self, zone_id: ZoneId, jar: &DefaultCookieJar) {
        let mut conn = self.conn();
        let tx = conn.transaction().expect("Transaction failed");

        tx.execute("DELETE FROM cookies WHERE zone_id = ?1", [zone_id.to_string()])
            .expect("Failed to delete cookies");

        let mut stmt = tx.prepare(
            "INSERT INTO cookies (zone_id, origin, name, value, path, domain, secure, expires, same_site, http_only)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)"
        ).expect("Prepare failed");

        for (origin, cookies) in &jar.entries {
            for cookie in cookies {
                stmt.execute(params![
                    zone_id.to_string(),
                    origin,
                    cookie.name,
                    cookie.value,
                    cookie.path,
                    cookie.domain,
                    cookie.secure as i64,
                    cookie.expires,
                    cookie.same_site,
                    cookie.http_only as i64
                ])
                .expect("Failed to insert cookie");
            }
        }

        drop(stmt);
        tx.commit().expect("Commit failed");
    }

    fn remove_zone_from_db(&self, zone_id: ZoneId) {
        let conn = self.conn();
        conn.execute("DELETE FROM cookies WHERE zone_id = ?1", [zone_id.to_string()])
            .expect("Failed to delete zone cookies");
    }
}

impl CookieStore for SqliteCookieStore {
    fn jar_for(&self, zone_id: ZoneId) -> Option<CookieJarHandle> {
        {
            let jars = self.jars.read().unwrap();
            if let Some(jar) = jars.get(&zone_id) {
                return Some(jar.clone());
            }
        }

        let jar = self.load_zone(zone_id);
        let arc_jar: CookieJarHandle = jar.into();

        let store_ref = self.store_self.read().unwrap();
        let store = store_ref.as_ref().expect("store_self not initialized").clone();

        let persistent = PersistentCookieJar::new(zone_id, arc_jar.clone(), store);
        let handle = CookieJarHandle::new(persistent);

        self.jars.write().unwrap().insert(zone_id, handle.clone());
        Some(handle)
    }

    fn persist_zone_from_snapshot(&self, zone_id: ZoneId, snapshot: &DefaultCookieJar) {
        self.save_zone(zone_id, snapshot);
    }

    fn remove_zone(&self, zone_id: ZoneId) {
        self.jars.write().unwrap().remove(&zone_id);
        self.remove_zone_from_db(zone_id);
    }

    fn persist_all(&self) {
        let jars = self.jars.read().unwrap();
        for (zone_id, jar_handle) in jars.iter() {
            let jar = jar_handle.read();
            if let Some(persist) = jar.as_any().downcast_ref::<PersistentCookieJar>() {
                let inner = persist.inner.read();
                if let Some(default) = inner.as_any().downcast_ref::<DefaultCookieJar>() {
                    self.save_zone(*zone_id, default);
                }
            }
        }
    }
}
