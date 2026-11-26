//! Running scripts for a tab's page.
//!
//! A page's [`ScriptHost`] lives on a thread of its own. A script engine ties its global
//! scope to the thread that made it, and a script runs for as long as it likes, which a tab
//! worker cannot afford to wait for. The worker hands over source text and gets the result
//! back as data; a script that runs past its time limit is terminated from here.

use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use gosub_interface::script::{
    ConsoleLevel, ScriptError, ScriptErrorKind, ScriptHost, ScriptOrigin, ScriptTerminator, ScriptValue,
};
use parking_lot::Mutex;
use tokio::sync::oneshot;

#[cfg(feature = "js")]
mod console;
#[cfg(feature = "js")]
mod v8;

/// Where a page's console output goes. Called on the script thread.
pub(crate) type ConsoleSink = Arc<dyn Fn(ConsoleLevel, String) + Send + Sync>;

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
    /// The engine starts no thread of its own. For a renderer process, which may not
    /// start any. Decided by the first host of the process.
    #[cfg_attr(not(feature = "js"), allow(dead_code))]
    pub single_threaded: bool,
}

/// The engine's script host, or why there is none.
#[cfg(feature = "js")]
pub(crate) fn page_host(limits: ScriptLimits, console: ConsoleSink) -> Result<Box<dyn ScriptHost>, ScriptError> {
    Ok(Box::new(v8::V8ScriptHost::new(limits, console)?))
}

#[cfg(not(feature = "js"))]
pub(crate) fn page_host(_limits: ScriptLimits, _console: ConsoleSink) -> Result<Box<dyn ScriptHost>, ScriptError> {
    Err(unavailable("this build has no JavaScript engine (cargo feature `js`)"))
}

pub(crate) fn unavailable(why: &str) -> ScriptError {
    ScriptError::new(ScriptErrorKind::Unavailable, why)
}

/// A page's console line in the engine's own log.
#[cfg_attr(not(feature = "js"), allow(dead_code))] // nothing writes to a console without an engine
pub(crate) fn log_console(level: ConsoleLevel, text: &str) {
    match level {
        ConsoleLevel::Debug => log::debug!(target: "console", "{text}"),
        ConsoleLevel::Log | ConsoleLevel::Info => log::info!(target: "console", "{text}"),
        ConsoleLevel::Warn => log::warn!(target: "console", "{text}"),
        ConsoleLevel::Error => log::error!(target: "console", "{text}"),
    }
}

struct Job {
    id: u64,
    source: String,
    origin: ScriptOrigin,
    reply: Box<dyn FnOnce(ScriptResult) + Send>,
}

struct Shared {
    /// The job the thread is running and when it started. A script is only terminated with
    /// this held and naming it, so that a request never lands on the script after it.
    running: Mutex<Option<(u64, std::time::Instant)>>,
    /// Set once the host exists.
    terminator: std::sync::OnceLock<ScriptTerminator>,
    /// URL of the script that is running, or ran last.
    origin: Arc<Mutex<String>>,
}

enum Queue {
    /// No script yet, so no thread and no host.
    Idle(Option<HostFactory>),
    Started(mpsc::Sender<Job>),
}

/// The script thread of one page. It starts with the first script. Dropping this ends the
/// thread once the script it is running returns.
pub(crate) struct ScriptThread {
    queue: Mutex<Queue>,
    shared: Arc<Shared>,
    next_id: std::sync::atomic::AtomicU64,
}

impl ScriptThread {
    /// `make` runs on the script thread. If it fails, every script fails with its error.
    /// `origin` is kept at the URL of the script that is running.
    pub(crate) fn new(make: HostFactory, origin: Arc<Mutex<String>>) -> Self {
        Self {
            queue: Mutex::new(Queue::Idle(Some(make))),
            shared: Arc::new(Shared {
                running: Mutex::new(None),
                terminator: std::sync::OnceLock::new(),
                origin,
            }),
            next_id: std::sync::atomic::AtomicU64::new(1),
        }
    }

    /// Queues a script and returns its id. Scripts run in the order they are queued, and
    /// `reply` gets each result on the script thread, so in that order too.
    pub(crate) fn submit(
        &self,
        source: String,
        origin: ScriptOrigin,
        reply: Box<dyn FnOnce(ScriptResult) + Send>,
    ) -> Result<u64, ScriptError> {
        let id = self.next_id.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let job = Job {
            id,
            source,
            origin,
            reply,
        };

        let mut queue = self.queue.lock();
        if let Queue::Idle(make) = &mut *queue {
            let make = make
                .take()
                .ok_or_else(|| unavailable("the script thread did not start"))?;
            let (jobs, incoming) = mpsc::channel::<Job>();
            let shared = Arc::clone(&self.shared);
            std::thread::Builder::new()
                .name("gosub-script".to_owned())
                .spawn(move || run_jobs(make, &incoming, &shared))
                .map_err(|e| unavailable(&format!("no script thread: {e}")))?;
            *queue = Queue::Started(jobs);
        }

        match &*queue {
            Queue::Started(jobs) if jobs.send(job).is_ok() => Ok(id),
            _ => Err(unavailable("the script thread is gone")),
        }
    }

    /// Ends script `id` if it is still running `limit` after it started. Returns when
    /// `done` does, which is for the script's `reply` to arrange.
    pub(crate) async fn watch(&self, id: u64, limit: Duration, mut done: oneshot::Receiver<()>) {
        let mut wait = limit;
        while tokio::time::timeout(wait, &mut done).await.is_err() {
            wait = self.check(id, limit);
        }
    }

    /// Runs `source`, then the promise jobs it queued. A script still running `limit`
    /// after it started is terminated.
    #[cfg(test)]
    pub(crate) async fn execute(&self, source: String, origin: ScriptOrigin, limit: Duration) -> ScriptResult {
        let (reply, result) = oneshot::channel();
        let (finished, done) = oneshot::channel();
        let id = self.submit(
            source,
            origin,
            Box::new(move |result| {
                let _ = reply.send(result);
                let _ = finished.send(());
            }),
        )?;
        self.watch(id, limit, done).await;

        result.await.map_err(|_| unavailable("the script thread is gone"))?
    }

    /// The same for a thread that may block: the HTML parser's.
    pub(crate) fn execute_blocking(&self, source: String, origin: ScriptOrigin, limit: Duration) -> ScriptResult {
        let (reply, result) = mpsc::channel();
        let id = self.submit(
            source,
            origin,
            Box::new(move |result| {
                let _ = reply.send(result);
            }),
        )?;

        let mut wait = limit;
        loop {
            match result.recv_timeout(wait) {
                Ok(result) => return result,
                Err(mpsc::RecvTimeoutError::Disconnected) => return Err(unavailable("the script thread is gone")),
                Err(mpsc::RecvTimeoutError::Timeout) => wait = self.check(id, limit),
            }
        }
    }

    /// Terminates script `id` if it has run for `limit`, and returns how long to wait
    /// before looking again.
    fn check(&self, id: u64, limit: Duration) -> Duration {
        let running = self.shared.running.lock();
        match *running {
            Some((running_id, since)) if running_id == id => {
                let ran = since.elapsed();
                if ran < limit {
                    return limit - ran;
                }
                if let Some(terminator) = self.shared.terminator.get() {
                    terminator.terminate();
                }
                // Asked again until the script has ended: a request can arrive just before
                // the script starts, where the host drops it.
                TERMINATE_RETRY
            }
            // still queued behind another script: its time has not started
            _ => limit,
        }
    }
}

fn run_jobs(make: HostFactory, queue: &mpsc::Receiver<Job>, shared: &Shared) {
    let mut host = make();
    if let Ok(host) = &host {
        let _ = shared.terminator.set(host.terminator());
    }

    for job in queue {
        shared.origin.lock().clone_from(&job.origin.url);
        *shared.running.lock() = Some((job.id, std::time::Instant::now()));
        let result = match &mut host {
            Ok(host) => run(host.as_mut(), &job),
            Err(e) => Err(e.clone()),
        };
        *shared.running.lock() = None;

        (job.reply)(result);
    }
}

fn run(host: &mut dyn ScriptHost, job: &Job) -> ScriptResult {
    // A request to stop that came too late for the previous script is not for this one.
    // One that is still meant is made again (`TERMINATE_RETRY`).
    host.forget_termination();
    let result = host.execute(&job.source, &job.origin);
    // A script that threw still has the promise jobs it queued run now, before the next
    // script. One that was ended does not: there is no telling what its jobs would do.
    if matches!(&result, Err(e) if e.kind != ScriptErrorKind::Exception) {
        return result;
    }
    host.forget_termination();
    let jobs = host.microtask_checkpoint();

    let value = result?;
    jobs?;
    Ok(value)
}

/// A page's script thread as the HTML parser sees it: a host that runs each inline script
/// there and waits for it. `on_error` gets the scripts that fail.
#[derive(Clone)]
pub(crate) struct ParserScripts {
    pub thread: Arc<ScriptThread>,
    /// How long one script may run.
    pub limit: Duration,
    pub on_error: Arc<dyn Fn(&ScriptError) + Send + Sync>,
}

impl std::fmt::Debug for ParserScripts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParserScripts").field("limit", &self.limit).finish()
    }
}

impl ScriptHost for ParserScripts {
    fn execute(&mut self, source: &str, origin: &ScriptOrigin) -> ScriptResult {
        let result = self
            .thread
            .execute_blocking(source.to_owned(), origin.clone(), self.limit);
        if let Err(e) = &result {
            (self.on_error)(e);
        }

        result
    }

    /// Nothing to do: the script thread runs a script's promise jobs right after it.
    fn microtask_checkpoint(&mut self) -> Result<(), ScriptError> {
        Ok(())
    }

    /// The thread ends a script that runs past the limit; nothing else stops one.
    fn terminator(&self) -> ScriptTerminator {
        ScriptTerminator::noop()
    }
}

impl crate::html::ScriptHostSource for ParserScripts {
    fn host(&self) -> Box<dyn ScriptHost + Send> {
        Box::new(self.clone())
    }
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

    /// Spins until terminated or sleeps, for sources that say so; else answers with the
    /// source length. A sleep that was asked to end says so when it wakes.
    struct StubHost {
        stop: Arc<AtomicBool>,
        checkpoints: Arc<AtomicUsize>,
    }

    impl ScriptHost for StubHost {
        fn execute(&mut self, source: &str, _origin: &ScriptOrigin) -> ScriptResult {
            // like the real host: a request from before this script is not for it
            self.stop.store(false, Ordering::SeqCst);
            if source == "throw" {
                return Err(ScriptError::new(ScriptErrorKind::Exception, "thrown"));
            }
            if let Some(ms) = source.strip_prefix("sleep ") {
                std::thread::sleep(Duration::from_millis(ms.parse().unwrap()));
                if self.stop.load(Ordering::SeqCst) {
                    return Err(ScriptError::new(ScriptErrorKind::Terminated, "terminated"));
                }
                return Ok(ScriptValue::Null);
            }
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
        let thread = ScriptThread::new(
            Box::new(move || {
                Ok(Box::new(StubHost {
                    stop: Arc::new(AtomicBool::new(false)),
                    checkpoints: counted,
                }) as Box<dyn ScriptHost>)
            }),
            Arc::default(),
        );

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
    async fn a_script_that_throws_still_has_its_promise_jobs_run() {
        let (thread, checkpoints) = stub_thread();

        let thrown = thread.execute("throw".to_owned(), ScriptOrigin::default(), LONG).await;
        assert_eq!(thrown.map_err(|e| e.kind), Err(ScriptErrorKind::Exception));
        assert_eq!(checkpoints.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn the_time_limit_counts_from_when_a_script_starts() {
        let (thread, _) = stub_thread();
        let thread = Arc::new(thread);

        // 500 ms ahead in the queue, then 100 ms of its own: over a 300 ms limit only if
        // the wait counted. The own run is kept short: on the macOS CI runners a sleep
        // overshoots by tens of milliseconds, and 150 ms under 200 was ended there.
        let first = Arc::clone(&thread);
        let ahead = tokio::spawn(async move {
            first
                .execute("sleep 500".to_owned(), ScriptOrigin::default(), LONG)
                .await
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        let queued = thread
            .execute(
                "sleep 100".to_owned(),
                ScriptOrigin::default(),
                Duration::from_millis(300),
            )
            .await;

        assert_eq!(queued, Ok(ScriptValue::Null));
        assert_eq!(ahead.await.unwrap(), Ok(ScriptValue::Null));
    }

    #[test]
    fn results_come_in_the_order_the_scripts_were_queued() {
        let (thread, _) = stub_thread();
        let (results, received) = mpsc::channel();

        for n in 0..50usize {
            let results = results.clone();
            thread
                .submit(
                    "x".repeat(n),
                    ScriptOrigin::default(),
                    Box::new(move |result| {
                        let _ = results.send(result);
                    }),
                )
                .unwrap();
        }

        let order: Vec<ScriptResult> = received.iter().take(50).collect();
        let expected: Vec<ScriptResult> = (0..50).map(|n| Ok(ScriptValue::Number(f64::from(n)))).collect();
        assert_eq!(order, expected);
    }

    #[tokio::test]
    async fn without_a_host_every_script_fails_with_the_reason() {
        let thread = ScriptThread::new(Box::new(|| Err(unavailable("no engine"))), Arc::default());

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
    fn the_parsers_host_runs_scripts_on_the_thread_and_reports_the_failures() {
        let (thread, checkpoints) = stub_thread();
        let origin_cell = Arc::clone(&thread.shared.origin);
        let failures = Arc::new(AtomicUsize::new(0));
        let failed = Arc::clone(&failures);
        let source = ParserScripts {
            thread: Arc::new(thread),
            limit: Duration::from_millis(50),
            on_error: Arc::new(move |_| {
                failed.fetch_add(1, Ordering::SeqCst);
            }),
        };
        let mut host = crate::html::ScriptHostSource::host(&source);
        let origin = ScriptOrigin {
            url: "https://example.test/page".to_owned(),
            line: 3,
        };

        assert_eq!(host.execute("abc", &origin), Ok(ScriptValue::Number(3.0)));
        assert_eq!(*origin_cell.lock(), "https://example.test/page");
        // the thread ran the promise jobs; the parser's own checkpoint adds nothing
        assert_eq!(host.microtask_checkpoint(), Ok(()));
        assert_eq!(checkpoints.load(Ordering::SeqCst), 1);
        assert_eq!(failures.load(Ordering::SeqCst), 0);

        // over the limit: terminated, and reported
        assert_eq!(
            host.execute("spin", &origin).map_err(|e| e.kind),
            Err(ScriptErrorKind::Terminated)
        );
        assert_eq!(failures.load(Ordering::SeqCst), 1);
        assert_eq!(host.execute("ab", &origin), Ok(ScriptValue::Number(2.0)));
    }

    #[test]
    fn no_thread_starts_before_the_first_script() {
        let made = Arc::new(AtomicBool::new(false));
        let seen = Arc::clone(&made);
        let thread = ScriptThread::new(
            Box::new(move || {
                seen.store(true, Ordering::SeqCst);
                Err(unavailable("unused"))
            }),
            Arc::default(),
        );

        assert!(matches!(&*thread.queue.lock(), Queue::Idle(Some(_))));
        let _ = thread.execute_blocking("1".to_owned(), ScriptOrigin::default(), LONG);
        assert!(made.load(Ordering::SeqCst));
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
        let error = page_host(
            ScriptLimits {
                heap_bytes: 1 << 20,
                single_threaded: false,
            },
            Arc::new(|_, _| {}),
        )
        .err()
        .unwrap();
        assert_eq!(error.kind, ScriptErrorKind::Unavailable);
    }
}
