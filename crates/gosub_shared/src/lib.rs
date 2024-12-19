//! Shared functionality
//!
//! This crate supplies a lot of shared functionality in the gosub engine.
//!

extern crate core;

pub mod async_executor;
pub mod byte_stream;
pub mod document;
pub mod errors;
pub mod font;
pub mod node;
pub mod render_backend;
pub mod timing;
pub mod traits;
pub mod types;

pub const ROBOTO_FONT: &[u8] = include_bytes!("../resources/fonts/Roboto-Regular.ttf");
