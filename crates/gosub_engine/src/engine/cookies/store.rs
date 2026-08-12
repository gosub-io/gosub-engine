//! Cookie store infrastructure.
//!
//! A cookie store is a persistence backend for per-zone cookie jars
//! ([`JsonCookieStore`], [`SqliteCookieStore`], [`InMemoryCookieStore`]). Zones only
//! hold a [`CookieJarHandle`]; they never hold a store. Providing a `cookie_store` in
//! `ZoneServices` makes the engine wrap the zone's jar (via [`CookieStore::jar_for`])
//! in a [`PersistentCookieJar`](crate::engine::cookies::PersistentCookieJar) that
//! snapshots to the store on every mutation; providing a `cookie_jar` directly gives
//! an ephemeral/private zone with no persistence.
//!
//! Persistent cookies (SQLite):
//! ```rust,no_run
//! use std::sync::Arc;
//! use gosub_engine::GosubEngine;
//! use gosub_engine::zone::{ZoneConfig, ZoneServices};
//! use gosub_engine::storage::{StorageService, InMemoryLocalStore, InMemorySessionStore, PartitionPolicy};
//! use gosub_engine::cookies::SqliteCookieStore;
//! # use tokio::sync::mpsc;
//!
//! # async fn demo(mut engine: GosubEngine) -> anyhow::Result<()> {
//! let services = ZoneServices {
//!     storage: Arc::new(StorageService::new(
//!         Arc::new(InMemoryLocalStore::new()),
//!         Arc::new(InMemorySessionStore::new()),
//!     )),
//!     cookie_store: Some(SqliteCookieStore::new("cookies.db".into())?.into()),
//!     cookie_jar: None, // engine will attach a PersistentCookieJar that snapshots to the store
//!     partition_policy: PartitionPolicy::None,
//!     places: None,
//! };
//! let _zone = engine.create_zone(None, services, None)?;
//! # Ok(()) }
//! ```
//!
//! Ephemeral/private cookies (in-memory jar, no persistence):
//! ```rust,no_run
//! use std::sync::Arc;
//! use gosub_engine::GosubEngine;
//! use gosub_engine::zone::{ZoneConfig, ZoneServices};
//! use gosub_engine::storage::{StorageService, InMemoryLocalStore, InMemorySessionStore, PartitionPolicy};
//! use gosub_engine::cookies::DefaultCookieJar;
//! # use tokio::sync::mpsc;
//!
//! # async fn demo(mut engine: GosubEngine) -> anyhow::Result<()> {
//! let services = ZoneServices {
//!     storage: Arc::new(StorageService::new(
//!         Arc::new(InMemoryLocalStore::new()),
//!         Arc::new(InMemorySessionStore::new()),
//!     )),
//!     cookie_store: None,
//!     cookie_jar: Some(DefaultCookieJar::new().into()),
//!     partition_policy: PartitionPolicy::None,
//!     places: None,
//! };
//! let _zone = engine.create_zone(None, services, None)?;
//! # Ok(()) }
//! ```
//!
//! Per-zone override (e.g. a JSON file for a "private" profile):
//! ```rust,no_run
//! use std::sync::Arc;
//! use gosub_engine::GosubEngine;
//! use gosub_engine::zone::{ZoneConfig, ZoneServices};
//! use gosub_engine::storage::{StorageService, InMemoryLocalStore, InMemorySessionStore, PartitionPolicy};
//! use gosub_engine::cookies::JsonCookieStore;
//! # use tokio::sync::mpsc;
//!
//! # async fn demo(mut engine: GosubEngine) -> anyhow::Result<()> {
//! let services = ZoneServices {
//!     storage: Arc::new(StorageService::new(
//!         Arc::new(InMemoryLocalStore::new()),
//!         Arc::new(InMemorySessionStore::new()),
//!     )),
//!     cookie_store: Some(JsonCookieStore::new("private-cookies.json".into())?.into()),
//!     cookie_jar: None,
//!     partition_policy: PartitionPolicy::None,
//!     places: None,
//! };
//! let _zone = engine.create_zone(None, services, None)?;
//! # Ok(()) }
//! ```
mod in_memory;
mod json;
mod sqlite;

use crate::engine::cookies::cookie_jar::DefaultCookieJar;
use crate::engine::cookies::cookies::{CookieJarHandle, CookieStoreHandle};
use crate::engine::cookies::persistent_cookie_jar::PersistentCookieJar;
use crate::engine::zone::ZoneId;
use parking_lot::RwLock;
use std::collections::HashMap;

/// In-memory cookie store
pub use in_memory::InMemoryCookieStore;
/// File-backed JSON cookie store (one file for all zones).
pub use json::JsonCookieStore;
/// SQLite-backed cookie store (one database for all zones).
pub use sqlite::SqliteCookieStore;

/// A cookie store mints per-zone cookie jars and (optionally) persists them.
///
/// Zones never store a `CookieStore`; they only hold a [`CookieJarHandle`].
/// Implementations must be `Send + Sync` and safe for concurrent use.
pub trait CookieStore: Send + Sync {
    /// Returns (or lazily creates) the cookie jar handle for `zone_id`.
    ///
    /// Must return the same logical jar instance for a given `zone_id` across calls
    /// so all holders observe consistent state; after `remove_zone`/`release_zone` a
    /// subsequent call provisions a fresh jar. Returns `None` only when provisioning
    /// fails irrecoverably.
    fn jar_for(&self, zone_id: ZoneId) -> Option<CookieJarHandle>;

    /// Persists the cookie state for `zone_id` from a snapshot, so the store needs
    /// no reference to the live jar. Best-effort; must not panic.
    fn persist_zone_from_snapshot(&self, zone_id: ZoneId, snapshot: &DefaultCookieJar);

    /// Removes all persisted cookie data for `zone_id`, including any internal cache
    /// for the zone. Idempotent; must not panic.
    fn remove_zone(&self, zone_id: ZoneId);

    /// Releases the in-memory jar for a closed zone: persists a final snapshot and
    /// evicts the cache entry, leaving durable data intact for when the zone reopens.
    /// Contrast with [`CookieStore::remove_zone`], which deletes the persisted data.
    /// Idempotent; must not panic.
    fn release_zone(&self, zone_id: ZoneId);

    /// Persists all known zone jars; called during graceful shutdown or at explicit
    /// flush points. Best-effort; must not panic.
    fn persist_all(&self);
}

/// Shared `jar_for` implementation for persisting stores (JSON, SQLite).
///
/// Returns the cached jar for `zone_id` when present; otherwise calls `load` for the
/// zone's persisted state, wraps it in a [`PersistentCookieJar`] bound to `store_self`
/// (so every mutation writes back to the store), and caches the handle.
pub(crate) fn provision_persistent_jar(
    jars: &RwLock<HashMap<ZoneId, CookieJarHandle>>,
    store_self: &RwLock<Option<CookieStoreHandle>>,
    zone_id: ZoneId,
    load: impl FnOnce() -> DefaultCookieJar,
) -> Option<CookieJarHandle> {
    if let Some(jar) = jars.read().get(&zone_id) {
        return Some(jar.clone());
    }

    let inner: CookieJarHandle = load().into();
    let store = match store_self.read().as_ref() {
        Some(store) => store.clone(),
        None => {
            log::error!("store_self not initialized; cannot provision cookie jar");
            return None;
        }
    };

    let handle = CookieJarHandle::new(PersistentCookieJar::new(zone_id, inner, store));

    // Re-check under the write lock: another thread may have provisioned this zone's jar between
    // our read miss and now. Every `PersistentCookieJar` snapshots the whole jar back to the store,
    // so two live handles for one zone would make concurrent mutations last-write-wins and drop
    // cookies. Return the already-cached handle if present; only the loser's `load()` is wasted.
    let mut guard = jars.write();
    if let Some(existing) = guard.get(&zone_id) {
        return Some(existing.clone());
    }
    guard.insert(zone_id, handle.clone());
    Some(handle)
}

/// Shared `release_zone` cache eviction for persisting stores (JSON, SQLite).
///
/// Removes the zone's jar from the cache and returns a final snapshot to persist,
/// when the cached jar has the [`PersistentCookieJar`]-around-[`DefaultCookieJar`] shape.
pub(crate) fn evict_and_snapshot(
    jars: &RwLock<HashMap<ZoneId, CookieJarHandle>>,
    zone_id: ZoneId,
) -> Option<DefaultCookieJar> {
    let handle = jars.write().remove(&zone_id)?;
    let jar = handle.read();
    let persist = jar.as_any().downcast_ref::<PersistentCookieJar>()?;
    let inner = persist.inner.read();
    inner.as_any().downcast_ref::<DefaultCookieJar>().cloned()
}

/// Shared `persist_all` snapshot loop for persisting stores (JSON, SQLite).
///
/// Calls `save` with a snapshot of every cached jar that is a [`PersistentCookieJar`]
/// wrapping a [`DefaultCookieJar`] - the only shape these stores mint, and the only one
/// with a stable serialization.
pub(crate) fn snapshot_cached_jars(
    jars: &HashMap<ZoneId, CookieJarHandle>,
    mut save: impl FnMut(ZoneId, &DefaultCookieJar),
) {
    for (zone_id, jar_handle) in jars {
        let jar = jar_handle.read();
        let Some(persist) = jar.as_any().downcast_ref::<PersistentCookieJar>() else {
            continue;
        };
        let inner = persist.inner.read();
        let Some(default) = inner.as_any().downcast_ref::<DefaultCookieJar>() else {
            continue;
        };
        save(*zone_id, default);
    }
}
