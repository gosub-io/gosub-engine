//! Engine API surface.
//!
//! Most users should start with [`GosubEngine`].

mod context;
pub mod damage;
pub(crate) mod edit;
#[allow(clippy::module_inception)]
mod engine;
mod errors;
pub(crate) mod focus;
mod form;
pub(crate) mod input;
pub mod internal_pages;
mod media_source;
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
pub use damage::{Damage, DamageLevel};
pub use engine::EngineContext;
pub use engine::{GosubEngine, ZoneBuilder};
pub use errors::{EngineError, LoadError};
pub use settings_store::default_config as default_settings;

pub use policy::UaPolicy;

/// Default capacity for MPSC channels
const DEFAULT_CHANNEL_CAPACITY: usize = 512;
/// Buffer for the resource stream. Larger than the control bus because it carries one
/// `Progress` per chunk per subresource: a heavy page can produce thousands of these while a
/// shell is busy, and dropping them matters far less than dropping a crash notification.
const RESOURCE_CHANNEL_CAPACITY: usize = 4096;

pub mod resource_pipeline;
