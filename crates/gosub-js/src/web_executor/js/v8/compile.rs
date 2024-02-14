use crate::web_executor::js::v8::{FromContext, V8Context, V8Ctx, V8Engine, V8Value};
use crate::web_executor::js::{JSCompiled, JSRuntime};
use gosub_shared::types::Result;
use std::rc::Rc;
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
    type RT = V8Engine<'a>;
    fn run(&mut self) -> Result<<Self::RT as JSRuntime>::Value> {
        let try_catch = &mut v8::TryCatch::new(self.context.borrow_mut().scope());

        let Some(value) = self.compiled.run(try_catch) else {
            return Err(V8Ctx::report_exception(try_catch).into()); //catch compile errors
        };

        Ok(V8Value::from_value(Rc::clone(&self.context), value))
    }
}
