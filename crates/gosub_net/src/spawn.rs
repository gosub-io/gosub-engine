use tokio::task::JoinHandle;

/// Spawns a named task on the tokio runtime.
pub fn spawn_named<F, T>(_name: &str, fut: F) -> JoinHandle<T>
where
    F: std::future::Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn(fut)
}
