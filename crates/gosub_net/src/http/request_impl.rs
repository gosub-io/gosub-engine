#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod reqwest_impl;

#[cfg(target_arch = "wasm32")]
mod wasm_impl;

#[cfg(not(target_arch = "wasm32"))]
pub type RequestImpl = reqwest_impl::ReqwestAgent;

#[cfg(target_arch = "wasm32")]
pub type RequestImpl = wasm_impl::WasmAgent;
