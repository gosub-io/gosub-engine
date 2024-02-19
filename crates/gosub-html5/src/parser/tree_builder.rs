use crate::parser::NodeId;
use gosub_shared::types::Result;

/// TreeBuilder is an interface to abstract DOM tree modifications.
///
/// This is implemented by DocumentHandle to support direct immediate manipulation of the DOM
/// and implemented by DocumentTaskQueue to support queueing up several mutations to be performed at once.
pub trait TreeBuilder {
    /// Create a new element node with the given tag name and append it to a parent
    /// with an optional position parameter which places the element at a specific child index.
    fn create_element(
        &mut self,
        name: &str,
        parent_id: NodeId,
        position: Option<usize>,
        namespace: &str,
    ) -> NodeId;

    /// Create a new text node with the given content and append it to a parent.
    fn create_text(&mut self, content: &str, parent_id: NodeId) -> NodeId;

    /// Create a new comment node with the given content and append it to a parent.
    fn create_comment(&mut self, content: &str, parent_id: NodeId) -> NodeId;

    /// Insert/update an attribute for an element node.
    fn insert_attribute(&mut self, key: &str, value: &str, element_id: NodeId) -> Result<()>;
}
