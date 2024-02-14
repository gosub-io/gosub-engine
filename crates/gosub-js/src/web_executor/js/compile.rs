use crate::web_executor::js::{JSContext, JSRuntime, JSValue};
use gosub_shared::types::Result;

//compiled code will be stored with this trait for later execution (e.g HTML parsing not done yet)
pub trait JSCompiled {
    type RT: JSRuntime;
    fn run(&mut self) -> Result<<Self::RT as JSRuntime>::Value>;
}
