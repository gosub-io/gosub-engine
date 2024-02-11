use crate::web_executor::js::{JSContext, JSRuntime, JSValue};

//compiled code will be stored with this trait for later execution (e.g HTML parsing not done yet)
pub trait JSCompiled {
    type RT: JSRuntime;
    fn run(&mut self) -> crate::types::Result<<Self::RT as JSRuntime>::Value>;
}
