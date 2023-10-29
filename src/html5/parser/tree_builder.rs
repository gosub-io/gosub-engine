use crate::html5::parser::NodeId;

/// TreeBuilder is an interface to be implemented by a DocumentTaskQueue
/// which is used by the parser to queue up tasks to modify the DOM tree.
///
/// Once tasks are queued up, a call to execute() will commit all changes
/// to the DOM. If there are errors during the application of these changes,
/// the DOM will return a list of the errors encountered but execution is not halted.
///
/// create_element() will generate and return a new NodeId for the parser to keep
/// track of the current context node and optionally store this in a list of open elements.
/// When encountering a closing tag, the parser must pop this ID off of its list.
pub trait TreeBuilder {
    /// Check if the task queue is empty
    fn is_empty(&self) -> bool;
    /// Create a new element node with the given tag name and append it to a parent
    /// with an optional position parameter which places the element at a specific child index.
    /// A pseudo ID is returned that will represent the element's ID if
    /// the changes are committed to the tree via execute() but is not actually
    /// added to the NodeArena until then.
    fn create_element(
        &mut self,
        name: &str,
        parent_id: NodeId,
        position: Option<usize>,
        namespace: &str,
    ) -> NodeId;
    /// Create a new text node with the given content and append it to a parent.
    fn create_text(&mut self, content: &str, parent_id: NodeId);
    /// Create a new comment node with the given content and append it to a parent.
    fn create_comment(&mut self, content: &str, parent_id: NodeId);
    /// Insert/update an attribute for an element node.
    fn insert_attribute(&mut self, key: &str, value: &str, element_id: NodeId);
    /// Commit all queued up changes to the DOM.
    /// Returns a vector of encountered errors, if any.
    fn execute(&mut self) -> Vec<String>;
}
