use gosub_shared::types::Result;

use crate::js::WebRuntime;

//compiled code will be stored with this trait for later execution (e.g HTML parsing not done yet)
pub trait WebCompiled {
    type RT: WebRuntime<Compiled = Self>;
    fn run(&mut self) -> Result<<Self::RT as WebRuntime>::Value>;
}
