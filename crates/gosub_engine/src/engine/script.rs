//! Running scripts for a tab's page.
//!
//! A page's [`ScriptHost`] lives on a thread of its own. A script engine ties its global
//! scope to the thread that made it, and a script runs for as long as it likes, which a tab
//! worker cannot afford to wait for. The worker hands over source text and gets the result
//! back as data; a script that runs past its time limit is terminated from here.

use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use gosub_interface::script::{ScriptError, ScriptErrorKind, ScriptHost, ScriptOrigin, ScriptTerminator, ScriptValue};
use parking_lot::Mutex;
use tokio::sync::oneshot;

#[cfg(feature = "js")]
mod v8;

/// How often a script that is over its time limit is told to stop, until it has.
const TERMINATE_RETRY: Duration = Duration::from_millis(50);

type ScriptResult = Result<ScriptValue, ScriptError>;
type HostFactory = Box<dyn FnOnce() -> Result<Box<dyn ScriptHost>, ScriptError> + Send>;

/// Limits for the host of one page.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ScriptLimits {
    /// Heap size in bytes at which a script is ended.
    #[cfg_attr(not(feature = "js"), allow(dead_code))] // read by the host only
    pub heap_bytes: usize,
}

/// The engine's script host, or why there is none.
#[cfg(feature = "js")]
pub(crate) fn page_host(limits: ScriptLimits) -> Result<Box<dyn ScriptHost>, ScriptError> {
    Ok(Box::new(v8::V8ScriptHost::new(limits)?))
}

#[cfg(not(feature = "js"))]
pub(crate) fn page_host(_limits: ScriptLimits) -> Result<Box<dyn ScriptHost>, ScriptError> {
    Err(unavailable("this build has no JavaScript engine (cargo feature `js`)"))
}

pub(crate) fn unavailable(why: &str) -> ScriptError {
    ScriptError::new(ScriptErrorKind::Unavailable, why)
}

struct Job {
    id: u64,
    source: String,
    origin: ScriptOrigin,
    reply: oneshot::Sender<ScriptResult>,
}

struct Shared {
    /// The job the thread is running, 0 for none. A script is only terminated with this
    /// held and naming it, so that a request never lands on the script after it.
    running: Mutex<u64>,
    /// Set once the host exists.
    terminator: std::sync::OnceLock<ScriptTerminator>,
}

/// The script thread of one page. Dropping it ends the thread once the script it is running
/// returns.
pub(crate) struct ScriptThread {
    jobs: mpsc::Sender<Job>,
    shared: Arc<Shared>,
    next_id: std::sync::atomic::AtomicU64,
}

impl ScriptThread {
    /// `make` runs on the new thread. If it fails, every script fails with its error.
    pub(crate) fn spawn(make: HostFactory) -> std::io::Result<Self> {
        let (jobs, queue) = mpsc::channel::<Job>();
        let shared = Arc::new(Shared {
            running: Mutex::new(0),
            terminator: std::sync::OnceLock::new(),
        });

        let thread_shared = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("gosub-script".to_owned())
            .spawn(move || run_jobs(make, &queue, &thread_shared))?;

        Ok(Self {
            jobs,
            shared,
            next_id: std::sync::atomic::AtomicU64::new(1),
        })
    }

    /// Runs `source`, then the promise jobs it queued. A script still running after `limit`
    /// is terminated.
    pub(crate) async fn execute(&self, source: String, origin: ScriptOrigin, limit: Duration) -> ScriptResult {
        let id = self.next_id.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let (reply, mut result) = oneshot::channel();
        let job = Job {
            id,
            source,
            origin,
            reply,
        };
        if self.jobs.send(job).is_err() {
            return Err(unavailable("the script thread is gone"));
        }

        let mut wait = limit;
        loop {
            match tokio::time::timeout(wait, &mut result).await {
                Ok(Ok(result)) => return result,
                Ok(Err(_)) => return Err(unavailable("the script thread is gone")),
                // Asked again until the script has ended: a request can arrive just before
                // the script starts, where the host drops it.
                Err(_) => {
                    self.terminate(id);
                    wait = TERMINATE_RETRY;
                }
            }
        }
    }

    fn terminate(&self, id: u64) {
        let running = self.shared.running.lock();
        if *running == id {
            if let Some(terminator) = self.shared.terminator.get() {
                terminator.terminate();
            }
        }
    }
}

fn run_jobs(make: HostFactory, queue: &mpsc::Receiver<Job>, shared: &Shared) {
    let mut host = make();
    if let Ok(host) = &host {
        let _ = shared.terminator.set(host.terminator());
    }

    for job in queue {
        *shared.running.lock() = job.id;
        let result = match &mut host {
            Ok(host) => run(host.as_mut(), &job),
            Err(e) => Err(e.clone()),
        };
        *shared.running.lock() = 0;

        let _ = job.reply.send(result);
    }
}

fn run(host: &mut dyn ScriptHost, job: &Job) -> ScriptResult {
    let value = host.execute(&job.source, &job.origin)?;
    host.microtask_checkpoint()?;

    Ok(value)
}

/// `ScriptValue` as the JSON an embedder gets. JSON has no undefined and no NaN or
/// infinity: those are null.
pub(crate) fn to_json(value: ScriptValue) -> serde_json::Value {
    use serde_json::Value;

    match value {
        ScriptValue::Undefined | ScriptValue::Null => Value::Null,
        ScriptValue::Bool(b) => Value::Bool(b),
        ScriptValue::Number(n) => serde_json::Number::from_f64(n).map_or(Value::Null, Value::Number),
        ScriptValue::String(s) => Value::String(s),
        ScriptValue::Json(text) => serde_json::from_str(&text).unwrap_or(Value::String(text)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    /// Spins until terminated, for sources that say so; else answers with the source length.
    struct StubHost {
        stop: Arc<AtomicBool>,
        checkpoints: Arc<AtomicUsize>,
    }

    impl ScriptHost for StubHost {
        fn execute(&mut self, source: &str, _origin: &ScriptOrigin) -> ScriptResult {
            // like the real host: a request from before this script is not for it
            self.stop.store(false, Ordering::SeqCst);
            if source == "spin" {
                while !self.stop.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(1));
                }
                return Err(ScriptError::new(ScriptErrorKind::Terminated, "terminated"));
            }
            Ok(ScriptValue::Number(source.len() as f64))
        }

        fn microtask_checkpoint(&mut self) -> Result<(), ScriptError> {
            self.checkpoints.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn terminator(&self) -> ScriptTerminator {
            let stop = Arc::clone(&self.stop);
            ScriptTerminator::new(move || stop.store(true, Ordering::SeqCst))
        }
    }

    fn stub_thread() -> (ScriptThread, Arc<AtomicUsize>) {
        let checkpoints = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&checkpoints);
        let thread = ScriptThread::spawn(Box::new(move || {
            Ok(Box::new(StubHost {
                stop: Arc::new(AtomicBool::new(false)),
                checkpoints: counted,
            }) as Box<dyn ScriptHost>)
        }))
        .unwrap();

        (thread, checkpoints)
    }

    const LONG: Duration = Duration::from_secs(30);

    #[tokio::test]
    async fn a_script_runs_and_its_promise_jobs_after_it() {
        let (thread, checkpoints) = stub_thread();

        let value = thread.execute("abc".to_owned(), ScriptOrigin::default(), LONG).await;
        assert_eq!(value, Ok(ScriptValue::Number(3.0)));
        assert_eq!(checkpoints.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_script_over_its_limit_is_terminated_and_the_next_one_runs() {
        let (thread, checkpoints) = stub_thread();

        let spin = thread
            .execute("spin".to_owned(), ScriptOrigin::default(), Duration::from_millis(50))
            .await;
        assert_eq!(spin.map_err(|e| e.kind), Err(ScriptErrorKind::Terminated));
        // no promise jobs for a script that did not finish
        assert_eq!(checkpoints.load(Ordering::SeqCst), 0);

        let value = thread.execute("ab".to_owned(), ScriptOrigin::default(), LONG).await;
        assert_eq!(value, Ok(ScriptValue::Number(2.0)));
    }

    #[tokio::test]
    async fn a_script_queued_behind_a_runaway_is_not_terminated_with_it() {
        let (thread, _) = stub_thread();
        let thread = Arc::new(thread);

        let runaway = Arc::clone(&thread);
        let spin = tokio::spawn(async move {
            runaway
                .execute("spin".to_owned(), ScriptOrigin::default(), Duration::from_millis(100))
                .await
        });
        // queued while the first still spins; its own limit is far away
        tokio::time::sleep(Duration::from_millis(20)).await;
        let value = thread.execute("abcd".to_owned(), ScriptOrigin::default(), LONG).await;

        assert_eq!(value, Ok(ScriptValue::Number(4.0)));
        assert_eq!(
            spin.await.unwrap().map_err(|e| e.kind),
            Err(ScriptErrorKind::Terminated)
        );
    }

    #[tokio::test]
    async fn without_a_host_every_script_fails_with_the_reason() {
        let thread = ScriptThread::spawn(Box::new(|| Err(unavailable("no engine")))).unwrap();

        for _ in 0..2 {
            let error = thread
                .execute("1".to_owned(), ScriptOrigin::default(), LONG)
                .await
                .unwrap_err();
            assert_eq!(error.kind, ScriptErrorKind::Unavailable);
            assert_eq!(error.message, "no engine");
        }
    }

    #[test]
    fn values_as_json() {
        use serde_json::json;

        assert_eq!(to_json(ScriptValue::Undefined), json!(null));
        assert_eq!(to_json(ScriptValue::Null), json!(null));
        assert_eq!(to_json(ScriptValue::Bool(true)), json!(true));
        assert_eq!(to_json(ScriptValue::Number(2.0)), json!(2.0));
        assert_eq!(to_json(ScriptValue::Number(f64::NAN)), json!(null));
        assert_eq!(to_json(ScriptValue::String("a".to_owned())), json!("a"));
        assert_eq!(to_json(ScriptValue::Json(r#"{"a":[1]}"#.to_owned())), json!({"a": [1]}));
    }

    #[cfg(not(feature = "js"))]
    #[test]
    fn a_build_without_js_has_no_host() {
        let error = page_host(ScriptLimits { heap_bytes: 1 << 20 }).err().unwrap();
        assert_eq!(error.kind, ScriptErrorKind::Unavailable);
    }
}
