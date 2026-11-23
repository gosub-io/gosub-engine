//! The V8-backed script host. The only place in the engine that names `gosub_v8`.

use gosub_interface::script::{ScriptError, ScriptErrorKind, ScriptHost, ScriptOrigin, ScriptTerminator, ScriptValue};
use gosub_v8::{V8Context, V8ContextOptions, V8Engine, V8Error, V8EvalError, V8Options, V8Plain};

use super::ScriptLimits;

pub(super) struct V8ScriptHost {
    context: V8Context,
}

impl V8ScriptHost {
    pub(super) fn new(limits: ScriptLimits) -> Result<Self, ScriptError> {
        // Interpreter only: no executable memory, which a sandboxed renderer may not map.
        // The first initialization in a process decides, and this is the only one.
        V8Engine::initialize_with(&V8Options {
            jitless: true,
            ..Default::default()
        });

        let context = V8Context::with_options(&V8ContextOptions {
            heap_limit: Some(limits.heap_bytes),
            explicit_microtasks: true,
        })
        .map_err(|e| ScriptError::new(ScriptErrorKind::Unavailable, format!("no V8 context: {e}")))?;

        Ok(Self { context })
    }
}

impl ScriptHost for V8ScriptHost {
    fn execute(&mut self, source: &str, origin: &ScriptOrigin) -> Result<ScriptValue, ScriptError> {
        // a termination request that came too late for the previous script is not for this one
        self.context.cancel_termination();

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
    use std::time::Duration;

    fn host() -> V8ScriptHost {
        V8ScriptHost::new(ScriptLimits {
            heap_bytes: 32 * 1024 * 1024,
        })
        .unwrap()
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
        let stopper = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            terminator.terminate();
        });

        let error = host.execute("for (;;) {}", &origin()).unwrap_err();
        assert_eq!(error.kind, ScriptErrorKind::Terminated);
        stopper.join().unwrap();
        assert_eq!(host.execute("1 + 1", &origin()), Ok(ScriptValue::Number(2.0)));
    }

    #[test]
    fn a_termination_request_before_a_script_does_not_end_it() {
        let mut host = host();

        host.terminator().terminate();
        assert_eq!(host.execute("1 + 1", &origin()), Ok(ScriptValue::Number(2.0)));
    }

    #[test]
    fn the_heap_limit_ends_a_script() {
        let mut host = host();

        let hoard = "(function () { const a = []; for (;;) a.push(new Array(1000).fill(0)); })()";
        let error = host.execute(hoard, &origin()).unwrap_err();
        assert_eq!(error.kind, ScriptErrorKind::OutOfMemory);
        assert_eq!(host.execute("1 + 1", &origin()), Ok(ScriptValue::Number(2.0)));
    }
}
