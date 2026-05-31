use tokio::task::JoinHandle;

/// Spawns a named async task. On native targets uses `tokio::spawn` (multi-thread safe).
/// On WASM uses `tokio::task::spawn_local` (single-thread, no `Send` required).
#[cfg(not(target_arch = "wasm32"))]
pub fn spawn_named<F, T>(_name: &str, fut: F) -> JoinHandle<T>
where
    F: std::future::Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    tokio::spawn(fut)
}

#[cfg(target_arch = "wasm32")]
pub fn spawn_named<F, T>(_name: &str, fut: F) -> JoinHandle<T>
where
    F: std::future::Future<Output = T> + 'static,
    T: 'static,
{
    tokio::task::spawn_local(fut)
}
