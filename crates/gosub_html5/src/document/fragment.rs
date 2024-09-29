use crate::DocumentHandle;
use core::fmt;
use core::fmt::Debug;

use crate::document::document_impl::DocumentImpl;
use crate::node::arena::NodeArena;
use crate::node::node_impl::NodeImpl;
use gosub_shared::node::NodeId;
use gosub_shared::traits::css3::CssSystem;
use gosub_shared::traits::document::DocumentFragment;

/// Defines a document fragment which can be attached to for instance a <template> element
#[derive(PartialEq)]
pub struct DocumentFragmentImpl<C: CssSystem> {
    /// Node elements inside this fragment
    arena: NodeArena<NodeImpl<C>, C>,
    /// Document handle of the parent
    pub handle: DocumentHandle<DocumentImpl<C>, C>,
    /// Host node on which this fragment is attached
    host: NodeId,
}

impl<C: CssSystem> Clone for DocumentFragmentImpl<C> {
    /// Clones the document fragment
    fn clone(&self) -> Self {
        Self {
            arena: self.arena.clone(),
            handle: self.handle.clone(),
            host: self.host,
        }
    }
}

impl<C: CssSystem> Debug for DocumentFragmentImpl<C> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "DocumentFragment")
    }
}

impl<C: CssSystem> DocumentFragmentImpl<C> {
    /// Creates a new document fragment and attaches it to "host" node inside "handle"
    #[must_use]
    pub(crate) fn new(handle: DocumentHandle<DocumentImpl<C>, C>, host: NodeId) -> Self {
        Self {
            arena: NodeArena::new(),
            handle,
            host,
        }
    }
}

impl<C: CssSystem> DocumentFragment<C> for DocumentFragmentImpl<C> {
    type Document = DocumentImpl<C>;

    /// Returns the document handle for this document
    fn handle(&self) -> DocumentHandle<Self::Document, C> {
        self.handle.clone()
    }

    fn new(handle: DocumentHandle<Self::Document, C>, node_id: NodeId) -> Self {
        Self::new(handle, node_id)
    }
}
