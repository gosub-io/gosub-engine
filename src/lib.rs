extern crate alloc;
extern crate core;

pub mod api;
pub mod bytes;

#[allow(dead_code)]
pub mod byte_stream;

#[allow(dead_code)]
pub mod css3;
#[allow(dead_code)]
pub mod html5;

pub mod render_tree;
pub mod testing;
pub mod types;

#[allow(dead_code)]
mod engine;
#[allow(dead_code)]
mod timing;
pub mod web_executor;
