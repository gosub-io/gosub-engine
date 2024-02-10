use alloc::rc::Rc;
use std::any::Any;
use std::cell::RefCell;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicBool, Ordering};

pub use array::*;
pub use compile::*;
pub use context::*;
pub use function::*;
pub use object::*;
pub use value::*;

use crate::types::Result;
use crate::web_executor::js::{
    JSArray, JSContext, JSFunction, JSObject, JSRuntime, JSValue, ValueConversion,
};

mod array;
mod compile;
mod context;
mod function;
mod object;
mod value;

// status of the V8 engine
static PLATFORM_INITIALIZED: AtomicBool = AtomicBool::new(false);
static PLATFORM_INITIALIZING: AtomicBool = AtomicBool::new(false);

trait FromContext<'a, T> {
    fn from_ctx(ctx: V8Context<'a>, value: T) -> Self;
}

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
        if PLATFORM_INITIALIZED.load(Ordering::SeqCst) {
            return;
        }

        let mut wait_time = MAX_V8_INIT_SECONDS * 1000;

        if PLATFORM_INITIALIZING.load(Ordering::SeqCst) {
            while !PLATFORM_INITIALIZED.load(Ordering::SeqCst) {
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

        PLATFORM_INITIALIZING.store(true, Ordering::SeqCst);

        //https://github.com/denoland/rusty_v8/issues/1381
        let platform = v8::new_unprotected_default_platform(0, false).make_shared();
        v8::V8::initialize_platform(platform);
        v8::V8::initialize();

        PLATFORM_INITIALIZED.store(true, Ordering::SeqCst);
        PLATFORM_INITIALIZING.store(false, Ordering::SeqCst);
    }

    pub fn new() -> Self {
        Self::initialize();
        Self {
            _marker: std::marker::PhantomData,
        }
    }
}

//V8 context is stored in a Rc<RefCell<>>, so we can attach it to Values, ...
pub type V8Context<'a> = Rc<RefCell<V8Ctx<'a>>>;

impl<'a> JSRuntime for V8Engine<'a> {
    type Context = V8Context<'a>;

    //let isolate = &mut Isolate::new(Default::default());
    //let hs = &mut HandleScope::new(isolate);
    //let c = Context::new(hs);
    //let s = &mut ContextScope::new(hs, c);

    fn new_context(&mut self) -> Result<Self::Context> {
        V8Ctx::default()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use colored::Colorize;

    use crate::types::Error;
    use crate::web_executor::js::v8::PLATFORM_INITIALIZED;
    use crate::web_executor::js::{JSContext, JSRuntime, JSValue};

    #[test]
    fn v8_engine_initialization() {
        let mut engine = crate::web_executor::js::v8::V8Engine::new();

        assert!(PLATFORM_INITIALIZED.load(Ordering::SeqCst));
    }

    #[test]
    fn v8_js_execution() {
        let mut engine = crate::web_executor::js::v8::V8Engine::new();
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
    fn v8_run_invalid_syntax() {
        let mut engine = crate::web_executor::js::v8::V8Engine::new();

        let mut context = engine.new_context().unwrap();

        let result = context.run(
            r#"
        console.log(Hello World!);
        1234
        "#,
        );

        assert!(result.is_err());

        assert!(matches!(
            result,
            Err(Error::JS(crate::web_executor::js::JSError::Compile(_)))
        ));
    }

    #[test]
    fn v8_context_creation() {
        let mut engine = crate::web_executor::js::v8::V8Engine::new();

        let context = engine.new_context();
        assert!(context.is_ok());
    }
}
