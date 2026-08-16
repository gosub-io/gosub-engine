//! HTML5 LocalStorage and SessionStorage: in-memory and persistent backends, a
//! unified service API, and event hooks for reacting to storage changes.
//!
//! Local storage ([`LocalStore`], e.g. [`SqliteLocalStore`]) is persistent key/value
//! data per `(origin, partition)`, shared by all tabs in a zone. Session storage
//! ([`SessionStore`], e.g. [`InMemorySessionStore`]) is ephemeral data per
//! `(zone, tab, origin, partition)`, dropped when the tab closes. All areas
//! implement [`StorageArea`]; a [`StorageService`] bundles one local and one
//! session store for a [`Zone`](crate::zone::Zone) to hand to its tabs.
//!
//! # Example: Attaching storage to a zone
//!
//! ```rust,no_run
//! use std::sync::Arc;
//! use gosub_engine::GosubEngine;
//! use gosub_render_pipeline::render::backends::null::NullBackend;
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
//! let backend = NullBackend::new();
//! let compositor = gosub_render_pipeline::render::DefaultCompositor::default();
//! let mut engine_handle: GosubEngine = GosubEngine::new(
//!     None,
//!     Arc::new(backend),
//!     Arc::new(compositor),
//! );
//!
//! // 3) Attach storage via ZoneServices and create the zone
//! let services = ZoneServices {
//!     storage: storage.clone(),
//!     cookie_store: None,
//!     cookie_jar: None, // or Some(DefaultCookieJar::new().into()) for ephemeral cookies
//!     partition_policy: PartitionPolicy::None,
//! };
//!
//! let _zone = engine_handle.create_zone(None, services, None)?;
//! # Ok(()) }
//! ```

use std::sync::Arc;

pub mod area;
pub mod event;
pub mod service;
pub mod types;

pub mod local {
    pub mod in_memory;
    pub mod sqlite_store;
}

pub mod session {
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
