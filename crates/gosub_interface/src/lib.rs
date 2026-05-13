#![forbid(unsafe_code)]
#![deny(clippy::todo)]
#![deny(clippy::unimplemented)]
#![deny(clippy::dbg_macro)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod chrome;
pub mod config;
pub mod css3;
pub mod document;
pub mod draw;
pub mod eventloop;
pub mod font;
pub mod html5;
pub mod input;
pub mod instance;
pub mod layout;
pub mod node;
pub mod render_backend;
pub mod render_tree;
pub mod request;
pub mod svg;
