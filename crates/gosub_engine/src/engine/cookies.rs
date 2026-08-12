//! Cookie storage and persistence (RFC 6265). Zones use a [`CookieJar`] (via a cheap,
//! cloneable [`CookieJarHandle`]) and can optionally persist jar mutations to a backend
//! [`CookieStore`] ([`InMemoryCookieStore`], [`JsonCookieStore`], [`SqliteCookieStore`]).
//!
//! `CookieJarHandle` (and `CookieStoreHandle`) are cloneable `Send + Sync` handles;
//! reads are concurrent, mutations are synchronized inside the jar implementation.
//!
//! Each [`Zone`](crate::zone::Zone) uses exactly one [`CookieJar`], wired up through
//! `ZoneServices` when creating the zone. For ephemeral (private) cookies, supply a
//! `cookie_jar` directly:
//!
//! ```no_run
//! # use std::sync::Arc;
//! # use tokio::sync::mpsc;
//! use gosub_render_pipeline::render::Viewport;
//! use gosub_engine::zone::{ZoneConfig, ZoneServices};
//! use gosub_engine::storage::{StorageService, InMemoryLocalStore, InMemorySessionStore, PartitionPolicy};
//! use gosub_engine::cookies::{DefaultCookieJar};
//!
//! # async fn demo(mut engine: gosub_engine::GosubEngine) -> anyhow::Result<()> {
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
//!
//! let zone_cfg = ZoneConfig::default();
//! let _zone = engine.create_zone(Some(zone_cfg), services, None)?;
//! # Ok(()) }
//! ```
//!
//! For persistent cookies, supply a `cookie_store` and omit `cookie_jar`; the engine
//! attaches a `PersistentCookieJar` for the zone that snapshots on every mutation.
//! A single store backend (e.g. one SQLite DB) can serve all zones; each zone still
//! operates on its own jar.
//!
//! ```no_run
//! # use std::sync::Arc;
//! # use tokio::sync::mpsc;
//! use gosub_engine::zone::{ZoneConfig, ZoneServices};
//! use gosub_engine::storage::{StorageService, InMemoryLocalStore, InMemorySessionStore, PartitionPolicy};
//! use gosub_engine::cookies::{SqliteCookieStore};
//!
//! # async fn demo(mut engine: gosub_engine::GosubEngine) -> anyhow::Result<()> {
//! let store = SqliteCookieStore::new("cookies.db".into())?;
//!
//! let services = ZoneServices {
//!     storage: Arc::new(StorageService::new(
//!         Arc::new(InMemoryLocalStore::new()),
//!         Arc::new(InMemorySessionStore::new()),
//!     )),
//!     cookie_store: Some(store.into()),
//!     cookie_jar: None, // engine will wrap with PersistentCookieJar per zone
//!     partition_policy: PartitionPolicy::None,
//!     places: None,
//! };
//!
//! let zone_cfg = ZoneConfig::default();
//! let _zone = engine.create_zone(Some(zone_cfg), services, None)?;
//! # Ok(()) }
//! ```
mod cookie_jar;
#[allow(clippy::module_inception)]
mod cookies;
mod persistent_cookie_jar;
mod store;
#[cfg(test)]
mod tests;

pub use cookies::Cookie;
pub use cookies::CookieJarHandle;
pub use cookies::CookieStoreHandle;

pub use cookie_jar::CookieJar;
pub use cookie_jar::DefaultCookieJar;
pub use cookie_jar::SameSiteContext;
pub use cookie_jar::ThirdPartyCookiePolicy;
pub use persistent_cookie_jar::PersistentCookieJar;

pub use store::CookieStore;
pub use store::InMemoryCookieStore;
pub use store::JsonCookieStore;
pub use store::SqliteCookieStore;
