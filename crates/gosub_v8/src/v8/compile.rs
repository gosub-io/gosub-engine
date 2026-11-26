use v8::{Global, Local, Script};

use gosub_shared::types::Result;

use crate::{FromContext, V8Context, V8Ctx, V8Engine, V8Value};
use gosub_webexecutor::js::{WebCompiled, WebRuntime};

pub struct V8Compiled {
    compiled: Global<Script>,
    context: V8Context,
}

impl FromContext<Local<'_, Script>> for V8Compiled {
    fn from_ctx(ctx: V8Context, value: Local<Script>) -> Self {
        let compiled = Global::new(&ctx.isolate(), value);

        Self { context: ctx, compiled }
    }
}

impl WebCompiled for V8Compiled {
    type RT = V8Engine;
    fn run(&mut self) -> Result<<Self::RT as WebRuntime>::Value> {
        ctx_scope!(scope, self.context);
        v8::tc_scope!(try_catch, scope);

        let compiled = Local::new(try_catch, &self.compiled);

        let Some(value) = compiled.run(try_catch) else {
            if let Some(e) = self.context.take_termination(try_catch.has_terminated()) {
                return Err(e.into());
            }
            return Err(V8Ctx::report_exception(try_catch).into()); //catch compile errors
        };
        // a script that a native function ran was ended, and this one got past it
        if let Some(e) = self.context.take_termination(false) {
            return Err(e.into());
        }

        Ok(V8Value::from_local(V8Context::clone(&self.context), value))
    }
}
