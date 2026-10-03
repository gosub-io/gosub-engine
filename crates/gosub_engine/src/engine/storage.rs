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
//! // 1) Build a storage service. Pick the local store now (a `FileLocalStore` to persist):
//! //    it is fixed once the first area is handed out.
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
//!     places: None,
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
    pub mod file_store;
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

#[cfg(all(feature = "process-isolation", target_os = "linux"))]
pub use crate::storage_service::client::ServiceLocalStore;
pub use area::{LocalStore, SessionStore, StorageArea};
pub use event::StorageEvent;
pub use local::file_store;
pub use local::file_store::FileLocalStore;
pub use local::in_memory::InMemoryLocalStore;
pub use local::sqlite_store::SqliteLocalStore;
pub use service::{StorageService, Subscription};
pub use session::in_memory::InMemorySessionStore;
pub use types::PartitionKey;
pub use types::PartitionPolicy;

/// Create `dir` (and its parents) for this user alone: `0700`, so another
/// local user on a traversable profile directory cannot read what pages
/// stored. An existing directory keeps its mode (the embedder's choice).
pub fn private_dir(dir: &std::path::Path) -> std::io::Result<()> {
    if dir.is_dir() {
        return Ok(());
    }
    if let Some(parent) = dir.parent() {
        if !parent.as_os_str().is_empty() {
            private_dir(parent)?;
        }
    }
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(0o700);
    }
    match builder.create(dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(e),
    }
}
