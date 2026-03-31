//! Engine API surface.
//!
//! Most users should start with [`GosubEngine`].

mod context;
mod engine;
mod errors;

pub mod events;

pub use gosub_storage::cookies;
pub use gosub_storage::storage;
pub mod tab;
pub mod zone;

pub mod config;
mod downloader;
mod policy;
pub mod types;

pub use context::BrowsingContext;
pub use engine::GosubEngine;
pub use errors::EngineError;

pub use policy::UaPolicy;

/// Default capacity for MPSC channels
const DEFAULT_CHANNEL_CAPACITY: usize = 512;

pub mod pipeline;
