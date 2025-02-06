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
        let compiled = Global::new(&mut ctx.isolate(), value);

        Self { context: ctx, compiled }
    }
}

impl WebCompiled for V8Compiled {
    type RT = V8Engine;
    fn run(&mut self) -> Result<<Self::RT as WebRuntime>::Value> {
        let mut scope = self.context.scope();

        let try_catch = &mut v8::TryCatch::new(&mut scope);

        let compiled = self.compiled.open(try_catch);

        let Some(value) = compiled.run(try_catch) else {
            return Err(V8Ctx::report_exception(try_catch).into()); //catch compile errors
        };

        Ok(V8Value::from_local(V8Context::clone(&self.context), value))
    }
}
