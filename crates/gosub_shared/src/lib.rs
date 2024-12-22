//! Shared functionality
//!
//! This crate supplies a lot of shared functionality in the gosub engine.
//!

extern crate core;

pub mod async_executor;
pub mod byte_stream;
pub mod config;
pub mod errors;
pub mod font;
pub mod geo;
pub mod node;
pub mod timing;
pub mod types;

pub const ROBOTO_FONT: &[u8] = include_bytes!("../resources/fonts/Roboto-Regular.ttf");
