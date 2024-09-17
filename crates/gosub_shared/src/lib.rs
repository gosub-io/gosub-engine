//! Shared functionality
//!
//! This crate supplies a lot of shared functionality in the gosub engine.
//!

extern crate core;

pub mod byte_stream;
pub mod document;
pub mod errors;
pub mod node;
pub mod timing;
pub mod traits;
pub mod types;
#[cfg(target_arch = "wasm32")]
pub mod worker;
pub mod async_executor;
