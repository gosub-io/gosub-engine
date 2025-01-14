use gosub_interface::config::HasDocument;
use gosub_interface::node::Node;
use gosub_shared::node::NodeId;
use std::collections::HashMap;

/// The node arena is the single source for nodes in a document (or fragment).
#[derive(Debug, Clone)]
pub struct NodeArena<C: HasDocument> {
    /// Current nodes stored as <id, node>
    nodes: HashMap<NodeId, C::Node>,
    /// Next node ID to use
    next_id: NodeId,
}

impl<C: HasDocument> NodeArena<C> {
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }
}

impl<C: HasDocument> PartialEq for NodeArena<C> {
    fn eq(&self, other: &Self) -> bool {
        if self.next_id != other.next_id {
            return false;
        }

        self.nodes == other.nodes
    }
}

impl<C: HasDocument> NodeArena<C> {
    /// Creates a new NodeArena
    #[must_use]
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            next_id: NodeId::default(),
        }
    }

    pub(crate) fn get_next_id(&mut self) -> NodeId {
        let node_id = self.next_id;
        self.next_id = node_id.next();

        node_id
    }

    /// Peek what the next node ID is without incrementing the internal counter.
    /// Used by DocumentTaskQueue for create_element() tasks.
    pub(crate) fn peek_next_id(&self) -> NodeId {
        self.next_id
    }

    /// Gets the node with the given id
    pub fn node_ref(&self, node_id: NodeId) -> Option<&C::Node> {
        self.nodes.get(&node_id)
    }

    /// Gets the node with the given id
    pub fn node(&self, node_id: NodeId) -> Option<C::Node> {
        self.nodes.get(&node_id).cloned()
    }

    // /// Get the node with the given id as a mutable reference
    // pub fn node_mut(&mut self, node_id: NodeId) -> Option<&mut N> {
    //     self.nodes.get_mut(&node_id)
    // }

    pub fn delete_node(&mut self, node_id: NodeId) {
        self.nodes.remove(&node_id);
    }

    pub fn update_node(&mut self, node: C::Node) {
        self.nodes.insert(node.id(), node);
    }

    pub fn register_node_with_node_id(&mut self, mut node: C::Node, node_id: NodeId) {
        assert!(!node.is_registered(), "Node is already attached to an arena");

        node.set_id(node_id);
        node.set_registered(true);

        self.nodes.insert(node_id, node);
    }

    /// Registered an unregistered node into the arena
    pub fn register_node(&mut self, mut node: C::Node) -> NodeId {
        assert!(!node.is_registered(), "Node is already attached to an arena");

        let id = self.next_id;
        self.next_id = id.next();

        node.set_id(id);
        node.set_registered(true);

        self.nodes.insert(id, node);
        id
    }

    pub fn nodes(&self) -> &HashMap<NodeId, C::Node> {
        &self.nodes
    }
}

impl<C: HasDocument> Default for NodeArena<C> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::builder::DocumentBuilderImpl;
    use crate::document::document_impl::DocumentImpl;
    use crate::document::fragment::DocumentFragmentImpl;
    use crate::node::HTML_NAMESPACE;
    use gosub_css3::system::Css3System;
    use gosub_interface::config::HasCssSystem;
    use gosub_interface::document::Document;
    use gosub_interface::document::DocumentBuilder;

    use gosub_shared::byte_stream::Location;

    #[derive(Clone, Debug, PartialEq)]
    struct Config;

    impl HasCssSystem for Config {
        type CssSystem = Css3System;
    }
    impl HasDocument for Config {
        type Document = DocumentImpl<Self>;
        type DocumentFragment = DocumentFragmentImpl<Self>;
        type DocumentBuilder = DocumentBuilderImpl;
    }

    #[test]
    fn register_node() {
        let mut doc = <DocumentBuilderImpl as DocumentBuilder<Config>>::new_document(None);

        let node =
            DocumentImpl::<Config>::new_element_node("test", Some(HTML_NAMESPACE), HashMap::new(), Location::default());

        let id = doc.arena.register_node(node);

        assert_eq!(doc.arena.nodes.len(), 2);
        assert_eq!(doc.arena.next_id, 2usize.into());
        assert_eq!(id, NodeId::from(1_usize));
    }

    #[test]
    #[should_panic(expected = "Node is already attached to an arena")]
    fn register_node_twice() {
        let mut doc_handle = <DocumentBuilderImpl as DocumentBuilder<Config>>::new_document(None);

        let node =
            DocumentImpl::<Config>::new_element_node("test", Some(HTML_NAMESPACE), HashMap::new(), Location::default());
        doc_handle.arena.register_node(node);

        let node = doc_handle.node_by_id(NodeId::root()).unwrap().to_owned();
        doc_handle.arena.register_node(node);
    }

    #[test]
    fn get_node() {
        let mut doc = <DocumentBuilderImpl as DocumentBuilder<Config>>::new_document(None);

        let node =
            DocumentImpl::<Config>::new_element_node("test", Some(HTML_NAMESPACE), HashMap::new(), Location::default());

        let id = doc.arena.register_node(node);

        let node = doc.arena.node(id);
        assert!(node.is_some());
        assert_eq!(node.unwrap().get_element_data().unwrap().name, "test");
    }

    // #[test]
    // fn get_node_mut() {
    //     let mut doc_handle = <DocumentBuilderImpl as DocumentBuilder<Config>::new_document(None);
    //
    //     let node = DocumentImpl::<Config>::new_element_node(
    //         doc_handle.clone(),
    //         "test",
    //         Some(HTML_NAMESPACE),
    //         HashMap::new(),
    //         Location::default(),
    //     );
    //
    //     let node_id = doc_handle.get_mut().arena.register_node(node);
    //
    //     let binding = doc_handle.get();
    //     let node = binding.arena.node(node_id);
    //     assert!(node.is_some());
    //     assert_eq!(node.unwrap().get_element_data().unwrap().name, "test");
    // }

    #[test]
    fn register_node_through_document() {
        let mut doc = <DocumentBuilderImpl as DocumentBuilder<Config>>::new_document(None);

        let parent = DocumentImpl::<Config>::new_element_node(
            "parent",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let child = DocumentImpl::<Config>::new_element_node(
            "child",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );

        let parent_id = doc.arena.register_node(parent);
        let child_id = doc.register_node_at(child, parent_id, None);

        let parent = doc.node_by_id(parent_id);
        assert!(parent.is_some());
        assert_eq!(parent.unwrap().children().len(), 1);
        assert_eq!(parent.unwrap().children()[0], child_id);

        let child = doc.node_by_id(child_id);
        assert!(child.is_some());
        assert_eq!(child.unwrap().parent, Some(parent_id));
    }
}
