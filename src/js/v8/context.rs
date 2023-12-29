use alloc::rc::Rc;
use std::cell::RefCell;
use std::ptr::NonNull;

use v8::{ContextScope, CreateParams, HandleScope, Isolate, OwnedIsolate, TryCatch};

use crate::js::compile::JSCompiled;
use crate::js::v8::compile::V8Compiled;
use crate::js::v8::{FromContext, V8Context, V8Object, V8Value};
use crate::js::{Context, JSContext, JSError};
use crate::types::{Error, Result};

/// SAFETY: This is NOT thread safe, as the rest of the engine is not thread safe.
/// This struct uses `NonNull` internally to store pointers to the V8Context "values" in one struct.
pub struct V8Ctx<'a> {
    isolate: NonNull<OwnedIsolate>,
    handle_scope: NonNull<HandleScope<'a, ()>>,
    context_scope: NonNull<ContextScope<'a, HandleScope<'a>>>,
}

impl<'a> V8Ctx<'a> {
    fn new(params: CreateParams) -> Result<Context<V8Context<'a>>> {
        let mut v8_ctx = Self {
            isolate: NonNull::dangling(),
            handle_scope: NonNull::dangling(),
            context_scope: NonNull::dangling(),
        };

        let isolate = Box::new(Isolate::new(params));

        let Some(isolate) = NonNull::new(Box::into_raw(isolate)) else {
            return Err(Error::JS(JSError::Compile(
                "Failed to create isolate".to_owned(),
            )));
        };
        v8_ctx.isolate = isolate;

        let handle_scope = Box::new(HandleScope::new(unsafe { v8_ctx.isolate.as_mut() }));

        let Some(handle_scope) = NonNull::new(Box::into_raw(handle_scope)) else {
            return Err(Error::JS(JSError::Compile(
                "Failed to create handle scope".to_owned(),
            )));
        };

        v8_ctx.handle_scope = handle_scope;

        let ctx = v8::Context::new(unsafe { v8_ctx.handle_scope.as_mut() });

        let ctx_scope = Box::new(ContextScope::new(
            unsafe { v8_ctx.handle_scope.as_mut() },
            ctx,
        ));

        let Some(ctx_scope) = NonNull::new(Box::into_raw(ctx_scope)) else {
            return Err(Error::JS(JSError::Compile(
                "Failed to create context scope".to_owned(),
            )));
        };

        v8_ctx.context_scope = ctx_scope;

        Ok(Context(Rc::new(RefCell::new(v8_ctx))))
    }

    pub(super) fn scope(&mut self) -> &'a mut ContextScope<'a, HandleScope<'a>> {
        unsafe { self.context_scope.as_mut() }
    }

    pub(super) fn default() -> Result<Context<Rc<RefCell<Self>>>> {
        Self::new(Default::default())
    }

    pub(super) fn report_exception(try_catch: &mut TryCatch<HandleScope>) -> Error {
        if let Some(exception) = try_catch.exception() {
            let e = exception.to_rust_string_lossy(try_catch);

            return Error::JS(JSError::Compile(e));
        }

        if let Some(m) = try_catch.message() {
            let message = m.get(try_catch).to_rust_string_lossy(try_catch);

            return Error::JS(JSError::Compile(message));
        }

        Error::JS(JSError::Compile("unknown error".to_owned()))
    }
}

impl<'a> JSContext for V8Context<'a> {
    type Object = V8Object<'a>;
    type Value = V8Value<'a>;
    type Compiled = V8Compiled<'a>;

    fn run(&mut self, code: &str) -> Result<Self::Value> {
        self.compile(code)?.run()
    }

    fn compile(&mut self, code: &str) -> Result<Self::Compiled> {
        let s = self.borrow_mut().scope();

        let try_catch = &mut TryCatch::new(s);

        let code = v8::String::new(try_catch, code).unwrap();

        let script = v8::Script::compile(try_catch, code, None);

        let Some(script) = script else {
            return Err(V8Ctx::report_exception(try_catch));
        };

        Ok(V8Compiled::from_ctx(Rc::clone(self), script))
    }

    fn run_compiled(&mut self, compiled: &mut Self::Compiled) -> Result<Self::Value> {
        compiled.run()
    }

    fn new_global_object(&mut self, name: &str) -> Result<Self::Object> {
        todo!()
    }
}
