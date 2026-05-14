pub mod http;

#[cfg(not(target_arch = "wasm32"))]
pub use gosub_engine::net;
