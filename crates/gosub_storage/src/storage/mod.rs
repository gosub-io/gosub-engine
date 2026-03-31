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

#[derive(Clone)]
pub struct StorageHandles {
    pub local: Arc<dyn area::StorageArea>,
    pub session: Arc<dyn area::StorageArea>,
}

pub use area::{LocalStore, SessionStore, StorageArea};
pub use event::StorageEvent;
pub use local::in_memory::InMemoryLocalStore;
pub use local::sqlite_store::SqliteLocalStore;
pub use service::{StorageService, Subscription};
pub use session::in_memory::InMemorySessionStore;
pub use types::PartitionKey;
pub use types::PartitionPolicy;
