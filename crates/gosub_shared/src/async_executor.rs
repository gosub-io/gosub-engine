//! Cross-platform future spawning and thread-bound aliases.
//!
//! The engine runs on native targets (threads available) and on wasm32 (single-threaded,
//! futures driven by the JS event loop). That difference leaks into trait bounds: code that
//! is generic over async work needs `Send`/`Sync` on native, but requiring them on wasm would
//! reject perfectly fine single-threaded futures. The `WasmNot*` traits paper over this —
//! they alias `Send`/`Sync` on native and are unconditional (bound-free) on wasm, so a single
//! `T: WasmNotSend` bound means "Send where threads exist".
//!
//! [`spawn`] and [`spawn_from`] are fire-and-forget executors on top of the same split:
//! `wasm_bindgen_futures::spawn_local` on wasm, a dedicated thread running `block_on` on
//! native.

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
///
/// On wasm32 the future is queued on the JS event loop; on native it gets a dedicated
/// thread that blocks on it.
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
///
/// Only the closure has to be `Send` (on native): it is moved to the spawned thread and the
/// future is constructed there, so the future itself may be `!Send`. Use this for async work
/// built from thread-bound (`Rc`, `RefCell`, …) state.
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
