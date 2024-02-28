use std::cell::RefCell;
use std::rc::Rc;

use gosub_shared::types::Result;

use crate::js::JSRuntime;

pub trait JSInterop {
    fn implement<RT: JSRuntime>(s: Rc<RefCell<Self>>, ctx: RT::Context) -> Result<()>;
}
