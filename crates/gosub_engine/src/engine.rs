//! Engine API surface.
//!
//! Most users should start with [`GosubEngine`].

mod context;
#[allow(clippy::module_inception)]
mod engine;
mod errors;
pub mod internal_pages;
pub mod places;

pub mod events;

pub mod cookies;
pub mod storage;
pub mod tab;
pub mod zone;

pub mod config;
mod policy;
pub mod settings_store;
pub mod types;

pub use context::BrowsingContext;
pub use engine::EngineContext;
pub use engine::GosubEngine;
pub use errors::EngineError;
pub use settings_store::default_config as default_settings;

pub use policy::UaPolicy;

/// Default capacity for MPSC channels
const DEFAULT_CHANNEL_CAPACITY: usize = 512;
/// Buffer for the resource stream. Larger than the control bus because it carries one
/// `Progress` per chunk per subresource: a heavy page can produce thousands of these while a
/// shell is busy, and dropping them matters far less than dropping a crash notification.
const RESOURCE_CHANNEL_CAPACITY: usize = 4096;

pub mod resource_pipeline;
