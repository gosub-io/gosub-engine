use crate::js::{JSContext, JSValue};

//compiled code will be stored with this trait for later execution (e.g HTML parsing not done yet)
pub trait JSCompiled {
    type Value: JSValue;

    type Context: JSContext;

    fn run(&mut self) -> crate::types::Result<Self::Value>;
}
