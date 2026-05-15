#[cfg(not(target_arch = "wasm32"))]
pub mod blocking;
pub mod response;
