use std::fmt;

use v8::{Local, Value};

use crate::{V8Context, V8Error};

/// What a script evaluated to, copied out of V8.
#[derive(Clone, Debug, PartialEq)]
pub enum V8Plain {
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    /// An object or array, as JSON text.
    Json(String),
}

/// A compile error or an uncaught exception.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct V8Exception {
    /// The exception as a string, e.g. `ReferenceError: x is not defined`.
    pub message: String,
    /// Position in the script, both 1-based; 0 when V8 has none. The line includes the
    /// offset [`V8Context::evaluate`] was given.
    pub line: u32,
    pub column: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum V8EvalError {
    Exception(V8Exception),
    /// Terminated, or out of memory.
    Ended(V8Error),
}

impl fmt::Display for V8EvalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exception(e) => f.write_str(&e.message),
            Self::Ended(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for V8EvalError {}

impl V8Context {
    /// Runs a classic script and returns its completion value as data.
    ///
    /// `url` names the script in stack traces; `line_offset` is the 0-based line of the
    /// document the source starts on.
    pub fn evaluate(&self, code: &str, url: &str, line_offset: u32) -> Result<V8Plain, V8EvalError> {
        let result = {
            ctx_scope!(scope, self);
            v8::tc_scope!(try_catch, scope);

            match run(try_catch, code, url, line_offset) {
                Some(value) => Ok(value),
                None if try_catch.has_terminated() => Err(None),
                None => {
                    let thrown = exception(try_catch);
                    // the thrown value's own `toString()` ran for that, and can be ended
                    // like any script
                    if try_catch.has_terminated() {
                        Err(None)
                    } else {
                        Err(Some(thrown))
                    }
                }
            }
        };

        match result {
            Err(None) => Err(V8EvalError::Ended(
                self.take_termination(true).unwrap_or(V8Error::Terminated),
            )),
            // A script that a native function ran was ended, and this one got past it:
            // with a value, or with what the function threw instead. It was ended as well.
            result => match (self.take_termination(false), result) {
                (Some(by), _) => Err(V8EvalError::Ended(by)),
                (None, Ok(value)) => Ok(value),
                (None, Err(Some(e))) => Err(V8EvalError::Exception(e)),
                (None, Err(None)) => Err(V8EvalError::Ended(V8Error::Terminated)),
            },
        }
    }
}

/// None when V8 has an exception pending or the script was terminated.
fn run(scope: &mut v8::PinScope<'_, '_>, code: &str, url: &str, line_offset: u32) -> Option<V8Plain> {
    let code = v8::String::new(scope, code)?;
    let name = v8::String::new(scope, url)?;
    let line_offset = i32::try_from(line_offset).unwrap_or(i32::MAX);
    let origin = v8::ScriptOrigin::new(
        scope,
        name.into(),
        line_offset,
        0,
        false,
        0,
        None,
        false,
        false,
        false,
        None,
    );

    let value = v8::Script::compile(scope, code, Some(&origin))?.run(scope)?;

    plain(scope, value)
}

fn plain(scope: &mut v8::PinScope<'_, '_>, value: Local<Value>) -> Option<V8Plain> {
    Some(if value.is_undefined() || value.is_function() || value.is_symbol() {
        V8Plain::Undefined
    } else if value.is_null() {
        V8Plain::Null
    } else if value.is_boolean() {
        V8Plain::Bool(value.is_true())
    } else if value.is_number() {
        V8Plain::Number(value.number_value(scope)?)
    } else if value.is_string() {
        V8Plain::String(value.to_rust_string_lossy(scope))
    } else {
        // throws for what JSON cannot hold: a BigInt, a cycle
        let json = v8::json::stringify(scope, value)?;
        V8Plain::Json(json.to_rust_string_lossy(scope))
    })
}

fn exception(try_catch: &mut v8::PinnedRef<'_, v8::TryCatch<'_, '_, v8::HandleScope<'_>>>) -> V8Exception {
    let message = match try_catch.exception() {
        Some(exception) => exception.to_rust_string_lossy(try_catch),
        // no exception and no termination: a string V8 could not allocate
        None => "script could not be run".to_owned(),
    };

    // an exception thrown outside any script (JSON.stringify on the result) has no position
    let line = try_catch
        .message()
        .and_then(|m| m.get_line_number(try_catch))
        .and_then(|line| u32::try_from(line).ok())
        .unwrap_or(0);
    let column = try_catch
        .message()
        .filter(|_| line > 0)
        // V8 counts columns from 0
        .and_then(|m| u32::try_from(m.get_start_column()).ok())
        .map_or(0, |column| column.saturating_add(1));

    V8Exception { message, line, column }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::{V8Context, V8ContextOptions, V8Engine, V8Error, V8EvalError, V8Plain};

    fn context() -> V8Context {
        V8Engine::initialize();
        V8Context::with_options(&V8ContextOptions::default()).unwrap()
    }

    fn eval(code: &str) -> Result<V8Plain, V8EvalError> {
        context().evaluate(code, "test.js", 0)
    }

    #[test]
    fn values_come_out_as_data() {
        assert_eq!(eval("1 + 1"), Ok(V8Plain::Number(2.0)));
        assert_eq!(eval("'a' + 'b'"), Ok(V8Plain::String("ab".to_owned())));
        assert_eq!(eval("1 < 2"), Ok(V8Plain::Bool(true)));
        assert_eq!(eval("null"), Ok(V8Plain::Null));
        assert_eq!(eval("undefined"), Ok(V8Plain::Undefined));
        assert_eq!(eval("var x = 1"), Ok(V8Plain::Undefined));
        assert_eq!(
            eval("0 / 0").map(|v| matches!(v, V8Plain::Number(n) if n.is_nan())),
            Ok(true)
        );
        assert_eq!(
            eval("({a: [1, 'two', null]})"),
            Ok(V8Plain::Json(r#"{"a":[1,"two",null]}"#.to_owned()))
        );
        assert_eq!(eval("[1, 2]"), Ok(V8Plain::Json("[1,2]".to_owned())));
    }

    #[test]
    fn values_json_has_no_form_for_are_undefined() {
        assert_eq!(eval("(function () {})"), Ok(V8Plain::Undefined));
        assert_eq!(eval("Symbol('s')"), Ok(V8Plain::Undefined));
    }

    #[test]
    fn a_value_json_rejects_is_an_exception() {
        for code in ["var a = {}; a.a = a; a", "({n: 1n})"] {
            let Err(V8EvalError::Exception(e)) = eval(code) else {
                panic!("{code} did not throw");
            };
            assert!(e.message.starts_with("TypeError"), "{}", e.message);
        }
    }

    #[test]
    fn an_exception_has_its_position() {
        let Err(V8EvalError::Exception(e)) = eval("var a = 1;\n  missing()") else {
            panic!("did not throw");
        };
        assert_eq!(e.message, "ReferenceError: missing is not defined");
        assert_eq!((e.line, e.column), (2, 3));
    }

    #[test]
    fn a_syntax_error_has_its_position() {
        let Err(V8EvalError::Exception(e)) = eval("var = 1") else {
            panic!("did not throw");
        };
        assert!(e.message.starts_with("SyntaxError"), "{}", e.message);
        assert_eq!((e.line, e.column), (1, 5));
    }

    #[test]
    fn the_line_offset_moves_the_reported_line() {
        let Err(V8EvalError::Exception(e)) = context().evaluate("\nmissing()", "page.html", 10) else {
            panic!("did not throw");
        };
        assert_eq!(e.line, 12);
    }

    #[test]
    fn a_thrown_value_is_the_message() {
        let Err(V8EvalError::Exception(e)) = eval("throw 'plain'") else {
            panic!("did not throw");
        };
        assert_eq!(e.message, "plain");
    }

    #[test]
    fn the_url_shows_in_a_stack_trace() {
        let stack = context().evaluate("new Error('x').stack", "https://example.org/a.js", 0);
        let Ok(V8Plain::String(stack)) = stack else {
            panic!("no stack: {stack:?}");
        };
        assert!(stack.contains("https://example.org/a.js:1"), "{stack}");
    }

    #[test]
    fn globals_stay_between_scripts() {
        let ctx = context();
        assert_eq!(ctx.evaluate("var kept = 41", "a.js", 0), Ok(V8Plain::Undefined));
        assert_eq!(ctx.evaluate("kept + 1", "b.js", 0), Ok(V8Plain::Number(42.0)));
    }

    #[test]
    fn a_script_ended_while_its_exception_is_read_was_terminated() {
        let ctx = context();
        let terminator = ctx.terminator();
        let stopper = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            terminator.terminate();
        });

        assert_eq!(
            ctx.evaluate("throw { toString() { for (;;) {} } }", "test.js", 0),
            Err(V8EvalError::Ended(V8Error::Terminated))
        );
        stopper.join().unwrap();
        assert_eq!(ctx.evaluate("1 + 1", "test.js", 0), Ok(V8Plain::Number(2.0)));
    }

    #[test]
    fn a_terminated_script_ends_and_the_context_runs_again() {
        let ctx = context();
        let terminator = ctx.terminator();
        let stopper = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            terminator.terminate();
        });

        assert_eq!(
            ctx.evaluate("for (;;) {}", "a.js", 0),
            Err(V8EvalError::Ended(V8Error::Terminated))
        );
        stopper.join().unwrap();
        assert_eq!(ctx.evaluate("1 + 1", "a.js", 0), Ok(V8Plain::Number(2.0)));
    }

    #[test]
    fn the_heap_limit_ends_the_script() {
        V8Engine::initialize();
        let ctx = V8Context::with_options(&V8ContextOptions {
            heap_limit: Some(32 * 1024 * 1024),
            ..Default::default()
        })
        .unwrap();

        let hoard = "(function () { const a = []; for (;;) a.push({ x: 1 }); })()";
        assert_eq!(
            ctx.evaluate(hoard, "a.js", 0),
            Err(V8EvalError::Ended(V8Error::OutOfMemory))
        );
        assert_eq!(ctx.evaluate("1 + 1", "a.js", 0), Ok(V8Plain::Number(2.0)));
    }
}
