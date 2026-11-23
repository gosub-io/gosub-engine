use std::cell::RefCell;
use std::fmt::Display;
use std::ops::{Deref, DerefMut};
use std::ptr::NonNull;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Once;

use v8::{CreateParams, Exception, Global, Isolate, Local, PinScope, Value};

pub use array::*;
pub use compile::*;
pub use context::*;
pub use control::*;
pub use evaluate::*;
pub use function::*;
use gosub_shared::types::Result;
use gosub_webexecutor::js::WebRuntime;
pub use object::*;
pub use value::*;

mod array;
mod compile;
mod context;
mod control;
mod evaluate;
mod function;
mod object;
mod value;

// status of the V8 engine
static V8_INITIALIZING: AtomicBool = AtomicBool::new(false);
static V8_INITIALIZED: Once = Once::new();

//V8 keeps track of the state internally, so this is just a dummy struct for the wrapper
pub struct V8Engine;

impl Default for V8Engine {
    fn default() -> Self {
        Self::new()
    }
}

const MAX_V8_INIT_SECONDS: u64 = 10;

impl V8Engine {
    pub fn initialize() {
        Self::initialize_with(&V8Options::default());
    }

    /// Returns false when V8 was already initialized, in which case `options` had no effect.
    #[allow(clippy::panic)] // deliberate fail-fast: a hung V8 initialization cannot be recovered from
    pub fn initialize_with(options: &V8Options) -> bool {
        let mut wait_time = MAX_V8_INIT_SECONDS * 1000;

        if V8_INITIALIZING.load(Ordering::SeqCst) {
            while !V8_INITIALIZED.is_completed() {
                std::thread::sleep(std::time::Duration::from_millis(10));
                wait_time -= 10;
                if wait_time <= 9 {
                    panic!("V8 initialization timed out after {MAX_V8_INIT_SECONDS} seconds");
                }
            }
            return false;
        }

        let mut applied = false;

        V8_INITIALIZED.call_once(|| {
            V8_INITIALIZING.store(true, Ordering::SeqCst);

            let mut flags = options.flags.clone();
            if options.jitless {
                flags.push("--jitless".to_owned());
            }
            if options.single_threaded {
                flags.push("--single-threaded".to_owned());
            }
            if !flags.is_empty() {
                v8::V8::set_flags_from_string(&flags.join(" "));
            }

            // V8 otherwise opens /dev/urandom for every isolate and context
            v8::V8::set_entropy_source(|buf| getrandom::fill(buf).is_ok());

            let platform = if options.single_threaded {
                v8::new_single_threaded_default_platform(false)
            } else {
                //https://github.com/denoland/rusty_v8/issues/1381
                v8::new_unprotected_default_platform(0, false)
            }
            .make_shared();
            v8::V8::initialize_platform(platform);
            v8::V8::initialize();
            V8_INITIALIZING.store(false, Ordering::SeqCst);
            applied = true;
        });

        applied
    }

    pub fn new() -> Self {
        Self::initialize();
        Self
    }
}

//V8 context is stored in a Rc<RefCell<>>, so we can attach it to Values, ...
pub struct V8Context {
    ctx: Rc<RefCell<V8Ctx>>,
}

/// Just a thin wrapper around the V8 Isolate. It has a strong reference to the V8Context to keep the Isolate alive
///
/// The isolate is the thread's current one for as long as this lives: a thread can hold
/// the contexts of several pages, and V8 works on the isolate that was entered last.
/// Keep it in a local, like a scope: wrappers have to go in the reverse order they came.
/// Nothing checks that order, which is why code outside this crate gets no wrapper.
pub(crate) struct IsolateWrapper {
    iso: NonNull<Isolate>,    // we are not responsible for dropping the Isolate
    _ctx: Rc<RefCell<V8Ctx>>, //ctx to keep the isolate alive
}

impl Deref for IsolateWrapper {
    type Target = Isolate;

    fn deref(&self) -> &Self::Target {
        unsafe { self.iso.as_ref() }
    }
}

impl DerefMut for IsolateWrapper {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { self.iso.as_mut() }
    }
}

impl Drop for IsolateWrapper {
    fn drop(&mut self) {
        // Safety: entered in `V8Context::isolate`, and wrappers go in the reverse order
        // they came in, so this isolate is the current one
        unsafe { self.iso.as_ref().exit() }
    }
}

impl V8Context {
    pub(crate) fn isolate(&self) -> IsolateWrapper {
        let mut this = self.ctx.borrow_mut();
        let iso = NonNull::from(this.isolate().as_mut());
        // Safety: left again when the wrapper is dropped
        unsafe { iso.as_ref().enter() }

        IsolateWrapper {
            iso,
            _ctx: Rc::clone(&self.ctx),
        }
    }

    pub fn error(&self, error: impl Display) {
        ctx_scope!(scope, self);
        let err = error.to_string();
        let Some(e) = v8::String::new(scope, &err) else {
            log::error!("failed to create exception string\nexception was: {err}");
            return;
        };

        let e = Exception::error(scope, e);
        scope.throw_exception(e);
    }

    pub fn create_exception<'b>(scope: &mut PinScope<'b, '_>, error: impl Display) -> Option<Local<'b, Value>> {
        let err = error.to_string();
        let Some(e) = v8::String::new(scope, &err) else {
            log::error!("failed to create exception string\nexception was: {err}");
            return None;
        };

        Some(Exception::error(scope, e))
    }

    /// The isolate and context a scope is opened on, see `ctx_scope!`.
    pub(crate) fn scope_parts(&self) -> (IsolateWrapper, Global<v8::Context>) {
        let context = self.ctx.borrow().ctx.clone();

        (self.isolate(), context)
    }
}

impl Clone for V8Context {
    fn clone(&self) -> Self {
        Self {
            ctx: Rc::clone(&self.ctx),
        }
    }
}

impl V8Context {
    pub fn with_default() -> Result<Self> {
        Self::new(Default::default())
    }

    pub fn new(params: CreateParams) -> Result<Self> {
        Self::new_with(params, &V8ContextOptions::default())
    }

    pub fn with_options(options: &V8ContextOptions) -> Result<Self> {
        Self::new_with(Default::default(), options)
    }

    fn new_with(params: CreateParams, options: &V8ContextOptions) -> Result<Self> {
        let ctx = V8Ctx::new(params, options);
        Ok(Self {
            ctx: Rc::new(RefCell::new(ctx)),
        })
    }

    pub fn borrow_mut(&self) -> std::cell::RefMut<'_, V8Ctx> {
        self.ctx.borrow_mut()
    }

    pub fn borrow(&self) -> std::cell::Ref<'_, V8Ctx> {
        self.ctx.borrow()
    }
}

impl WebRuntime for V8Engine {
    type Context = V8Context;
    type Value = V8Value;
    type Object = V8Object;
    type Compiled = V8Compiled;
    type GetterCB = GetterCallback;
    type SetterCB = SetterCallback;
    type Function = V8Function;
    type FunctionVariadic = V8FunctionVariadic;
    type Array = V8Array;
    type FunctionCallBack = V8FunctionCallBack;
    type FunctionCallBackVariadic = V8FunctionCallBackVariadic;
    type Args = V8Args;
    type VariadicArgs = V8VariadicArgs;
    type VariadicArgsInternal = V8VariadicArgsInternal;

    // let isolate = &mut Isolate::new(Default::default());
    // let hs = &mut HandleScope::new(isolate);
    // let c = Context::new(hs);
    // let s = &mut ContextScope::new(hs, c);

    fn new_context(&mut self) -> Result<Self::Context> {
        V8Context::with_default()
    }
}

#[cfg(test)]
mod tests {
    use gosub_webexecutor::js::{WebContext, WebRuntime, WebValue};

    use crate::v8::V8_INITIALIZED;

    #[test]
    fn v8_engine_initialization() {
        let _engine = crate::v8::V8Engine::new();

        assert!(V8_INITIALIZED.is_completed());
    }

    #[test]
    fn v8_js_execution() {
        let mut engine = crate::v8::V8Engine::new();
        let mut context = engine.new_context().unwrap();

        let value = context
            .run(
                r#"
            console.log("Hello World!");
            1234
        "#,
            )
            .unwrap();

        assert!(value.is_number());
        assert_eq!(value.as_number().unwrap(), 1234.0);
    }

    #[test]
    #[should_panic = "called `Result::unwrap()` on an `Err` value: js: exception: SyntaxError: missing ) after argument list\nMessage: Uncaught SyntaxError: missing ) after argument list\nStacktrace: <missing information>\n\nCaused by:\n    exception: SyntaxError: missing ) after argument list\n    Message: Uncaught SyntaxError: missing ) after argument list\n    Stacktrace: <missing information>"]
    fn v8_run_invalid_syntax() {
        let mut engine = crate::v8::V8Engine::new();

        let mut context = engine.new_context().unwrap();

        let result = context.run(
            r#"
        console.log(Hello World!);
        1234
        "#,
        );

        assert!(result.is_err());
        result.unwrap();
    }

    #[test]
    fn v8_context_creation() {
        let mut engine = crate::v8::V8Engine::new();

        let context = engine.new_context();
        assert!(context.is_ok());
    }
}
