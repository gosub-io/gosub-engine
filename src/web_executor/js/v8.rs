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
mod utils;
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

impl V8Engine<'_> {
    pub fn initialize() {
        if PLATFORM_INITIALIZED.load(Ordering::SeqCst) {
            return;
        }

        if PLATFORM_INITIALIZING.load(Ordering::SeqCst) {
            while !PLATFORM_INITIALIZED.load(Ordering::SeqCst) {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            return;
        }

        PLATFORM_INITIALIZING.store(true, Ordering::SeqCst);

        let platform = v8::new_default_platform(0, false).make_shared();
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
    fn v8_test() {
        //This is needed because the v8 engine is not thread safe - TODO: make it "thread safe"

        println!("running 4 tests in one test function ...");

        v8_engine_initialization();
        println!(
            "test js::v8::tests::v8_engine_initialization ... {}",
            "ok".green()
        );

        v8_context_creation();
        println!(
            "test js::v8::tests::v8_context_creation ... {}",
            "ok".green()
        );

        v8_js_execution();
        println!("test js::v8::tests::v8_js_execution ... {}", "ok".green());

        v8_run_invalid_syntax();
        println!(
            "test js::v8::tests::v8_run_invalid_syntax ... {}",
            "ok".green()
        );
    }

    fn v8_engine_initialization() {
        let mut engine = crate::web_executor::js::v8::V8Engine::new();

        assert!(PLATFORM_INITIALIZED.load(Ordering::SeqCst));
    }

    // #[test]
    // fn v8_bindings_test() {
    //     let platform = v8::new_default_platform(0, false).make_shared();
    //     v8::V8::initialize_platform(platform);
    //     v8::V8::initialize();
    //
    //     let isolate = &mut v8::Isolate::new(Default::default());
    //     let hs = &mut v8::HandleScope::new(isolate);
    //     let c = v8::Context::new(hs);
    //     let s = &mut v8::ContextScope::new(hs, c);
    //
    //     let code = v8::String::new(s, "console.log(\"Hello World!\"); 1234").unwrap();
    //
    //     let value = v8::Script::compile(s, code, None).unwrap().run(s).unwrap();
    //
    //     println!("{}", value.to_rust_string_lossy(s));
    // }

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

    fn v8_context_creation() {
        let mut engine = crate::web_executor::js::v8::V8Engine::new();

        let context = engine.new_context();
        assert!(context.is_ok());
    }
}
