use crate::DocumentHandle;
use core::fmt;
use core::fmt::Debug;

use crate::node::arena::NodeArena;
use gosub_shared::node::NodeId;
use gosub_shared::traits::config::HasDocument;
use gosub_shared::traits::document::DocumentFragment;

/// Defines a document fragment which can be attached to for instance a <template> element
#[derive(PartialEq)]
pub struct DocumentFragmentImpl<C: HasDocument> {
    /// Node elements inside this fragment
    arena: NodeArena<C>,
    /// Document handle of the parent
    pub handle: DocumentHandle<C>,
    /// Host node on which this fragment is attached
    host: NodeId,
}

impl<C: HasDocument> Clone for DocumentFragmentImpl<C> {
    /// Clones the document fragment
    fn clone(&self) -> Self {
        Self {
            arena: self.arena.clone(),
            handle: self.handle.clone(),
            host: self.host,
        }
    }
}

impl<C: HasDocument> Debug for DocumentFragmentImpl<C> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "DocumentFragment")
    }
}

impl<C: HasDocument> DocumentFragmentImpl<C> {
    /// Creates a new document fragment and attaches it to "host" node inside "handle"
    #[must_use]
    pub(crate) fn new(handle: DocumentHandle<C>, host: NodeId) -> Self {
        Self {
            arena: NodeArena::new(),
            handle,
            host,
        }
    }
}

impl<C: HasDocument> DocumentFragment<C> for DocumentFragmentImpl<C> {
    /// Returns the document handle for this document
    fn handle(&self) -> DocumentHandle<C> {
        self.handle.clone()
    }

    fn new(handle: DocumentHandle<C>, node_id: NodeId) -> Self {
        Self::new(handle, node_id)
    }
}
