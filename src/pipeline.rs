use crate::html5::node::NodeId;
use crate::html5::parser::document::DocumentHandle;
use crate::styles::StyleCalculator;

/// The rendering pipeline to convert a document and stylesheets into a rendered page
pub struct Pipeline {}

impl Default for Pipeline {
    fn default() -> Self {
        Self::new()
    }
}

impl Pipeline {
    pub fn new() -> Self {
        Self {}
    }

    /// Generates a render tree by duplicating the DOM tree and removing all nodes that are not renderable or hidden
    pub fn generate_render_tree(
        &self,
        doc_handle: DocumentHandle,
        _calculator: &StyleCalculator,
    ) -> DocumentHandle {
        // Create a complete copy of the document tree into the render tree
        let mut rendertree_handle = doc_handle.deep_clone();

        // Iterate tree and remove elements that are hidden or not renderable
        remove_unrenderable_nodes(&mut rendertree_handle, NodeId::root(), _calculator);

        DocumentHandle::clone(&rendertree_handle)
    }
}

fn remove_unrenderable_nodes(
    rendertree_handle: &mut DocumentHandle,
    node_id: NodeId,
    _calculator: &StyleCalculator,
) {
    let node;
    {
        let binding = rendertree_handle.get();
        node = binding.get_node_by_id(node_id).unwrap().clone();
        // println!("Visiting node: {:?}", node);
    }

    let removable_elements = ["head", "script", "style", "svg"];

    if node.is_element() && removable_elements.contains(&node.as_element().name.as_str()) {
        rendertree_handle.get_mut().delete_node(&node);
        return;
    }

    // Check CSS styles and remove if not renderable

    // Iterate all children from this node
    for &child_id in &node.children {
        remove_unrenderable_nodes(rendertree_handle, child_id, _calculator);
    }
}
