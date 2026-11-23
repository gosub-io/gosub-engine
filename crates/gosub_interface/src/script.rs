//! Running page scripts somewhere behind an interface.
//!
//! The engine and the HTML parser talk to a script engine through [`ScriptHost`] only.
//! Source text goes in and plain data comes out: no script value, object or callback
//! crosses this interface, so nothing here names a script engine.

use std::fmt;
use std::sync::Arc;

/// Where a script came from, for error messages and stack traces.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScriptOrigin {
    /// The script's own URL, or the document's for an inline script.
    pub url: String,
    /// Line of the document the source starts on (0-based); 0 for an external script.
    pub line: u32,
}

/// What a script evaluated to, as data.
#[derive(Clone, Debug, PartialEq)]
pub enum ScriptValue {
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    /// Anything else, as JSON text. A value JSON has no form for (a function, a symbol) is
    /// [`Undefined`](ScriptValue::Undefined).
    Json(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScriptErrorKind {
    /// The source did not compile, or the script threw.
    Exception,
    /// Stopped through a [`ScriptTerminator`].
    Terminated,
    /// Reached the host's memory limit.
    OutOfMemory,
    /// Nothing ran: scripting is off, or there is no engine to run it.
    Unavailable,
}

/// Why a script produced no value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptError {
    pub kind: ScriptErrorKind,
    pub message: String,
    /// Position in the source, both 1-based; 0 when unknown.
    pub line: u32,
    pub column: u32,
}

impl ScriptError {
    pub fn new(kind: ScriptErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            line: 0,
            column: 0,
        }
    }
}

impl fmt::Display for ScriptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.line == 0 {
            write!(f, "{}", self.message)
        } else {
            write!(f, "{} (line {}, column {})", self.message, self.line, self.column)
        }
    }
}

impl std::error::Error for ScriptError {}

/// The severity of a line a script wrote to its console.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConsoleLevel {
    Debug,
    Log,
    Info,
    Warn,
    Error,
}

/// Stops the script a host is running. Can be used from any thread, which is the point:
/// the thread running the script is busy.
#[derive(Clone)]
pub struct ScriptTerminator(Arc<dyn Fn() + Send + Sync>);

impl ScriptTerminator {
    pub fn new(terminate: impl Fn() + Send + Sync + 'static) -> Self {
        Self(Arc::new(terminate))
    }

    /// For a host that never runs anything.
    pub fn noop() -> Self {
        Self::new(|| {})
    }

    pub fn terminate(&self) {
        (self.0)();
    }
}

impl fmt::Debug for ScriptTerminator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ScriptTerminator")
    }
}

/// A script engine with one global scope: what one document's scripts run in.
pub trait ScriptHost {
    /// Runs a classic script to completion.
    fn execute(&mut self, source: &str, origin: &ScriptOrigin) -> Result<ScriptValue, ScriptError>;

    /// Runs the promise jobs the scripts so far have queued.
    fn microtask_checkpoint(&mut self) -> Result<(), ScriptError>;

    /// A handle that ends the running script with [`ScriptErrorKind::Terminated`].
    fn terminator(&self) -> ScriptTerminator;
}

/// Runs nothing: every script evaluates to undefined. For where scripting is off or no
/// engine is compiled in.
#[derive(Clone, Copy, Debug, Default)]
pub struct NullScriptHost;

impl ScriptHost for NullScriptHost {
    fn execute(&mut self, _source: &str, _origin: &ScriptOrigin) -> Result<ScriptValue, ScriptError> {
        Ok(ScriptValue::Undefined)
    }

    fn microtask_checkpoint(&mut self) -> Result<(), ScriptError> {
        Ok(())
    }

    fn terminator(&self) -> ScriptTerminator {
        ScriptTerminator::noop()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn the_null_host_runs_nothing() {
        // through `dyn`, the way the engine holds a host
        let mut host: Box<dyn ScriptHost> = Box::new(NullScriptHost);

        let value = host.execute("throw new Error('never runs')", &ScriptOrigin::default());
        assert_eq!(value, Ok(ScriptValue::Undefined));
        assert_eq!(host.microtask_checkpoint(), Ok(()));
        host.terminator().terminate();
    }

    #[test]
    fn a_terminator_reaches_its_host_from_another_thread() {
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&calls);
        let terminator = ScriptTerminator::new(move || {
            seen.fetch_add(1, Ordering::SeqCst);
        });

        let copy = terminator.clone();
        let joined = std::thread::spawn(move || copy.terminate()).join();
        terminator.terminate();

        assert!(joined.is_ok());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn an_error_shows_its_position_when_it_has_one() {
        let mut error = ScriptError::new(ScriptErrorKind::Exception, "ReferenceError: x is not defined");
        assert_eq!(error.to_string(), "ReferenceError: x is not defined");

        error.line = 3;
        error.column = 7;
        assert_eq!(error.to_string(), "ReferenceError: x is not defined (line 3, column 7)");
    }
}
