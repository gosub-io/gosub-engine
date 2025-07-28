use v8::{CreateParams, Global, HandleScope, Isolate, Local, OwnedIsolate, StackFrame, StackTrace, TryCatch};

use gosub_shared::types::Result;
use gosub_webexecutor::js::{JSError, WebCompiled, WebContext, WebRuntime};
use gosub_webexecutor::Error;

use crate::{FromContext, V8Compiled, V8Context, V8Engine};

pub struct V8Ctx {
    isolate: OwnedIsolate, // Safety: this should NEVER be replaced with a new isolate
    pub ctx: Global<v8::Context>,

    parent_scope: Option<HandleScope<'static>>, // Safety: this does NOT have an actual 'static lifetime
}

pub struct ScopeGuard<'a> {
    _marker: std::marker::PhantomData<&'a ()>,
    ctx: V8Context,
    prev_scope: Option<HandleScope<'static>>,
}

impl Drop for ScopeGuard<'_> {
    fn drop(&mut self) {
        let mut x = self.ctx.borrow_mut();

        x.parent_scope = self.prev_scope.take();
    }
}

impl V8Ctx {
    pub(crate) fn new(params: CreateParams) -> Self {
        let mut isolate = Isolate::new(params);

        let ctx = {
            let mut handle_scope = HandleScope::new(&mut isolate);

            let ctx = v8::Context::new(&mut handle_scope, Default::default());

            Global::new(&mut handle_scope, ctx)
        };

        Self {
            isolate,
            ctx,
            parent_scope: None,
        }
    }

    pub(crate) fn isolate(&mut self) -> &mut OwnedIsolate {
        &mut self.isolate
    }

    pub fn context(&mut self) -> &mut Global<v8::Context> {
        &mut self.ctx
    }

    pub fn new_scope(&mut self) -> HandleScope<'_> {
        if let Some(parent_scope) = &mut self.parent_scope {
            HandleScope::new(parent_scope)
        } else {
            HandleScope::with_context(&mut self.isolate, &self.ctx)
        }
    }

    pub fn report_exception(try_catch: &mut TryCatch<HandleScope>) -> Error {
        let mut err = String::new();

        if let Some(exception) = try_catch.exception() {
            err = exception.to_rust_string_lossy(try_catch);
        }

        if let Some(m) = try_catch.message() {
            err.push_str("\nMessage: ");
            err.push_str(&m.get(try_catch).to_rust_string_lossy(try_catch));
            if let Some(stacktrace) = m.get_stack_trace(try_catch) {
                let st = Self::handle_stack_trace(try_catch, stacktrace);
                err.push_str(&format!("\nStacktrace:\n{st}"))
            } else {
                err.push_str("\nStacktrace: <missing information>");
            };
        }

        Error::JS(JSError::Exception(err))
    }

    pub fn handle_stack_trace(ctx: &mut HandleScope, stacktrace: Local<StackTrace>) -> String {
        let mut st = String::new();

        for i in 0..stacktrace.get_frame_count() {
            if let Some(frame) = stacktrace.get_frame(ctx, i) {
                if let Some(frame) = Self::handle_stack_frame(ctx, frame) {
                    st.push_str(&frame);
                    st.push('\n');
                }
                continue;
            }
            st.push_str("<missing information>");
        }

        st
    }

    fn handle_stack_frame(ctx: &mut HandleScope, frame: Local<StackFrame>) -> Option<String> {
        let function = frame.get_function_name(ctx)?.to_rust_string_lossy(ctx);
        let script = frame.get_script_name_or_source_url(ctx)?.to_rust_string_lossy(ctx);
        let line = frame.get_line_number();
        let column = frame.get_column();

        Some(format!("{}@{}:{}: {}", function, script, line, column))
    }
}

impl Drop for V8Ctx {
    fn drop(&mut self) {
        //TODO order is important here: context scope, then handle scope (and ctx), then isolate
    }
}

impl V8Context {
    pub fn set_parent_scope<'a>(&self, scope: HandleScope<'a>) -> ScopeGuard<'a> {
        let mut borrowed = self.borrow_mut();

        // Safety: this only extends the lifetime of the scope, which is guarded by the ScopeGuard and removed when the guard is dropped
        let x = borrowed
            .parent_scope
            .replace(unsafe { std::mem::transmute::<HandleScope<'a>, HandleScope<'static>>(scope) });

        ScopeGuard {
            _marker: std::marker::PhantomData,
            ctx: self.clone(),
            prev_scope: x,
        }
    }
}

impl WebContext for V8Context {
    type RT = V8Engine;

    fn run(&mut self, code: &str) -> Result<<Self::RT as WebRuntime>::Value> {
        self.compile(code)?.run()
    }

    fn compile(&mut self, code: &str) -> Result<<Self::RT as WebRuntime>::Compiled> {
        let s = &mut self.scope();

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
        compiled: &mut <Self::RT as WebRuntime>::Compiled,
    ) -> Result<<Self::RT as WebRuntime>::Value> {
        compiled.run()
    }

    fn set_on_global_object(&mut self, name: &str, value: <Self::RT as WebRuntime>::Value) -> Result<()> {
        let mut c = self.borrow_mut();

        let ctx = c.ctx.clone();
        let scope = &mut c.new_scope();

        let iso_ctx = ctx.open(scope);

        let global = iso_ctx.global(scope);

        let name = v8::String::new(scope, name).ok_or(Error::JS(JSError::Conversion(
            "Failed to convert to string".to_string(),
        )))?;
        let value = Local::new(scope, value.value);

        let obj = global.set(scope, name.into(), value);

        if obj.is_none() {
            return Err(Error::JS(JSError::Conversion("Failed to set value".to_string())).into());
        }

        Ok(())
    }
}
