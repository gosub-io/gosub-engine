//! Cookie management system for the Gosub engine.
//!
//! This module defines the core types for storing, retrieving, and persisting HTTP
//! cookies. Zones use a [`CookieJar`] (via a cheap, cloneable [`CookieJarHandle`])
//! and can optionally persist jar mutations to a backend [`CookieStore`].
//!
//! ## Overview
//!
//! - [`Cookie`] — A single HTTP cookie (name, value, domain, path, expiry, SameSite, etc.).
//! - [`CookieJar`] — Thread-safe, in-memory cookie jar implementing RFC 6265 semantics.
//! - [`DefaultCookieJar`] — The engine’s default in-memory [`CookieJar`] (ephemeral).
//! - [`PersistentCookieJar`] — A [`CookieJar`] wrapper that snapshots changes to a
//!   persistent [`CookieStore`] backend.
//! - [`CookieStore`] — Trait for durable storage backends.
//!   - [`InMemoryCookieStore`] — Non-persistent store (useful for tests).
//!   - [`JsonCookieStore`] — Human-readable JSON files (handy for debugging).
//!   - [`SqliteCookieStore`] — SQLite-backed store (scales to many cookies).
//!
//! Internally, `CookieJarHandle` (and `CookieStoreHandle`) are cloneable, thread-safe
//! handles you can pass between engine tasks/zones. The jar itself is safe to read
//! concurrently; mutations are synchronized inside the jar implementation.
//!
//! ## How zones get a cookie jar
//!
//! Each [`Zone`](crate::zone::Zone) uses **exactly one** [`CookieJar`]. You wire this up
//! through `ZoneServices` when creating the zone:
//!
//! 1. **Ephemeral (private) cookies** — supply a direct `cookie_jar`:
//!
//! ```no_run
//! # use std::sync::Arc;
//! # use tokio::sync::mpsc;
//! use gosub_engine_api::render::Viewport;
//! use gosub_engine_api::zone::{ZoneConfig, ZoneServices};
//! use gosub_engine_api::storage::{StorageService, InMemoryLocalStore, InMemorySessionStore, PartitionPolicy};
//! use gosub_engine_api::cookies::{DefaultCookieJar};
//!
//! # async fn demo(mut engine: gosub_engine_api::GosubEngine) -> anyhow::Result<()> {
//! let services = ZoneServices {
//!     storage: Arc::new(StorageService::new(
//!         Arc::new(InMemoryLocalStore::new()),
//!         Arc::new(InMemorySessionStore::new()),
//!     )),
//!     cookie_store: None,
//!     cookie_jar: Some(DefaultCookieJar::new().into()),
//!     partition_policy: PartitionPolicy::None,
//! };
//!
//! let zone_cfg = ZoneConfig::default();
//! let _zone = engine.create_zone(zone_cfg, services, None)?;
//! # Ok(()) }
//! ```
//!
//! 2. **Persistent cookies** — supply a `cookie_store` and omit `cookie_jar`. The engine
//!    will attach a `PersistentCookieJar` for this zone that snapshots on every mutation:
//!
//! ```no_run
//! # use std::sync::Arc;
//! # use tokio::sync::mpsc;
//! use gosub_engine_api::zone::{ZoneConfig, ZoneServices};
//! use gosub_engine_api::storage::{StorageService, InMemoryLocalStore, InMemorySessionStore, PartitionPolicy};
//! use gosub_engine_api::cookies::{SqliteCookieStore};
//!
//! # async fn demo(mut engine: gosub_engine_api::GosubEngine) -> anyhow::Result<()> {
//! let store = SqliteCookieStore::new("cookies.db".into());
//!
//! let services = ZoneServices {
//!     storage: Arc::new(StorageService::new(
//!         Arc::new(InMemoryLocalStore::new()),
//!         Arc::new(InMemorySessionStore::new()),
//!     )),
//!     cookie_store: Some(store.into()),
//!     cookie_jar: None, // engine will wrap with PersistentCookieJar per zone
//!     partition_policy: PartitionPolicy::None,
//! };
//!
//! let zone_cfg = ZoneConfig::default();
//! let _zone = engine.create_zone(zone_cfg, services, None)?;
//! # Ok(()) }
//! ```
//!
//! > **Note:** A persistent store can serve **all zones** from a single backend
//! > (e.g., one SQLite DB). Each zone still operates on its own jar; the store
//! > takes care of mapping a zone’s jar to durable rows/records behind the scenes.
//!
//! ## Choosing a backend
//!
//! - [`InMemoryCookieStore`] — Zero I/O, great for tests and benches.
//! - [`JsonCookieStore`] — Easy to inspect; slower for large volumes.
//! - [`SqliteCookieStore`] — Durable and efficient for long-lived profiles.
//!
//! ## Concurrency & safety
//!
//! - `CookieJarHandle` is cloneable (`Send + Sync`). Reads are concurrent; writes are
//!   serialized internally. If using a persistent jar, each mutation triggers a snapshot
//!   to the store so state stays consistent across restarts.
//!
//! ## See also
//!
//! - [`Zone`](crate::zone::Zone) — where jars are attached/used.
//! - [`CookieStore`] — implement this trait to add your own persistence backend.
//! - RFC 6265 — for cookie parsing/matching semantics (domain/path, expiration,
//!   HttpOnly, Secure, SameSite, etc.).
mod cookie_jar;
#[allow(clippy::module_inception)]
mod cookies;
mod persistent_cookie_jar;
mod store;

pub use cookies::Cookie;
pub use cookies::CookieJarHandle;
pub use cookies::CookieStoreHandle;

pub use cookie_jar::CookieJar;
pub use cookie_jar::DefaultCookieJar;
pub use persistent_cookie_jar::PersistentCookieJar;

pub use store::CookieStore;
pub use store::InMemoryCookieStore;
pub use store::JsonCookieStore;
pub use store::SqliteCookieStore;
