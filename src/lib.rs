// Not compiled for wasm: it depends on the blocking sync_fetch path.
#[cfg(not(target_arch = "wasm32"))]
pub mod direct_loader;
pub mod prelude;

#[cfg(target_arch = "wasm32")]
mod wasm;
