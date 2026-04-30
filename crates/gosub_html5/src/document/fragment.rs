use core::fmt;
use core::fmt::Debug;

use crate::node::arena::NodeArena;
use gosub_shared::node::NodeId;

/// A document fragment (e.g. template element contents or parse fragment result)
#[derive(PartialEq, Clone)]
pub struct DocumentFragmentImpl {
    pub arena: NodeArena,
    pub host: NodeId,
}

impl Debug for DocumentFragmentImpl {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "DocumentFragment")
    }
}

impl DocumentFragmentImpl {
    #[must_use]
    pub fn new(host: NodeId) -> Self {
        Self {
            arena: NodeArena::new(),
            host,
        }
    }
}
