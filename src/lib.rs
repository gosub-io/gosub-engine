extern crate alloc;
extern crate core;

// Generic engine modules
#[allow(dead_code)]
mod engine;
pub mod bytes;
pub mod byte_stream;
#[allow(dead_code)]
mod timing;

// Html/CSS
#[allow(dead_code)]
pub mod css3;
#[allow(dead_code)]
pub mod html5;
pub mod styles;

// Rendering
pub mod render_tree;

// Misc
pub mod testing;
pub mod types;
#[allow(dead_code)]
pub mod config;

// Network
#[allow(dead_code)]
mod dns;
#[allow(dead_code)]
mod net;

// Javascript and js API
pub mod api;
#[allow(dead_code, unused_imports)]
pub mod js;

