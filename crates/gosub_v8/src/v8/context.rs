use std::ptr::NonNull;

use v8::{
    CallbackScope, ContextScope, CreateParams, HandleScope, Isolate, Local, Object, OwnedIsolate,
    TryCatch,
};

use gosub_shared::types::Result;

use crate::{FromContext, V8Compiled, V8Context, V8Engine, V8Object};
use gosub_webexecutor::js::{JSCompiled, JSContext, JSError, JSRuntime};
use gosub_webexecutor::Error;

/// SAFETY: This is NOT thread safe, as the rest of the engine is not thread safe.
/// This struct uses `NonNull` internally to store pointers to the V8Context "values" in one struct.
pub struct V8Ctx<'a> {
    pub isolate: NonNull<OwnedIsolate>,
    pub handle_scope: NonNull<HandleScopeType<'a>>,
    pub ctx: NonNull<Local<'a, v8::Context>>,
    pub context_scope: NonNull<ContextScope<'a, HandleScope<'a>>>,
    copied: Copied,
}

struct Copied {
    isolate: bool,
    handle_scope: bool,
    ctx: bool,
    context_scope: bool,
}

impl Copied {
    fn new() -> Self {
        Self {
            isolate: false,
            handle_scope: false,
            ctx: false,
            context_scope: false,
        }
    }
}

pub enum HandleScopeType<'a> {
    WithContextRef(&'a mut HandleScope<'a>),
    WithoutContext(HandleScope<'a, ()>),
    CallbackScope(CallbackScope<'a>),
}

impl<'a> HandleScopeType<'a> {
    fn new(isolate: &'a mut OwnedIsolate) -> Self {
        Self::WithoutContext(HandleScope::new(isolate))
    }

    pub fn get(&mut self) -> &mut HandleScope<'a, ()> {
        match self {
            Self::WithoutContext(scope) => scope,
            Self::CallbackScope(scope) => scope,
            Self::WithContextRef(scope) => scope,
        }
    }
}

impl<'a> V8Ctx<'a> {
    pub(crate) fn new(params: CreateParams) -> Result<Self> {
        let mut v8_ctx = Self {
            isolate: NonNull::dangling(),
            handle_scope: NonNull::dangling(),
            ctx: NonNull::dangling(),
            context_scope: NonNull::dangling(),
            copied: Copied::new(),
        };

        let isolate = Box::new(Isolate::new(params));

        let Some(isolate) = NonNull::new(Box::into_raw(isolate)) else {
            return Err(Error::JS(JSError::Compile("Failed to create isolate".to_owned())).into());
        };
        v8_ctx.isolate = isolate;

        let handle_scope = Box::new(HandleScopeType::new(unsafe { v8_ctx.isolate.as_mut() }));

        let Some(handle_scope) = NonNull::new(Box::into_raw(handle_scope)) else {
            return Err(
                Error::JS(JSError::Compile("Failed to create handle scope".to_owned())).into(),
            );
        };

        v8_ctx.handle_scope = handle_scope;

        let ctx = v8::Context::new(unsafe { v8_ctx.handle_scope.as_mut() }.get());

        let ctx_scope = Box::new(ContextScope::new(
            unsafe { v8_ctx.handle_scope.as_mut() }.get(),
            ctx,
        ));

        let Some(ctx) = NonNull::new(Box::into_raw(Box::new(ctx))) else {
            return Err(Error::JS(JSError::Compile("Failed to create context".to_owned())).into());
        };

        v8_ctx.ctx = ctx;

        let Some(ctx_scope) = NonNull::new(Box::into_raw(ctx_scope)) else {
            return Err(Error::JS(JSError::Compile(
                "Failed to create context scope".to_owned(),
            ))
            .into());
        };

        v8_ctx.context_scope = ctx_scope;

        Ok(v8_ctx)
    }

    pub fn scope(&mut self) -> &'a mut ContextScope<'a, HandleScope<'a>> {
        unsafe { self.context_scope.as_mut() }
    }

    // pub(crate) fn handle_scope(&mut self) -> &'a mut HandleScope<'a, ()> {
    //     unsafe { self.handle_scope.as_mut() }.get()
    // }

    pub fn context(&mut self) -> &'a mut Local<'a, v8::Context> {
        unsafe { self.ctx.as_mut() }
    }

    pub fn report_exception(try_catch: &mut TryCatch<HandleScope>) -> Error {
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

pub fn ctx_from_scope_isolate<'a>(
    scope: HandleScopeType<'a>,
    ctx: Local<'a, v8::Context>,
    isolate: NonNull<OwnedIsolate>,
) -> std::result::Result<V8Context<'a>, (HandleScopeType<'a>, Error)> {
    let mut v8_ctx = V8Ctx {
        isolate,
        handle_scope: NonNull::dangling(),
        ctx: NonNull::dangling(),
        context_scope: NonNull::dangling(),
        copied: Copied {
            isolate: true,
            handle_scope: false,
            ctx: false,
            context_scope: false,
        },
    };

    let ctx = Box::new(ctx);

    let Some(ctx) = NonNull::new(Box::into_raw(ctx)) else {
        return Err((
            scope,
            Error::JS(JSError::Compile("Failed to create context".to_owned())),
        ));
    };

    v8_ctx.ctx = ctx;

    let scope = Box::new(scope);

    let raw_scope = Box::into_raw(scope);
    let Some(scope) = NonNull::new(raw_scope) else {
        //SAFETY: we just created this, so it is safe to convert it back to a Box
        let scope = unsafe { Box::from_raw(raw_scope) };
        return Err((
            *scope,
            Error::JS(JSError::Compile("Failed to create handle scope".to_owned())),
        ));
    };

    v8_ctx.handle_scope = scope;

    let ctx_scope = Box::new(ContextScope::new(
        unsafe { v8_ctx.handle_scope.as_mut() }.get(),
        unsafe { v8_ctx.ctx.as_mut() }.to_owned(),
    ));

    let Some(ctx_scope) = NonNull::new(Box::into_raw(ctx_scope)) else {
        //SAFETY: we just created this, so it is safe to convert it back to a Box
        let scope = unsafe { Box::from_raw(v8_ctx.handle_scope.as_ptr()) };

        return Err((
            *scope,
            Error::JS(JSError::Compile(
                "Failed to create context scope".to_owned(),
            )),
        ));
    };

    v8_ctx.context_scope = ctx_scope;

    Ok(V8Context::from_ctx(v8_ctx))
    // Ok(v8_ctx)
}

pub fn ctx_from_function_callback_info(
    scope: CallbackScope,
    isolate: NonNull<OwnedIsolate>,
) -> std::result::Result<V8Context, (HandleScopeType, Error)> {
    let ctx = scope.get_current_context();
    let scope = HandleScopeType::CallbackScope(scope);

    ctx_from_scope_isolate(scope, ctx, isolate)
}

pub fn ctx_from<'a>(
    scope: &'a mut HandleScope,
    isolate: NonNull<OwnedIsolate>,
) -> std::result::Result<V8Context<'a>, (HandleScopeType<'a>, Error)> {
    let ctx = scope.get_current_context();

    //SAFETY: This can only shorten the lifetime of the scope, which is fine. (we borrow it for 'a and it is '2, which will always be longer than 'a)
    let scope = unsafe { std::mem::transmute(scope) };

    let scope = HandleScopeType::WithContextRef(scope);

    ctx_from_scope_isolate(scope, ctx, isolate)
}

impl Drop for V8Ctx<'_> {
    fn drop(&mut self) {
        // order is important here: context scope, then handle scope (and ctx), then isolate

        if !self.copied.context_scope {
            let _ = unsafe { Box::from_raw(self.context_scope.as_ptr()) };
            self.context_scope = NonNull::dangling(); //use a dangling pointer to prevent double free and segfaults, instead it crashes with a null pointer dereference
        }

        if !self.copied.handle_scope {
            let _ = unsafe { Box::from_raw(self.handle_scope.as_ptr()) };
            self.handle_scope = NonNull::dangling();
        }

        if !self.copied.ctx {
            let _ = unsafe { Box::from_raw(self.ctx.as_ptr()) };
            self.ctx = NonNull::dangling();
        }

        if !self.copied.isolate {
            let _ = unsafe { Box::from_raw(self.isolate.as_ptr()) };
            self.isolate = NonNull::dangling();
        }
    }
}

impl<'a> JSContext for V8Context<'a> {
    type RT = V8Engine<'a>;

    fn run(&mut self, code: &str) -> Result<<Self::RT as JSRuntime>::Value> {
        self.compile(code)?.run()
    }

    fn compile(&mut self, code: &str) -> Result<<Self::RT as JSRuntime>::Compiled> {
        let s = self.scope();

        let try_catch = &mut TryCatch::new(s);

        let code = v8::String::new(try_catch, code).unwrap();

        let script = v8::Script::compile(try_catch, code, None);

        let Some(script) = script else {
            return Err(V8Ctx::report_exception(try_catch).into());
        };

        Ok(V8Compiled::from_ctx(V8Context::clone(self), script))
    }

    fn run_compiled(
        &mut self,
        compiled: &mut <Self::RT as JSRuntime>::Compiled,
    ) -> Result<<Self::RT as JSRuntime>::Value> {
        compiled.run()
    }

    fn new_global_object(&mut self, name: &str) -> Result<<Self::RT as JSRuntime>::Object> {
        let scope = self.scope();
        let obj = Object::new(scope);
        let name = v8::String::new(scope, name).unwrap();

        let global = self.borrow_mut().context().global(scope);

        global.set(scope, name.into(), obj.into());

        Ok(V8Object::from_ctx(V8Context::clone(self), obj))
    }
}
