use std::cell::RefCell;
use std::rc::Rc;

use gosub_shared::types::Result;

use crate::js::WebRuntime;

pub trait JSInterop {
    fn implement<RT: WebRuntime>(s: Rc<RefCell<Self>>, ctx: RT::Context) -> Result<()>;
}
