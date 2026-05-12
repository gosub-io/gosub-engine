//! Local and Session storage systems
//!
//! This module defines the traits, types, and implementations that power
//! HTML5 **LocalStorage** and **SessionStorage** within the engine. It
//! provides both in-memory and persistent backends, a unified service API,
//! and event hooks for reacting to storage changes.
//!
//! # Concepts
//!
//! Gosub separates storage into two main categories:
//!
//! - **Local storage** — Persistent key/value data per `(origin, partition)`,
//!   shared by all tabs in a zone. Backed by a [`LocalStore`].
//! - **Session storage** — Ephemeral key/value data per `(zone, tab, origin, partition)`,
//!   valid for the lifetime of a browsing session or until the tab is closed.
//!   Backed by a [`SessionStore`].
//!
//! All stores implement the [`StorageArea`] trait, which provides the
//! basic API for `get_item`, `set_item`, `remove_item`, and `clear`.
//!
//! A [`StorageService`] wraps one local store and one session store into a
//! single handle that a [`Zone`](crate::zone::Zone) can use to provide both types
//! of storage to its tabs.
//!
//! # Available types
//!
//! - [`PartitionKey`] — Identifies a storage partition
//! - [`StorageArea`] — Trait for any storage backend.
//! - [`LocalStore`], [`SessionStore`] — Type aliases for specific store traits.
//! - [`StorageService`] — High-level handle for a zone's local+session storage.
//! - [`Subscription`] — Used to observe storage change events.
//! - [`StorageEvent`] — Describes a change in storage (key added, removed, etc.).
//! - [`SqliteLocalStore`] — SQLite-backed persistent local storage.
//! - [`InMemorySessionStore`] — In-memory session storage backend.
//!
//! # Choosing a backend
//!
//! - For persistent **LocalStorage**, use [`SqliteLocalStore`].
//! - For ephemeral **SessionStorage**, use [`InMemorySessionStore`].
//! - For testing or incognito modes, you can use in-memory for both.
//!
//! # Example: Attaching storage to a zone
//!
//! ```rust,no_run
//! use std::sync::Arc;
//! use gosub_engine::GosubEngine;
//! use gosub_engine::render::backends::null::NullBackend;
//! use gosub_engine::zone::{ZoneConfig, ZoneServices};
//! use gosub_engine::storage::{StorageService, InMemoryLocalStore, InMemorySessionStore, PartitionPolicy};
//!
//! # async fn demo() -> anyhow::Result<()> {
//! // 1) Build a storage service (persistent local area could be swapped in later)
//! let storage = Arc::new(StorageService::new(
//!     Arc::new(InMemoryLocalStore::new()),
//!     Arc::new(InMemorySessionStore::new()),
//! ));
//!
//! // 2) Engine + backend
//! let backend = NullBackend::new()?;
//! let compositor = gosub_engine::render::DefaultCompositor::default();
//! let mut engine_handle = GosubEngine::new(
//!     None,
//!     Arc::new(backend),
//!     Arc::new(parking_lot::RwLock::new(compositor)),
//! );
//!
//! // 4) Attach storage via ZoneServices and create the zone
//! let services = ZoneServices {
//!     storage: storage.clone(),
//!     cookie_store: None,
//!     cookie_jar: None, // or Some(DefaultCookieJar::new().into()) for ephemeral cookies
//!     partition_policy: PartitionPolicy::None,
//! };
//!
//! let _zone = engine_handle.create_zone(ZoneConfig::default(), services, None)?;
//! # Ok(()) }
//! ```
//!
//! # See also
//!
//! - [`Zone`](crate::zone::Zone) — how storage services are bound to zones.
//! - [`CookieJar`](crate::cookies::CookieJar) — for cookie storage.
//!

use std::sync::Arc;

/// Storage area module, defining the key/value storage interface.
pub mod area;
/// Event module, providing storage change events.
pub mod event;
/// Service module, providing a unified storage service for zones.
pub mod service;
/// Storage types
pub mod types;

/// Local storage module, providing persistent storage areas.
pub mod local {
    /// In-memory local storage implementation.
    pub mod in_memory;
    /// SQLite-backed local storage implementation.
    pub mod sqlite_store;
}

/// Session storage module, providing in-memory session storage.
pub mod session {
    /// In-memory session storage implementation.
    pub mod in_memory;
}

/// Handles to both local and session storage areas.
#[derive(Clone)]
pub struct StorageHandles {
    /// Local storage area, typically persistent and shared across tabs in a zone.
    pub local: Arc<dyn StorageArea>,
    /// Session storage area, typically ephemeral and tied to a specific tab.
    pub session: Arc<dyn StorageArea>,
}

pub use area::{LocalStore, SessionStore, StorageArea};
pub use event::StorageEvent;
pub use local::in_memory::InMemoryLocalStore;
pub use local::sqlite_store::SqliteLocalStore;
pub use service::{StorageService, Subscription};
pub use session::in_memory::InMemorySessionStore;
pub use types::PartitionKey;
pub use types::PartitionPolicy;
