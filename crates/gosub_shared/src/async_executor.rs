use std::future::Future;
use std::thread;

pub fn spawn<F: Future + Send + 'static>(f: F) {
    #[cfg(target_arch = "wasm32")]
    {
        wasm_bindgen_futures::spawn_local(f);
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        //TODO: this should be done with a thread pool
        thread::spawn(|| {
            futures::executor::block_on(f);
        });
    }
}