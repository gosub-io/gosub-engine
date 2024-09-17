#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod ureq_impl;

#[cfg(target_arch = "wasm32")]
mod wasm_impl;

#[cfg(not(target_arch = "wasm32"))]
pub type RequestImpl = ureq_impl::UreqAgent;

#[cfg(target_arch = "wasm32")]
pub type RequestImpl = wasm_impl::WasmAgent;
