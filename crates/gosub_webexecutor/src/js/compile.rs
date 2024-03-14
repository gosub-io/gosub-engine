use gosub_shared::types::Result;

use crate::js::JSRuntime;

//compiled code will be stored with this trait for later execution (e.g HTML parsing not done yet)
pub trait JSCompiled {
    type RT: JSRuntime<Compiled = Self>;
    fn run(&mut self) -> Result<<Self::RT as JSRuntime>::Value>;
}
