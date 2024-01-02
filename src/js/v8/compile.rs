use crate::js::v8::{FromContext, V8Context, V8Ctx, V8Value};
use crate::js::{Context, JSCompiled};
use alloc::rc::Rc;
use v8::{Local, Script};
pub struct V8Compiled<'a> {
    compiled: Local<'a, Script>,
    context: V8Context<'a>,
}

impl<'a> FromContext<'a, Local<'a, Script>> for V8Compiled<'a> {
    fn from_ctx(ctx: V8Context<'a>, value: Local<'a, Script>) -> Self {
        Self {
            context: ctx,
            compiled: value,
        }
    }
}

impl<'a> JSCompiled for V8Compiled<'a> {
    type Value = V8Value<'a>;

    type Context = Context<V8Context<'a>>;

    fn run(&mut self) -> crate::types::Result<Self::Value> {
        let try_catch = &mut v8::TryCatch::new(self.context.borrow_mut().scope());

        let Some(value) = self.compiled.run(try_catch) else {
            return Err(V8Ctx::report_exception(try_catch)); //catch compile errors
        };

        Ok(V8Value::from_value(Rc::clone(&self.context), value))
    }
}
