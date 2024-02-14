use gosub_shared::types::Result;
use crate::web_executor::js::JSRuntime;
use std::rc::Rc;
use std::cell::RefCell;

pub trait JSInterop {
    fn implement<RT: JSRuntime>(s: Rc<RefCell<Self>>, ctx: RT::Context) -> Result<()>;
}
