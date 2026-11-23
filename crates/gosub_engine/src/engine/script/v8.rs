//! The V8-backed script host. The only place in the engine that names `gosub_v8`.

use gosub_interface::script::{ScriptError, ScriptErrorKind, ScriptHost, ScriptOrigin, ScriptTerminator, ScriptValue};
use gosub_v8::{V8Context, V8ContextOptions, V8Engine, V8Error, V8EvalError, V8Options, V8Plain};

use super::{console, ConsoleSink, ScriptLimits};

pub(super) struct V8ScriptHost {
    context: V8Context,
}

impl V8ScriptHost {
    pub(super) fn new(limits: ScriptLimits, sink: ConsoleSink) -> Result<Self, ScriptError> {
        // Interpreter only: no executable memory, which a sandboxed renderer may not map.
        // The first initialization in a process decides, and this is the only one.
        V8Engine::initialize_with(&V8Options {
            jitless: true,
            single_threaded: limits.single_threaded,
            ..Default::default()
        });

        let context = V8Context::with_options(&V8ContextOptions {
            heap_limit: Some(limits.heap_bytes),
            explicit_microtasks: true,
        })
        .map_err(|e| ScriptError::new(ScriptErrorKind::Unavailable, format!("no V8 context: {e}")))?;

        console::install::<V8Engine>(V8Context::clone(&context), sink)
            .map_err(|e| ScriptError::new(ScriptErrorKind::Unavailable, format!("no console: {e}")))?;

        Ok(Self { context })
    }
}

impl ScriptHost for V8ScriptHost {
    fn execute(&mut self, source: &str, origin: &ScriptOrigin) -> Result<ScriptValue, ScriptError> {
        match self.context.evaluate(source, &origin.url, origin.line) {
            Ok(value) => Ok(match value {
                V8Plain::Undefined => ScriptValue::Undefined,
                V8Plain::Null => ScriptValue::Null,
                V8Plain::Bool(b) => ScriptValue::Bool(b),
                V8Plain::Number(n) => ScriptValue::Number(n),
                V8Plain::String(s) => ScriptValue::String(s),
                V8Plain::Json(json) => ScriptValue::Json(json),
            }),
            Err(V8EvalError::Exception(e)) => Err(ScriptError {
                kind: ScriptErrorKind::Exception,
                message: e.message,
                line: e.line,
                column: e.column,
            }),
            Err(V8EvalError::Ended(e)) => Err(ended(e)),
        }
    }

    fn microtask_checkpoint(&mut self) -> Result<(), ScriptError> {
        self.context
            .microtask_checkpoint()
            .map_err(|e| match e.downcast_ref::<V8Error>() {
                Some(ended_by) => ended(*ended_by),
                None => ScriptError::new(ScriptErrorKind::Exception, e.to_string()),
            })
    }

    fn terminator(&self) -> ScriptTerminator {
        let terminator = self.context.terminator();
        ScriptTerminator::new(move || {
            terminator.terminate();
        })
    }

    fn forget_termination(&mut self) {
        self.context.cancel_termination();
    }
}

fn ended(by: V8Error) -> ScriptError {
    let kind = match by {
        V8Error::Terminated => ScriptErrorKind::Terminated,
        V8Error::OutOfMemory => ScriptErrorKind::OutOfMemory,
    };
    ScriptError::new(kind, by.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gosub_interface::script::ConsoleLevel;
    use parking_lot::Mutex;
    use std::sync::Arc;
    use std::time::Duration;

    type Lines = Arc<Mutex<Vec<(ConsoleLevel, String)>>>;

    fn host_and_console() -> (V8ScriptHost, Lines) {
        let lines = Lines::default();
        let seen = Arc::clone(&lines);
        let host = V8ScriptHost::new(
            ScriptLimits {
                heap_bytes: 32 * 1024 * 1024,
                single_threaded: false,
            },
            Arc::new(move |level, text| seen.lock().push((level, text))),
        )
        .unwrap();

        (host, lines)
    }

    fn host() -> V8ScriptHost {
        host_and_console().0
    }

    /// What `source` writes to the console.
    fn console(source: &str) -> Vec<(ConsoleLevel, String)> {
        let (mut host, lines) = host_and_console();
        assert_eq!(host.execute(source, &origin()), Ok(ScriptValue::Undefined), "{source}");

        let lines = lines.lock().clone();
        lines
    }

    fn line(level: ConsoleLevel, text: &str) -> (ConsoleLevel, String) {
        (level, text.to_owned())
    }

    fn origin() -> ScriptOrigin {
        ScriptOrigin {
            url: "https://example.org/".to_owned(),
            line: 0,
        }
    }

    #[test]
    fn a_script_evaluates_to_data() {
        let mut host = host();

        assert_eq!(host.execute("1 + 1", &origin()), Ok(ScriptValue::Number(2.0)));
        assert_eq!(
            host.execute("({a: 1})", &origin()),
            Ok(ScriptValue::Json(r#"{"a":1}"#.to_owned()))
        );
        assert_eq!(host.execute("undefined", &origin()), Ok(ScriptValue::Undefined));
    }

    #[test]
    fn the_host_is_jitless() {
        // no WebAssembly without a JIT
        assert_eq!(
            host().execute("typeof WebAssembly", &origin()),
            Ok(ScriptValue::String("undefined".to_owned()))
        );
    }

    #[test]
    fn an_exception_is_an_error_with_its_position() {
        let error = host().execute("\n  missing()", &origin()).unwrap_err();

        assert_eq!(error.kind, ScriptErrorKind::Exception);
        assert_eq!(error.message, "ReferenceError: missing is not defined");
        assert_eq!((error.line, error.column), (2, 3));
    }

    #[test]
    fn promise_jobs_wait_for_the_checkpoint() {
        let mut host = host();

        assert_eq!(
            host.execute(
                "var seen = 0; Promise.resolve().then(() => { seen = 1 }); seen",
                &origin()
            ),
            Ok(ScriptValue::Number(0.0))
        );
        assert_eq!(host.microtask_checkpoint(), Ok(()));
        assert_eq!(host.execute("seen", &origin()), Ok(ScriptValue::Number(1.0)));
    }

    #[test]
    fn a_running_script_is_terminated_and_the_host_runs_again() {
        let mut host = host();
        let terminator = host.terminator();
        let ended = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let seen = std::sync::Arc::clone(&ended);
        // asked until it has worked
        let stopper = std::thread::spawn(move || {
            while !seen.load(std::sync::atomic::Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(50));
                terminator.terminate();
            }
        });

        let error = host.execute("for (;;) {}", &origin()).unwrap_err();
        ended.store(true, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(error.kind, ScriptErrorKind::Terminated);
        stopper.join().unwrap();
        // the stopper may have asked once more after the script ended
        host.forget_termination();
        assert_eq!(host.execute("1 + 1", &origin()), Ok(ScriptValue::Number(2.0)));
    }

    #[test]
    fn a_forgotten_termination_request_does_not_end_the_next_script() {
        let mut host = host();

        host.terminator().terminate();
        host.forget_termination();
        assert_eq!(host.execute("1 + 1", &origin()), Ok(ScriptValue::Number(2.0)));
    }

    #[test]
    fn a_termination_request_made_before_the_work_starts_ends_it() {
        // whoever times a script forgets the old request first and starts the clock
        // then; a request that follows is for this script, however early it comes
        let mut host = host();
        host.forget_termination();
        host.terminator().terminate();
        assert_eq!(
            host.execute("globalThis.ran = 1", &origin()).unwrap_err().kind,
            ScriptErrorKind::Terminated
        );

        host.execute("Promise.resolve().then(() => { globalThis.job = 1 })", &origin())
            .unwrap();
        host.terminator().terminate();
        assert_eq!(
            host.microtask_checkpoint().unwrap_err().kind,
            ScriptErrorKind::Terminated
        );

        host.forget_termination();
        assert_eq!(
            host.execute("typeof ran + typeof job", &origin()),
            Ok(ScriptValue::String("undefinedundefined".to_owned()))
        );
    }

    #[test]
    fn the_heap_limit_ends_a_script() {
        let mut host = host();

        let hoard = "(function () { const a = []; for (;;) a.push(new Array(1000).fill(0)); })()";
        let error = host.execute(hoard, &origin()).unwrap_err();
        assert_eq!(error.kind, ScriptErrorKind::OutOfMemory);
        assert_eq!(host.execute("1 + 1", &origin()), Ok(ScriptValue::Number(2.0)));
    }

    #[test]
    fn console_log_levels() {
        assert_eq!(console("console.log('a', 1)"), [line(ConsoleLevel::Log, "a 1")]);
        assert_eq!(console("console.info('i')"), [line(ConsoleLevel::Info, "i")]);
        assert_eq!(console("console.warn('w')"), [line(ConsoleLevel::Warn, "w")]);
        assert_eq!(console("console.error('e')"), [line(ConsoleLevel::Error, "e")]);
        assert_eq!(console("console.debug('d')"), [line(ConsoleLevel::Debug, "d")]);
        assert_eq!(console("console.trace('t')"), [line(ConsoleLevel::Log, "t")]);
        assert_eq!(console("console.log()"), []);
    }

    #[test]
    fn console_arguments_are_stringified() {
        assert_eq!(
            console("console.log(1.5, true, null, undefined, [1, 2], {})"),
            [line(ConsoleLevel::Log, "1.5 true null undefined 1,2 [object Object]")]
        );
    }

    #[test]
    fn console_assert_prints_only_when_the_condition_fails() {
        assert_eq!(console("console.assert(true, 'fine')"), []);
        assert_eq!(
            console("console.assert(1 === 2, 'broken')"),
            [line(ConsoleLevel::Error, "Assertion failed: broken")]
        );
        assert_eq!(
            console("console.assert(0, 'falsy')"),
            [line(ConsoleLevel::Error, "Assertion failed: falsy")]
        );
        assert_eq!(
            console("console.assert()"),
            [line(ConsoleLevel::Error, "Assertion failed")]
        );
    }

    #[test]
    fn console_count_and_count_reset() {
        assert_eq!(
            console("console.count(); console.count('x'); console.count(); console.countReset(); console.count()"),
            [
                line(ConsoleLevel::Log, "default: 1"),
                line(ConsoleLevel::Log, "x: 1"),
                line(ConsoleLevel::Log, "default: 2"),
                line(ConsoleLevel::Log, "default: 1"),
            ]
        );
        assert_eq!(
            console("console.countReset('never')"),
            [line(ConsoleLevel::Warn, "label does not exist")]
        );
    }

    #[test]
    fn console_groups_indent_what_is_inside() {
        assert_eq!(
            console(
                "console.group('outer'); console.log('in'); console.groupCollapsed('inner'); console.log('deep'); \
                 console.groupEnd(); console.log('in again'); console.groupEnd(); console.log('out')"
            ),
            [
                line(ConsoleLevel::Log, "outer"),
                line(ConsoleLevel::Log, "  in"),
                line(ConsoleLevel::Log, "  inner"),
                line(ConsoleLevel::Log, "    deep"),
                line(ConsoleLevel::Log, "  in again"),
                line(ConsoleLevel::Log, "out"),
            ]
        );
    }

    #[test]
    fn console_clear_leaves_every_group() {
        assert_eq!(
            console("console.group('g'); console.clear(); console.log('flat')"),
            [line(ConsoleLevel::Log, "g"), line(ConsoleLevel::Log, "flat")]
        );
    }

    #[test]
    fn console_timers() {
        let lines = console("console.time('t'); console.timeLog('t', 'half'); console.timeEnd('t')");
        let texts: Vec<&str> = lines.iter().map(|(_, text)| text.as_str()).collect();

        assert_eq!(texts.len(), 2, "{texts:?}");
        assert!(
            texts[0].starts_with("t: ") && texts[0].ends_with("ms half"),
            "{texts:?}"
        );
        assert!(texts[1].starts_with("t: ") && texts[1].ends_with("ms"), "{texts:?}");
    }

    #[test]
    fn console_dir_dirxml_and_table() {
        assert_eq!(console("console.dir('item')"), [line(ConsoleLevel::Log, "item")]);
        assert_eq!(console("console.dirxml('a', 'b')"), [line(ConsoleLevel::Log, "a b")]);
        assert_eq!(console("console.table('rows')"), [line(ConsoleLevel::Log, "rows")]);
        assert_eq!(console("console.dir(); console.table()"), []);
    }

    #[test]
    fn console_state_lasts_across_scripts() {
        let (mut host, lines) = host_and_console();

        assert_eq!(
            host.execute("console.count('n')", &origin()),
            Ok(ScriptValue::Undefined)
        );
        assert_eq!(
            host.execute("console.count('n')", &origin()),
            Ok(ScriptValue::Undefined)
        );
        assert_eq!(
            *lines.lock(),
            [line(ConsoleLevel::Log, "n: 1"), line(ConsoleLevel::Log, "n: 2")]
        );
    }
}
