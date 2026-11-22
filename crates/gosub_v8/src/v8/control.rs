use std::ffi::c_void;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};

use v8::IsolateHandle;

use gosub_shared::types::Result;

use crate::{V8Context, V8Ctx};

/// How V8 is started. V8 is initialized once per process, so the first initialization wins.
#[derive(Clone, Debug, Default)]
pub struct V8Options {
    /// Interpret only. V8 then maps no executable memory.
    pub jitless: bool,
    /// Start no worker threads. Needed where the process may not create threads.
    pub single_threaded: bool,
    /// Further V8 flags, each as it would be given on a command line.
    pub flags: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct V8ContextOptions {
    /// Maximum heap size in bytes. A script that reaches it ends with [`V8Error::OutOfMemory`].
    pub heap_limit: Option<usize>,
    /// Run promise jobs only at [`V8Context::microtask_checkpoint`], not whenever a script returns.
    pub explicit_microtasks: bool,
}

/// Why a script ended without finishing or throwing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum V8Error {
    /// Stopped through a [`V8Terminator`].
    Terminated,
    /// Reached the heap limit of its context.
    OutOfMemory,
}

impl fmt::Display for V8Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Terminated => f.write_str("script execution was terminated"),
            Self::OutOfMemory => f.write_str("script reached the heap limit"),
        }
    }
}

impl std::error::Error for V8Error {}

/// Stops the script a context is running. Can be used from any thread.
///
/// The request stays pending when no script is running, and then ends the next one.
#[derive(Clone)]
pub struct V8Terminator(IsolateHandle);

impl V8Terminator {
    /// Returns false when the context is gone.
    pub fn terminate(&self) -> bool {
        self.0.terminate_execution()
    }
}

pub(crate) struct HeapLimit {
    limit: usize,
    hit: AtomicBool,
    isolate: IsolateHandle,
}

impl HeapLimit {
    pub(crate) fn install(isolate: &mut v8::Isolate, limit: usize) -> Box<Self> {
        let heap = Box::new(Self {
            limit,
            hit: AtomicBool::new(false),
            isolate: isolate.thread_safe_handle(),
        });
        isolate.add_near_heap_limit_callback(near_heap_limit, heap.data());

        heap
    }

    fn data(&self) -> *mut c_void {
        self as *const Self as *mut c_void
    }
}

/// V8 calls this when the heap is about to exceed its limit, and aborts the process if the
/// limit is not raised. So end the script and raise the limit to let it unwind - but to at
/// most twice the configured limit. A script that is still allocating by then takes the
/// process down. Seen with the JIT on, where an optimized allocation loop can run past the
/// termination request; not seen jitless.
unsafe extern "C" fn near_heap_limit(data: *mut c_void, current_limit: usize, _initial_limit: usize) -> usize {
    // SAFETY: `data` is the `HeapLimit` boxed in the `V8Ctx` that owns the isolate.
    let heap = unsafe { &*(data as *const HeapLimit) };
    heap.hit.store(true, Ordering::SeqCst);
    heap.isolate.terminate_execution();

    let ceiling = heap.limit.saturating_mul(2);
    current_limit
        .saturating_add(heap.limit / 2)
        .min(ceiling)
        .max(current_limit)
}

impl V8Ctx {
    /// Puts the heap limit back after `near_heap_limit` raised it.
    fn restore_heap_limit(&mut self) -> bool {
        let Some(heap) = &self.heap else {
            return false;
        };
        if !heap.hit.swap(false, Ordering::SeqCst) {
            return false;
        }

        let (limit, data) = (heap.limit, heap.data());
        // collect what the ended script left behind first, or the old limit is exceeded at once
        self.isolate().low_memory_notification();
        self.isolate().remove_near_heap_limit_callback(near_heap_limit, limit);
        self.isolate().add_near_heap_limit_callback(near_heap_limit, data);

        true
    }
}

impl V8Context {
    pub fn terminator(&self) -> V8Terminator {
        V8Terminator(self.isolate().thread_safe_handle())
    }

    /// Drops a termination request that has not ended a script yet.
    pub fn cancel_termination(&self) {
        self.isolate().cancel_terminate_execution();
    }

    /// Runs the pending promise jobs.
    pub fn microtask_checkpoint(&self) -> Result<()> {
        let terminated = {
            ctx_scope!(scope, self);
            v8::tc_scope!(try_catch, scope);
            try_catch.perform_microtask_checkpoint();

            try_catch.has_terminated()
        };

        match self.take_termination(terminated) {
            Some(e) => Err(e.into()),
            None => Ok(()),
        }
    }

    /// Call when JavaScript returned without a value. `terminated` is the try-catch's
    /// `has_terminated()`: if set, this clears the termination so the context can run again
    /// and says what ended the script.
    pub(crate) fn take_termination(&self, terminated: bool) -> Option<V8Error> {
        if !terminated {
            return None;
        }

        let mut ctx = self.borrow_mut();
        ctx.isolate().cancel_terminate_execution();

        if ctx.restore_heap_limit() {
            Some(V8Error::OutOfMemory)
        } else {
            Some(V8Error::Terminated)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use gosub_webexecutor::js::{WebContext, WebFunction, WebFunctionCallBack, WebObject, WebValue};

    use crate::{V8Context, V8ContextOptions, V8Engine, V8Error, V8Function, V8Object};

    fn context(options: V8ContextOptions) -> V8Context {
        V8Engine::initialize();
        V8Context::with_options(&options).unwrap()
    }

    fn number(ctx: &mut V8Context, code: &str) -> f64 {
        ctx.run(code).unwrap().as_number().unwrap()
    }

    fn error_of<T>(result: gosub_shared::types::Result<T>) -> Option<V8Error> {
        result.err().and_then(|e| e.downcast_ref::<V8Error>().copied())
    }

    /// Stops whatever `ctx` runs next, shortly after it started.
    fn terminate_soon(ctx: &V8Context) -> std::thread::JoinHandle<bool> {
        let terminator = ctx.terminator();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            terminator.terminate()
        })
    }

    const PROMISE: &str = "globalThis.x = 0; Promise.resolve().then(() => { x = 1 }); x";

    #[test]
    fn promise_jobs_wait_for_the_checkpoint() {
        let mut ctx = context(V8ContextOptions {
            explicit_microtasks: true,
            ..Default::default()
        });

        assert_eq!(number(&mut ctx, PROMISE), 0.0);
        assert_eq!(number(&mut ctx, "x"), 0.0);

        ctx.microtask_checkpoint().unwrap();
        assert_eq!(number(&mut ctx, "x"), 1.0);
    }

    #[test]
    fn promise_jobs_run_on_return_by_default() {
        let mut ctx = context(V8ContextOptions::default());

        assert_eq!(number(&mut ctx, PROMISE), 0.0);
        assert_eq!(number(&mut ctx, "x"), 1.0);
    }

    #[test]
    fn a_running_script_is_terminated() {
        let mut ctx = context(V8ContextOptions::default());

        let stopper = terminate_soon(&ctx);
        assert_eq!(error_of(ctx.run("while (true) {}")), Some(V8Error::Terminated));
        assert!(stopper.join().unwrap());

        // the context runs scripts again afterwards
        assert_eq!(number(&mut ctx, "1 + 1"), 2.0);
    }

    #[test]
    fn a_running_promise_job_is_terminated() {
        let mut ctx = context(V8ContextOptions {
            explicit_microtasks: true,
            ..Default::default()
        });
        ctx.run("Promise.resolve().then(() => { while (true) {} })").unwrap();

        let stopper = terminate_soon(&ctx);
        assert_eq!(error_of(ctx.microtask_checkpoint()), Some(V8Error::Terminated));
        assert!(stopper.join().unwrap());

        assert_eq!(number(&mut ctx, "1 + 1"), 2.0);
    }

    #[test]
    fn a_called_function_is_terminated() {
        let mut ctx = context(V8ContextOptions::default());
        ctx.run("globalThis.o = { spin() { while (true) {} } }").unwrap();
        let object = ctx.run("o").unwrap().as_object().unwrap();

        let stopper = terminate_soon(&ctx);
        assert_eq!(error_of(object.call_method("spin", &[])), Some(V8Error::Terminated));
        assert!(stopper.join().unwrap());

        // a native function that calls back into a spinning script
        let inner = ctx.clone();
        let mut function = V8Function::new(ctx.clone(), move |cb| {
            if let Err(e) = inner.clone().run("while (true) {}") {
                cb.error(e);
            }
        })
        .unwrap();

        let stopper = terminate_soon(&ctx);
        assert!(function.call(&[]).is_err());
        assert!(stopper.join().unwrap());

        assert_eq!(number(&mut ctx, "1 + 1"), 2.0);
        drop(V8Object::new(ctx).unwrap());
    }

    // tests/init_options.rs has the jitless run, with a loop the JIT does not stop in time
    const HOARD: &str = "(function () { const a = []; for (;;) a.push({ x: 1 }); })()";

    #[test]
    fn the_heap_limit_ends_the_script() {
        let mut ctx = context(V8ContextOptions {
            heap_limit: Some(32 * 1024 * 1024),
            ..Default::default()
        });

        assert_eq!(error_of(ctx.run(HOARD)), Some(V8Error::OutOfMemory));
        assert_eq!(number(&mut ctx, "1 + 1"), 2.0);

        // the limit is back in place, not left where the first script pushed it
        assert_eq!(error_of(ctx.run(HOARD)), Some(V8Error::OutOfMemory));
        assert_eq!(error_of(ctx.run(HOARD)), Some(V8Error::OutOfMemory));
        assert_eq!(number(&mut ctx, "1 + 1"), 2.0);
    }

    #[test]
    fn a_context_without_options_has_wasm() {
        // the counterpart of tests/init_options.rs, where jitless takes it away
        let mut ctx = context(V8ContextOptions::default());

        assert_eq!(ctx.run("typeof WebAssembly").unwrap().as_string().unwrap(), "object");
    }
}
