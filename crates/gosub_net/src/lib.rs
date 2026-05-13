#![forbid(unsafe_code)]
#![deny(clippy::todo)]
#![deny(clippy::unimplemented)]
#![deny(clippy::dbg_macro)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::panic))]

pub mod http;

#[cfg(not(target_arch = "wasm32"))]
pub use gosub_engine::net;
