use crate::types::Result;
use crate::web_executor::js::JSRuntime;
use alloc::rc::Rc;
use std::cell::RefCell;

pub trait JSInterop {
    fn implement<RT: JSRuntime>(s: Rc<RefCell<Self>>, ctx: RT::Context) -> Result<()>;
}
