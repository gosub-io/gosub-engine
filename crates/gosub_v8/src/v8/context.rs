use v8::{
    CreateParams, Global, HandleScope, Isolate, Local, OwnedIsolate, PinScope, PinnedRef, StackFrame, StackTrace,
    TryCatch,
};

use gosub_shared::types::Result;
use gosub_webexecutor::js::{JSError, WebCompiled, WebContext, WebRuntime};
use gosub_webexecutor::Error;

use crate::v8::control::HeapLimit;
use crate::{FromContext, V8Compiled, V8Context, V8ContextOptions, V8Engine};

pub struct V8Ctx {
    isolate: OwnedIsolate, // Safety: this should NEVER be replaced with a new isolate
    pub ctx: Global<v8::Context>,
    pub(crate) heap: Option<Box<HeapLimit>>,
    /// How many native functions are running, one inside the other.
    pub(crate) in_callback: usize,
    /// A script that a native function ran was ended, and the request that ended it is
    /// still to end what called the function.
    pub(crate) unwinding: bool,
}

impl V8Ctx {
    pub(crate) fn new(params: CreateParams, options: &V8ContextOptions) -> Self {
        let params = match options.heap_limit {
            Some(limit) => params.heap_limits(0, limit),
            None => params,
        };
        let mut isolate = Isolate::new(params);

        if options.explicit_microtasks {
            isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        }
        let heap = options.heap_limit.map(|limit| HeapLimit::install(&mut isolate, limit));

        let ctx = {
            v8::scope!(handle_scope, &mut isolate);

            let ctx = v8::Context::new(handle_scope, Default::default());

            Global::new(handle_scope, ctx)
        };

        Self {
            isolate,
            ctx,
            heap,
            in_callback: 0,
            unwinding: false,
        }
    }

    pub(crate) fn isolate(&mut self) -> &mut OwnedIsolate {
        &mut self.isolate
    }

    pub fn context(&mut self) -> &mut Global<v8::Context> {
        &mut self.ctx
    }

    pub fn report_exception(try_catch: &mut PinnedRef<'_, TryCatch<'_, '_, HandleScope<'_>>>) -> Error {
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

    pub fn handle_stack_trace(ctx: &mut PinScope, stacktrace: Local<StackTrace>) -> String {
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

    fn handle_stack_frame(ctx: &mut PinScope, frame: Local<StackFrame>) -> Option<String> {
        let function = frame.get_function_name(ctx)?.to_rust_string_lossy(ctx);
        let script = frame.get_script_name_or_source_url(ctx)?.to_rust_string_lossy(ctx);
        let line = frame.get_line_number();
        let column = frame.get_column();

        Some(format!("{function}@{script}:{line}: {column}"))
    }
}

impl Drop for V8Ctx {
    fn drop(&mut self) {
        //TODO order is important here: context scope, then handle scope (and ctx), then isolate
    }
}

impl WebContext for V8Context {
    type RT = V8Engine;

    fn run(&mut self, code: &str) -> Result<<Self::RT as WebRuntime>::Value> {
        self.compile(code)?.run()
    }

    fn compile(&mut self, code: &str) -> Result<<Self::RT as WebRuntime>::Compiled> {
        ctx_scope!(s, self);
        v8::tc_scope!(try_catch, s);

        let Some(code) = v8::String::new(try_catch, code) else {
            return Err(anyhow::anyhow!("Failed to allocate V8 string for script source"));
        };

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
        ctx_scope!(scope, self);

        let global = scope.get_current_context().global(scope);

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
