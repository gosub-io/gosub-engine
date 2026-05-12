//! SQLite-backed cookie store.
//!
//! `SqliteCookieStore` persists **all zones'** cookie jars in a single SQLite
//! database. It implements the [`CookieStore`] trait and returns per-zone jars
//! wrapped in a [`PersistentCookieJar`], so that **every mutation** to a jar
//! triggers a snapshot write back to this store.
//!
//! ## Design
//! - One **table** (`cookies`) for all zones; each row is a single cookie.
//! - In-memory cache: `jars: RwLock<HashMap<ZoneId, CookieJarHandle>>` for quick reuse.
//! - The store keeps a self handle (`store_self`) so persistent jars can call
//!   back into `persist_zone_from_snapshot`.
//! - Database access is via an `r2d2` pool for safe multi-threaded use.
//!
//! ## Concurrency
//! - The store is internally synchronized with `RwLock` and intended to be used
//!   behind a `CookieStoreHandle = Arc<dyn CookieStore + Send + Sync>`.
//! - Each jar handle returned is an `Arc<RwLock<...>>` and may be shared safely
//!   across threads.
//!
//! ## I/O characteristics & caveats
//! - `save_zone` **rewrites** the set of cookies for a zone (DELETE + INSERT).
//! - Several helpers use `expect(...)` and will **panic** on DB errors. Consider
//!   replacing with fallible variants for production.
//!
//! ## Example
//! ```ignore,no_run
//! let store = SqliteCookieStore::new("cookies.sqlite".into()); // -> Arc<SqliteCookieStore>
//!
//! // New zones will receive a PersistentCookieJar minted by this store.
//! let zone_id = engine.zone().cookie_store(store).create()?;
//! ```

use parking_lot::RwLock;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use r2d2::{Pool, PooledConnection};
use r2d2_sqlite::rusqlite::params;
use r2d2_sqlite::SqliteConnectionManager;

use crate::engine::cookies::cookie_jar::DefaultCookieJar;
use crate::engine::cookies::persistent_cookie_jar::PersistentCookieJar;
use crate::engine::cookies::store::CookieStore;
use crate::engine::cookies::{Cookie, CookieJarHandle, CookieStoreHandle};
use crate::engine::zone::ZoneId;

/// A SQLite-based cookie store that persists cookies across sessions.
///
/// Creates per-zone jars on demand, caches them in memory, and snapshots them
/// back to SQLite after each mutation (via [`PersistentCookieJar`]).
pub struct SqliteCookieStore {
    /// Connection pool for SQLite database (so it can run multithreaded)
    pool: Pool<SqliteConnectionManager>,
    /// Cookie jars per zone
    jars: RwLock<HashMap<ZoneId, CookieJarHandle>>,
    /// Self handle provided to persistent jars for callback persistence.
    store_self: RwLock<Option<CookieStoreHandle>>,
}

impl SqliteCookieStore {
    /// Opens (or creates) a SQLite database at `path` and ensures the schema exists.
    ///
    /// Returns an `Arc<Self>` ready to be used as a `CookieStoreHandle`.
    ///
    /// # Panics
    /// Panics if the pool cannot be created or if the `cookies` table cannot be created.
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
            let mut self_ref = store.store_self.write();
            *self_ref = Some(CookieStoreHandle::from(store.clone()));
        }

        store
    }

    /// Borrows a pooled SQLite connection.
    ///
    /// # Panics
    /// Panics if a connection cannot be retrieved from the pool.
    fn conn(&self) -> PooledConnection<SqliteConnectionManager> {
        self.pool.get().expect("Failed to get DB connection")
    }

    /// Loads all cookies for `zone_id` from the database into a new [`DefaultCookieJar`].
    ///
    /// # Panics
    /// Panics on SQL preparation or query errors.
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
        for (origin, entry) in rows.flatten() {
            jar.entries.entry(origin).or_default().push(entry);
        }

        jar
    }

    /// Replaces all cookies for `zone_id` with the contents of `jar` in a transaction.
    ///
    /// DELETEs the existing rows for the zone and INSERTs the new set.
    ///
    /// # Panics
    /// Panics if the transaction, statement preparation, or execution fails.
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

    /// Deletes all cookies for `zone_id` from the database.
    ///
    /// # Panics
    /// Panics on SQL execution error.
    fn remove_zone_from_db(&self, zone_id: ZoneId) {
        let conn = self.conn();
        conn.execute("DELETE FROM cookies WHERE zone_id = ?1", [zone_id.to_string()])
            .expect("Failed to delete zone cookies");
    }
}

impl CookieStore for SqliteCookieStore {
    /// Returns the cookie jar handle for `zone_id`, creating it if needed.
    ///
    /// Behavior:
    /// - If a jar for `zone_id` exists in the in-memory cache, it is returned.
    /// - Otherwise, a serialized jar is loaded from SQLite (if present) or an empty
    ///   [`DefaultCookieJar`] is created.
    /// - That jar is wrapped in a [`PersistentCookieJar`] bound to this store
    ///   (via `store_self`) so that subsequent mutations persist automatically.
    fn jar_for(&self, zone_id: ZoneId) -> Option<CookieJarHandle> {
        {
            let jars = self.jars.read();
            if let Some(jar) = jars.get(&zone_id) {
                return Some(jar.clone());
            }
        }

        let jar = self.load_zone(zone_id);
        let arc_jar: CookieJarHandle = jar.into();

        let store_ref = self.store_self.read();
        let store = store_ref.as_ref().expect("store_self not initialized").clone();

        let persistent = PersistentCookieJar::new(zone_id, arc_jar.clone(), store);
        let handle = CookieJarHandle::new(persistent);

        self.jars.write().insert(zone_id, handle.clone());

        Some(handle)
    }

    /// Persists a snapshot of `zone_id`'s jar to SQLite.
    ///
    /// Called by [`PersistentCookieJar`] after each mutation.
    fn persist_zone_from_snapshot(&self, zone_id: ZoneId, snapshot: &DefaultCookieJar) {
        self.save_zone(zone_id, snapshot);
    }

    /// Removes `zone_id` from both the in-memory cache and the database.
    fn remove_zone(&self, zone_id: ZoneId) {
        self.jars.write().remove(&zone_id);
        self.remove_zone_from_db(zone_id);
    }

    /// Persists **all** in-memory jars to SQLite by snapshotting them.
    ///
    /// Only jars of type [`PersistentCookieJar`] that wrap a [`DefaultCookieJar`]
    /// are snapshotted here to keep the on-disk format stable.
    fn persist_all(&self) {
        let jars = self.jars.read();

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
