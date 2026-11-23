//! Page scripts in a resident renderer.
//!
//! The renderer has one thread and may not start another, so no watchdog can end a
//! script from outside as the in-process script thread does. The bound is the process's
//! own alarm instead (`gosub_sandbox::arm_soft_deadline`): when a script's time is up its
//! handler asks the engine to end the script, and if that has not happened a moment
//! later the alarm ends the renderer, as it does for a render that never finishes.

use crate::engine::script::{self, ScriptLimits};
use crate::fork_server::protocol::{
    clip_script_text, ScriptReport, ScriptSettings, MAX_SCRIPT_REPORTS, MAX_SCRIPT_TEXT,
};
use crate::net::resource_loader::ResourceLoader;
use crate::net::types::ResourceKind;
use gosub_html5::parser::ScriptSource;
use gosub_interface::script::{ScriptError, ScriptErrorKind, ScriptHost, ScriptOrigin, ScriptTerminator, ScriptValue};
use parking_lot::Mutex;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How long a script that was asked to stop has before the renderer ends itself.
const GRACE: Duration = Duration::from_secs(2);

/// The most a document's script heap may be, whatever the tab's setting: a renderer's
/// memory limit covers the page as well, and a heap that runs into it takes the process
/// down instead of the script.
const MAX_HEAP_BYTES: u64 = crate::fork_server::client::RENDERER_DATA_LIMIT / 4;

/// Reports kept below the wire's limit, so the closing notice and a result still fit.
const MAX_KEPT_REPORTS: usize = MAX_SCRIPT_REPORTS - 2;

/// How to end the script that is running, for [`expired`]. Only set while one runs.
static RUNNING: Mutex<Option<ScriptTerminator>> = Mutex::new(None);

/// The alarm's handler: runs in the middle of the script it is about. `try_lock`, since
/// it must not wait for anything; `RUNNING` is only locked while no alarm is armed.
///
/// Asking V8 to end a script is made for another thread, not for a signal handler on the
/// thread that runs it: V8 takes a lock for it. Should the script hold that lock at this
/// very moment the handler never returns, and the alarm's second expiry ends the renderer
/// - which is all it could do without the handler.
fn expired() {
    if let Some(running) = RUNNING.try_lock() {
        if let Some(terminator) = running.as_ref() {
            terminator.terminate();
        }
    }
}

#[derive(Default)]
struct Reports {
    list: Vec<ScriptReport>,
    /// The URL of the script that is running, for its console lines.
    origin: String,
}

impl Reports {
    /// Keeps `report`, up to what one exchange carries; says once that more was dropped.
    fn push(&mut self, report: ScriptReport) {
        match self.list.len() {
            n if n < MAX_KEPT_REPORTS => self.list.push(report),
            MAX_KEPT_REPORTS => self.list.push(ScriptReport::Console {
                level: gosub_interface::script::ConsoleLevel::Warn.into(),
                text: "more script output of this page was dropped".to_owned(),
                origin: String::new(),
            }),
            _ => {}
        }
    }

    fn error(&mut self, error: &ScriptError) {
        self.push(ScriptReport::Error {
            message: clip_script_text(error.message.clone()),
            line: error.line,
            column: error.column,
        });
    }
}

struct Inner {
    host: Box<dyn ScriptHost>,
    /// How long one script of the document may run.
    limit: Duration,
    /// When the request being served runs out; no script gets longer than that.
    request_ends: Instant,
    /// When the time of the script that ran last is up, for its promise jobs too.
    jobs_end: Instant,
}

impl Inner {
    /// Runs `work` on the host with the alarm set to `limit`, and puts the request's own
    /// deadline back after it.
    fn bounded<T>(&mut self, limit: Duration, work: impl FnOnce(&mut dyn ScriptHost) -> T) -> T {
        let left = self.request_ends.saturating_duration_since(Instant::now());
        // before the alarm is set: what it asks for from here on is about this work
        self.host.forget_termination();
        *RUNNING.lock() = Some(self.host.terminator());
        if gosub_sandbox::arm_soft_deadline(limit.min(left), GRACE, expired).is_err() {
            gosub_sandbox::exit_now(1);
        }

        let out = work(self.host.as_mut());

        let left = self
            .request_ends
            .saturating_duration_since(Instant::now())
            .max(Duration::from_millis(1));
        if gosub_sandbox::arm_deadline(left).is_err() {
            gosub_sandbox::exit_now(1);
        }
        *RUNNING.lock() = None;

        out
    }

    /// Runs a script with `limit` to spend. What it leaves is for its promise jobs.
    fn execute(&mut self, limit: Duration, source: &str, origin: &ScriptOrigin) -> Result<ScriptValue, ScriptError> {
        // an instant, not what is left: the jobs may start later than the script ended
        self.jobs_end = Instant::now() + limit;
        self.bounded(limit, |host| host.execute(source, origin))
    }

    /// Runs the promise jobs of the script that ran last, in the time it left. The alarm
    /// asks once, so they get a deadline of their own.
    fn jobs(&mut self) -> Result<(), ScriptError> {
        let left = self.jobs_end.saturating_duration_since(Instant::now());
        self.bounded(left, |host| host.microtask_checkpoint())
    }
}

/// The script host of one document in this renderer, and what its scripts reported
/// since the last [`PageScripts::take_reports`].
pub(super) struct PageScripts {
    inner: Rc<RefCell<Inner>>,
    reports: Arc<Mutex<Reports>>,
    /// The document's URL, which names a script that is not the page's own.
    url: String,
}

impl PageScripts {
    /// A host for the document at `url`, or `None` when this build has no script engine.
    pub(super) fn new(settings: ScriptSettings, url: &str, request_ends: Instant) -> Option<Self> {
        let reports = Arc::new(Mutex::new(Reports::default()));
        let console = Arc::clone(&reports);
        let limits = ScriptLimits {
            heap_bytes: settings.heap_limit_mb.saturating_mul(1024 * 1024).min(MAX_HEAP_BYTES) as usize,
            single_threaded: true,
        };
        let host = script::page_host(
            limits,
            Arc::new(move |level, text| {
                let mut reports = console.lock();
                let origin = reports.origin.clone();
                reports.push(ScriptReport::Console {
                    level: level.into(),
                    text: clip_script_text(text),
                    origin,
                });
            }),
        );
        let host = match host {
            Ok(host) => host,
            Err(e) => {
                log::warn!("{url} runs no scripts: {}", e.message);
                return None;
            }
        };

        Some(Self {
            inner: Rc::new(RefCell::new(Inner {
                host,
                limit: Duration::from_millis(settings.max_run_ms),
                request_ends,
                jobs_end: Instant::now(),
            })),
            reports,
            url: url.to_owned(),
        })
    }

    /// A request that may run scripts has begun; it ends at `request_ends`.
    pub(super) fn begin_request(&self, request_ends: Instant) {
        self.inner.borrow_mut().request_ends = request_ends;
    }

    /// The host as the HTML parser takes it.
    pub(super) fn for_parser(&self) -> Box<dyn ScriptHost> {
        Box::new(Parsing {
            inner: Rc::clone(&self.inner),
            reports: Arc::clone(&self.reports),
        })
    }

    /// Runs `source` and the promise jobs it queues, and reports its value or its error.
    pub(super) fn execute(&self, source: &str, limit: Duration) {
        let origin = ScriptOrigin {
            url: self.url.clone(),
            line: 0,
        };
        self.reports.lock().origin = clip_script_text(origin.url.clone());
        let mut inner = self.inner.borrow_mut();
        let result = inner.execute(limit, source, &origin);
        // Then its promise jobs, unless it was ended (as the script thread has it), in
        // what is left of `limit`.
        let result = match result {
            Err(e) if e.kind != ScriptErrorKind::Exception => Err(e),
            result => {
                let jobs = inner.jobs();
                result.and_then(|value| jobs.map(|()| value))
            }
        };
        drop(inner);

        let mut reports = self.reports.lock();
        match result.map(|value| script::to_json(value).to_string()) {
            // not through `push`: the answer is what the request was for
            Ok(json) if json.len() <= MAX_SCRIPT_TEXT => reports.list.push(ScriptReport::Result { json }),
            Ok(_) => reports.list.push(ScriptReport::Error {
                message: "the script's value is too large to report".to_owned(),
                line: 0,
                column: 0,
            }),
            Err(e) => reports.list.push(ScriptReport::Error {
                message: clip_script_text(e.message),
                line: e.line,
                column: e.column,
            }),
        }
    }

    pub(super) fn take_reports(&self) -> Vec<ScriptReport> {
        std::mem::take(&mut self.reports.lock().list)
    }
}

/// The host during a parse: each call bounded, each failure reported.
struct Parsing {
    inner: Rc<RefCell<Inner>>,
    reports: Arc<Mutex<Reports>>,
}

impl ScriptHost for Parsing {
    fn execute(&mut self, source: &str, origin: &ScriptOrigin) -> Result<ScriptValue, ScriptError> {
        self.reports.lock().origin = clip_script_text(origin.url.clone());
        let mut inner = self.inner.borrow_mut();
        let limit = inner.limit;
        let result = inner.execute(limit, source, origin);
        if let Err(e) = &result {
            self.reports.lock().error(e);
        }
        result
    }

    fn microtask_checkpoint(&mut self) -> Result<(), ScriptError> {
        let mut inner = self.inner.borrow_mut();
        let result = inner.jobs();
        if let Err(e) = &result {
            self.reports.lock().error(e);
        }
        result
    }

    fn terminator(&self) -> ScriptTerminator {
        self.inner.borrow().host.terminator()
    }
}

/// External scripts for a parse in this renderer: fetched through the broker, like the
/// page's stylesheets.
#[derive(Debug)]
pub(super) struct LoaderScripts(pub Arc<dyn ResourceLoader>);

impl ScriptSource for LoaderScripts {
    fn fetch_blocking(&self, url: &str) -> Option<String> {
        let (_, bytes) = self.0.fetch(url, ResourceKind::Script { blocking: true })?;

        Some(crate::engine::resource_pipeline::html::script_text(&bytes))
    }
}
