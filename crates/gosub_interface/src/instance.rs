use crate::config::HasChrome;
use crate::request::RequestServerHandle;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct InstanceId(pub u64);

/// holds handles to the various systems of the engine
#[derive(Clone)]
pub struct Handles<C: HasChrome> {
    pub chrome: C::ChromeHandle,
    pub request: RequestServerHandle,
}
