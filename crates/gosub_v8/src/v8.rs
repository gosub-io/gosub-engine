use std::cell::RefCell;
use std::fmt::Display;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Once;

use v8::{ContextScope, CreateParams, Exception, HandleScope, Local, Value};

pub use array::*;
pub use compile::*;
pub use context::*;
pub use function::*;
use gosub_shared::types::Result;
use gosub_webexecutor::js::JSRuntime;
pub use object::*;
pub use value::*;

mod array;
mod compile;
mod context;
mod function;
mod object;
mod value;

// status of the V8 engine
static V8_INITIALIZING: AtomicBool = AtomicBool::new(false);
static V8_INITIALIZED: Once = Once::new();

//V8 keeps track of the state internally, so this is just a dummy struct for the wrapper
pub struct V8Engine<'a> {
    _marker: std::marker::PhantomData<&'a ()>,
}

impl Default for V8Engine<'_> {
    fn default() -> Self {
        Self::new()
    }
}

const MAX_V8_INIT_SECONDS: u64 = 10;

impl V8Engine<'_> {
    pub fn initialize() {
        let mut wait_time = MAX_V8_INIT_SECONDS * 1000;

        if V8_INITIALIZING.load(Ordering::SeqCst) {
            while !V8_INITIALIZED.is_completed() {
                std::thread::sleep(std::time::Duration::from_millis(10));
                wait_time -= 10;
                if wait_time <= 9 {
                    panic!(
                        "V8 initialization timed out after {} seconds",
                        MAX_V8_INIT_SECONDS
                    );
                }
            }
            return;
        }

        V8_INITIALIZED.call_once(|| {
            V8_INITIALIZING.store(true, Ordering::SeqCst);
            //https://github.com/denoland/rusty_v8/issues/1381
            let platform = v8::new_unprotected_default_platform(0, false).make_shared();
            v8::V8::initialize_platform(platform);
            v8::V8::initialize();
            V8_INITIALIZING.store(false, Ordering::SeqCst);
        });
    }

    pub fn new() -> Self {
        Self::initialize();
        Self {
            _marker: std::marker::PhantomData,
        }
    }
}

//V8 context is stored in a Rc<RefCell<>>, so we can attach it to Values, ...
pub struct V8Context<'a> {
    ctx: Rc<RefCell<V8Ctx<'a>>>,
}

impl<'a> V8Context<'a> {
    pub fn error(&self, error: impl Display) {
        let scope = self.scope();
        let err = error.to_string();
        let Some(e) = v8::String::new(scope, &err) else {
            eprintln!("failed to create exception string\nexception was: {}", err);
            return;
        };

        let e = Exception::error(scope, e);
        scope.throw_exception(e);
    }

    pub fn create_exception<'b>(
        scope: &mut HandleScope<'b>,
        error: impl Display,
    ) -> Option<Local<'b, Value>> {
        let err = error.to_string();
        let Some(e) = v8::String::new(scope, &err) else {
            eprintln!("failed to create exception string\nexception was: {}", err);
            return None;
        };

        Some(Exception::error(scope, e))
    }
}

impl Clone for V8Context<'_> {
    fn clone(&self) -> Self {
        Self {
            ctx: Rc::clone(&self.ctx),
        }
    }
}

impl<'a> V8Context<'a> {
    pub fn with_default() -> Result<Self> {
        Self::new(Default::default())
    }

    pub fn new(params: CreateParams) -> Result<Self> {
        let ctx = V8Ctx::new(params)?;
        Ok(Self {
            ctx: Rc::new(RefCell::new(ctx)),
        })
    }

    pub fn borrow_mut(&self) -> std::cell::RefMut<'_, V8Ctx<'a>> {
        self.ctx.borrow_mut()
    }

    pub fn borrow(&self) -> std::cell::Ref<'_, V8Ctx<'a>> {
        self.ctx.borrow()
    }

    pub(crate) fn from_ctx(ctx: V8Ctx<'a>) -> Self {
        Self {
            ctx: Rc::new(RefCell::new(ctx)),
        }
    }

    pub(crate) fn scope(&self) -> &'a mut ContextScope<'a, HandleScope<'a>> {
        self.ctx.borrow_mut().scope()
    }
}

impl<'a> JSRuntime for V8Engine<'a> {
    type Context = V8Context<'a>;
    type Value = V8Value<'a>;
    type Object = V8Object<'a>;
    type Compiled = V8Compiled<'a>;
    type GetterCB = GetterCallback<'a>;
    type SetterCB = SetterCallback<'a>;
    type Function = V8Function<'a>;
    type FunctionVariadic = V8FunctionVariadic<'a>;
    type Array = V8Array<'a>;
    type FunctionCallBack = V8FunctionCallBack<'a>;
    type FunctionCallBackVariadic = V8FunctionCallBackVariadic<'a>;
    type Args = V8Args<'a>;
    type VariadicArgs = V8VariadicArgs<'a>;
    type VariadicArgsInternal = V8VariadicArgsInternal<'a>;

    //let isolate = &mut Isolate::new(Default::default());
    //let hs = &mut HandleScope::new(isolate);
    //let c = Context::new(hs);
    //let s = &mut ContextScope::new(hs, c);

    fn new_context(&mut self) -> Result<Self::Context> {
        V8Context::with_default()
    }
}

#[cfg(test)]
mod tests {
    use gosub_webexecutor::js::{JSContext, JSRuntime, JSValue};

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
