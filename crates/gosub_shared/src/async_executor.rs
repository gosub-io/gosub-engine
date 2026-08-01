//! Cross-platform future spawning and thread-bound aliases.

use std::future::Future;

/// `Send` on native targets; no bound at all on wasm32.
#[cfg(not(target_arch = "wasm32"))]
pub trait WasmNotSend: Send {}

#[cfg(not(target_arch = "wasm32"))]
impl<T: Send> WasmNotSend for T {}

#[cfg(target_arch = "wasm32")]
pub trait WasmNotSend {}

#[cfg(target_arch = "wasm32")]
impl<T> WasmNotSend for T {}

/// `Sync` on native targets; no bound at all on wasm32.
#[cfg(not(target_arch = "wasm32"))]
pub trait WasmNotSync: Sync {}

#[cfg(not(target_arch = "wasm32"))]
impl<T: Sync> WasmNotSync for T {}

#[cfg(target_arch = "wasm32")]
pub trait WasmNotSync {}

#[cfg(target_arch = "wasm32")]
impl<T> WasmNotSync for T {}

/// `Send + Sync` on native targets; no bound at all on wasm32.
pub trait WasmNotSendSync: WasmNotSend + WasmNotSync {}

impl<T: WasmNotSync + WasmNotSend> WasmNotSendSync for T {}

/// Spawn a future and let it run to completion in the background (fire-and-forget).
pub fn spawn<F: Future<Output = ()> + WasmNotSend + 'static>(f: F) {
    #[cfg(target_arch = "wasm32")]
    {
        wasm_bindgen_futures::spawn_local(f);
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        //TODO: this should be done with a thread pool
        std::thread::spawn(|| {
            futures::executor::block_on(f);
        });
    }
}

/// Like [`spawn`], but takes a closure that *creates* the future.
pub fn spawn_from<F: Future<Output = ()> + 'static>(f: impl FnOnce() -> F + 'static + WasmNotSend) {
    #[cfg(target_arch = "wasm32")]
    {
        let fut = f();
        wasm_bindgen_futures::spawn_local(fut);
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        std::thread::spawn(|| {
            let fut = f();
            futures::executor::block_on(fut);
        });
    }
}
