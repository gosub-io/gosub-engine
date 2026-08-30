//! Functionality shared across the gosub engine crates.

extern crate core;

pub mod animation;
pub mod async_executor;
pub mod byte_stream;
pub mod config;
pub mod css_colors;
pub mod errors;
pub mod font;
pub mod geo;
pub mod node;
pub mod tab_id;
pub mod timing;
pub mod types;

pub const ROBOTO_FONT: &[u8] = include_bytes!("../resources/fonts/Roboto-Regular.ttf");
