//! Engine API surface.
//!
//! Most users should start with [`GosubEngine`].

mod context;
#[allow(clippy::module_inception)]
mod engine;
mod errors;

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

pub mod resource_pipeline;
