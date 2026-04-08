use tokio::task::JoinHandle;

/// Spawns a name task on tokio runtime with the given name and future. Will return the join
/// handle.
///
/// # Examples
/// ```
/// let handle = spawn_named("my_task", async {
///   // task code here
/// });
/// ```
///
/// # Panics
/// Panics if the task fails to spawn.
///
/// # Arguments
/// * `name` - The name of the task.
/// * `fut` - The future to run in the task.
///
/// # Type Parameters
/// * `F` - The type of the future.
/// * `T` - The output type of the future.
///
/// # Constraints
/// * `F` must be a future that is Send and 'static.
/// * `T` must be Send and 'static.
///
/// # Returns
/// A JoinHandle to the spawned task.
///
pub fn spawn_named<F, T>(_name: &str, fut: F) -> JoinHandle<T>
where
    F: std::future::Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn(fut)
}
