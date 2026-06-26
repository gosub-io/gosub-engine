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
//! - Persistence is best-effort: database errors are logged, never panicked on.
//!
//! ## Example
//! ```ignore,no_run
//! let store = SqliteCookieStore::new("cookies.sqlite".into())?; // -> Arc<SqliteCookieStore>
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

use crate::engine::cookies::cookie_jar::{CookieJar, DefaultCookieJar};
use crate::engine::cookies::persistent_cookie_jar::PersistentCookieJar;
use crate::engine::cookies::store::CookieStore;
use crate::engine::cookies::{Cookie, CookieJarHandle, CookieStoreHandle};
use crate::engine::zone::ZoneId;
use crate::EngineError;

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
    /// # Errors
    /// Returns [`EngineError::CookieStore`] if the pool cannot be created or the
    /// `cookies` table cannot be created/migrated.
    pub fn new(path: PathBuf) -> Result<Arc<Self>, EngineError> {
        let manager = SqliteConnectionManager::file(path);
        let pool = Pool::new(manager).map_err(|e| EngineError::CookieStore(e.into()))?;

        {
            let conn = pool.get().map_err(|e| EngineError::CookieStore(e.into()))?;

            // Drop table if the expires column is TEXT (pre-i64 schema).
            let old_schema = conn
                .query_row(
                    "SELECT COUNT(*) FROM pragma_table_info('cookies') \
                     WHERE name='expires' AND type='TEXT'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap_or(0)
                > 0;
            if old_schema {
                conn.execute_batch("DROP TABLE IF EXISTS cookies;")
                    .map_err(|e| EngineError::CookieStore(e.into()))?;
            }

            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS cookies (
                    zone_id    TEXT    NOT NULL,
                    origin     TEXT    NOT NULL,
                    name       TEXT    NOT NULL,
                    value      TEXT    NOT NULL,
                    path       TEXT,
                    domain     TEXT,
                    secure     INTEGER NOT NULL,
                    expires    INTEGER,
                    same_site  TEXT,
                    http_only  INTEGER NOT NULL,
                    created_at INTEGER NOT NULL DEFAULT 0,
                    PRIMARY KEY (zone_id, origin, name, path, domain)
                );",
            )
            .map_err(|e| EngineError::CookieStore(e.into()))?;

            // Add created_at to any pre-existing table that lacks it.
            let _ = conn.execute_batch("ALTER TABLE cookies ADD COLUMN created_at INTEGER NOT NULL DEFAULT 0;");
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

        Ok(store)
    }

    /// Borrows a pooled SQLite connection, logging on failure.
    fn conn(&self) -> Option<PooledConnection<SqliteConnectionManager>> {
        match self.pool.get() {
            Ok(conn) => Some(conn),
            Err(e) => {
                log::error!("Failed to get cookie DB connection: {e}");
                None
            }
        }
    }

    /// Loads all cookies for `zone_id` from the database into a new [`DefaultCookieJar`].
    ///
    /// Best-effort: on database errors an empty jar is returned and the error is logged.
    fn load_zone(&self, zone_id: ZoneId) -> DefaultCookieJar {
        let mut jar = DefaultCookieJar::new();
        let Some(conn) = self.conn() else {
            return jar;
        };

        let mut stmt = match conn.prepare(
            "SELECT origin, name, value, path, domain, secure, expires, same_site, http_only, created_at
             FROM cookies WHERE zone_id = ?1",
        ) {
            Ok(stmt) => stmt,
            Err(e) => {
                log::error!("Failed to prepare cookie SELECT: {e}");
                return jar;
            }
        };

        let rows = match stmt.query_map([zone_id.to_string()], |row| {
            let origin: String = row.get(0)?;
            let entry = Cookie {
                name: row.get(1)?,
                value: row.get(2)?,
                path: row.get(3)?,
                domain: row.get(4)?,
                secure: row.get::<_, i64>(5)? != 0,
                expires: row.get::<_, Option<i64>>(6)?,
                same_site: row.get(7)?,
                http_only: row.get::<_, i64>(8)? != 0,
                created_at: row.get::<_, i64>(9).unwrap_or(0),
            };
            Ok((origin, entry))
        }) {
            Ok(rows) => rows,
            Err(e) => {
                log::error!("Failed to query cookies for zone {zone_id}: {e}");
                return jar;
            }
        };

        for (origin, entry) in rows.flatten() {
            jar.entries.entry(origin).or_default().push(entry);
        }

        // Remove any cookies that expired while the browser was closed.
        jar.purge_expired();
        jar
    }

    /// Replaces all cookies for `zone_id` with the contents of `jar` in a transaction.
    ///
    /// DELETEs the existing rows for the zone and INSERTs the new set.
    /// Best-effort: on database errors the snapshot is skipped and the error is logged.
    fn save_zone(&self, zone_id: ZoneId, jar: &DefaultCookieJar) {
        let Some(mut conn) = self.conn() else {
            return;
        };
        let tx = match conn.transaction() {
            Ok(tx) => tx,
            Err(e) => {
                log::error!("Failed to start cookie transaction for zone {zone_id}: {e}");
                return;
            }
        };

        if let Err(e) = tx.execute("DELETE FROM cookies WHERE zone_id = ?1", [zone_id.to_string()]) {
            log::error!("Failed to delete cookies for zone {zone_id}: {e}");
            return;
        }

        {
            let mut stmt = match tx.prepare(
                "INSERT INTO cookies (zone_id, origin, name, value, path, domain, secure, expires, same_site, http_only, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)"
            ) {
                Ok(stmt) => stmt,
                Err(e) => {
                    log::error!("Failed to prepare cookie INSERT: {e}");
                    return;
                }
            };

            for (origin, cookies) in &jar.entries {
                for cookie in cookies {
                    if let Err(e) = stmt.execute(params![
                        zone_id.to_string(),
                        origin,
                        cookie.name,
                        cookie.value,
                        cookie.path,
                        cookie.domain,
                        cookie.secure as i64,
                        cookie.expires,
                        cookie.same_site,
                        cookie.http_only as i64,
                        cookie.created_at,
                    ]) {
                        log::error!("Failed to insert cookie for zone {zone_id}: {e}");
                        return;
                    }
                }
            }
        }

        if let Err(e) = tx.commit() {
            log::error!("Failed to commit cookie snapshot for zone {zone_id}: {e}");
        }
    }

    /// Deletes all cookies for `zone_id` from the database (best-effort).
    fn remove_zone_from_db(&self, zone_id: ZoneId) {
        let Some(conn) = self.conn() else {
            return;
        };
        if let Err(e) = conn.execute("DELETE FROM cookies WHERE zone_id = ?1", [zone_id.to_string()]) {
            log::error!("Failed to delete cookies for zone {zone_id}: {e}");
        }
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
        let store = match store_ref.as_ref() {
            Some(store) => store.clone(),
            None => {
                log::error!("store_self not initialized; cannot provision cookie jar");
                return None;
            }
        };

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
