//! Cookie store infrastructure.
//!
//! A **cookie store** is a persistence backend for per-zone cookie jars. Zones
//! themselves only hold a [`CookieJarHandle`]; they never hold a store.
//!
//! When you create a zone, you can:
//! - Provide a **`cookie_store`** in `ZoneServices`. The engine will obtain (or
//!   initialize) the zone’s jar via [`CookieStore::jar_for`] and wrap it in a
//!   [`PersistentCookieJar`](crate::engine::cookies::PersistentCookieJar) so that
//!   **every mutation** snapshots to the store.
//! - Or provide a **`cookie_jar`** directly (e.g., [`DefaultCookieJar`]) for
//!   ephemeral/private zones with no persistence.
//!
//! ## Typical usage
//!
//! **Persistent cookies (SQLite):**
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
//!     cookie_store: Some(SqliteCookieStore::new("cookies.db".into()).into()),
//!     cookie_jar: None, // engine will attach a PersistentCookieJar that snapshots to the store
//!     partition_policy: PartitionPolicy::None,
//! };
//! let _zone = engine.create_zone(ZoneConfig::default(), services, None)?;
//! # Ok(()) }
//! ```
//!
//! **Ephemeral/private cookies (in-memory jar, no persistence):**
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
//! };
//! let _zone = engine.create_zone(ZoneConfig::default(), services, None)?;
//! # Ok(()) }
//! ```
//!
//! **Per-zone override (e.g., JSON file for a “private” profile):**
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
//!     cookie_store: Some(JsonCookieStore::new("private-cookies.json".into()).into()),
//!     cookie_jar: None,
//!     partition_policy: PartitionPolicy::None,
//! };
//! let _zone = engine.create_zone(ZoneConfig::default(), services, None)?;
//! # Ok(()) }
//! ```
//!
//! ## Design notes
//! - Stores are **backend components** (JSON/SQLite/in-memory). Zones only see a jar handle.
//! - Implementations must be `Send + Sync` and safe for concurrent use.
//! - [`CookieStore::jar_for`] should return the **same logical jar** for a given
//!   `ZoneId` across calls, so all holders observe consistent state.
//! - With a [`PersistentCookieJar`], the engine will call
//!   [`CookieStore::persist_zone_from_snapshot`] after each mutation to keep durable
//!   state in sync.
//!
//! ## Provided backends
//! - [`JsonCookieStore`]: file-backed JSON (easy to inspect/debug).
//! - [`SqliteCookieStore`]: SQLite (scales to many cookies, concurrent friendly).
//! - [`InMemoryCookieStore`]: non-persistent (tests, disposable profiles).
mod in_memory;
mod json;
mod sqlite;

use crate::engine::cookies::cookie_jar::DefaultCookieJar;
use crate::engine::cookies::cookies::CookieJarHandle;
use crate::engine::zone::ZoneId;

/// In-memory cookie store
pub use in_memory::InMemoryCookieStore;
/// File-backed JSON cookie store (one file for all zones).
pub use json::JsonCookieStore;
/// SQLite-backed cookie store (one database for all zones).
pub use sqlite::SqliteCookieStore;

/// A cookie **store** mints per-zone cookie **jars** and (optionally) persists them.
///
/// Zones never store a `CookieStore`; they only hold a [`CookieJarHandle`].
/// The store exists to:
/// 1) provide the jar for a given [`ZoneId`], and
/// 2) write/read cookie state to/from durable storage.
///
/// Implementations must be `Send + Sync` and safe for concurrent use.
pub trait CookieStore: Send + Sync {
    /// Returns (or creates and returns) the cookie jar handle for `zone_id`.
    ///
    /// ### Expectations
    /// - Should return the *same logical jar instance* for a given `zone_id`
    ///   across calls, so all holders observe consistent state.
    /// - May create the jar lazily on first request.
    /// - Return `None` if the store no longer manages this zone (e.g., after removal)
    ///   or if provisioning fails irrecoverably.
    fn jar_for(&self, zone_id: ZoneId) -> Option<CookieJarHandle>;

    /// Persists the cookie state for `zone_id` from a provided snapshot.
    ///
    /// This allows the engine to push the current in-memory state (captured in
    /// a [`DefaultCookieJar`] snapshot) into the store without requiring the store
    /// to hold a direct reference to the live jar.
    ///
    /// Implementations may choose to:
    /// - Replace the stored state, or
    /// - Merge it (e.g., last-write-wins), depending on policy.
    ///
    /// This should be **best-effort** and must not panic.
    fn persist_zone_from_snapshot(&self, zone_id: ZoneId, snapshot: &DefaultCookieJar);

    /// Removes all persisted cookie data for `zone_id` from the store.
    ///
    /// Implementations should also drop any internal cache for this zone so that
    /// subsequent calls to [`CookieStore::jar_for`] can recreate a fresh, empty jar (or return `None`).
    ///
    /// This operation should be **idempotent** and must not panic.
    fn remove_zone(&self, zone_id: ZoneId);

    /// Persists all known zone jars to durable storage.
    ///
    /// Called during graceful shutdown or at explicit flush points. Implementations
    /// should make a **best-effort** to write all dirty state and avoid panicking.
    fn persist_all(&self);
}
