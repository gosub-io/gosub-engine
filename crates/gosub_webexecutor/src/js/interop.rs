use crate::js::JSRuntime;
use gosub_shared::types::Result;
use std::cell::RefCell;
use std::rc::Rc;

pub trait JSInterop {
    fn implement<RT: JSRuntime>(s: Rc<RefCell<Self>>, ctx: RT::Context) -> Result<()>;
}
