use gosub_shared::node::NodeId;
use gosub_shared::traits::css3::CssSystem;
use gosub_shared::traits::node::Node;
use std::collections::HashMap;
use std::marker::PhantomData;

/// The node arena is the single source for nodes in a document (or fragment).
#[derive(Debug, Clone)]
pub struct NodeArena<N: Node<C>, C: CssSystem> {
    /// Current nodes stored as <id, node>
    nodes: HashMap<NodeId, N>,
    /// Next node ID to use
    next_id: NodeId,

    _marker: PhantomData<C>,
}

impl<C: CssSystem, N: Node<C>> NodeArena<N, C> {
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }
}

impl<C: CssSystem, N: Node<C>> PartialEq for NodeArena<N, C> {
    fn eq(&self, other: &Self) -> bool {
        if self.next_id != other.next_id {
            return false;
        }

        self.nodes == other.nodes
    }
}

impl<N: Node<C>, C: CssSystem> NodeArena<N, C> {
    /// Creates a new NodeArena
    #[must_use]
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            next_id: NodeId::default(),
            _marker: PhantomData,
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
    pub fn node_ref(&self, node_id: NodeId) -> Option<&N> {
        self.nodes.get(&node_id)
    }

    /// Gets the node with the given id
    pub fn node(&self, node_id: NodeId) -> Option<N> {
        self.nodes.get(&node_id).cloned()
    }

    // /// Get the node with the given id as a mutable reference
    // pub fn node_mut(&mut self, node_id: NodeId) -> Option<&mut N> {
    //     self.nodes.get_mut(&node_id)
    // }

    pub fn delete_node(&mut self, node_id: NodeId) {
        self.nodes.remove(&node_id);
    }

    pub fn update_node(&mut self, node: N) {
        self.nodes.insert(node.id(), node);
    }

    pub fn register_node_with_node_id(&mut self, mut node: N, node_id: NodeId) {
        assert!(!node.is_registered(), "Node is already attached to an arena");

        node.set_id(node_id);
        node.set_registered(true);

        self.nodes.insert(node_id, node);
    }

    /// Registered an unregistered node into the arena
    pub fn register_node(&mut self, mut node: N) -> NodeId {
        assert!(!node.is_registered(), "Node is already attached to an arena");

        let id = self.next_id;
        self.next_id = id.next();

        node.set_id(id);
        node.set_registered(true);

        self.nodes.insert(id, node);
        id
    }

    pub fn nodes(&self) -> &HashMap<NodeId, N> {
        &self.nodes
    }
}

impl<N: Node<C>, C: CssSystem> Default for NodeArena<N, C> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::document_impl::DocumentImpl;
    use gosub_css3::system::Css3System;
    use gosub_shared::byte_stream::Location;
    use gosub_shared::traits::document::Document;

    use crate::document::builder::DocumentBuilderImpl;
    use gosub_shared::traits::document::DocumentBuilder;

    use crate::node::HTML_NAMESPACE;

    #[test]
    fn register_node() {
        let mut doc_handle = DocumentBuilderImpl::new_document(None);

        let node = DocumentImpl::<Css3System>::new_element_node(
            doc_handle.clone(),
            "test",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );

        let id = doc_handle.get_mut().arena.register_node(node);

        let binding = doc_handle.get();
        assert_eq!(binding.arena.nodes.len(), 2);
        assert_eq!(binding.arena.next_id, 2usize.into());
        assert_eq!(id, NodeId::from(1_usize));
    }

    #[test]
    #[should_panic(expected = "Node is already attached to an arena")]
    fn register_node_twice() {
        let mut doc_handle = DocumentBuilderImpl::new_document(None);

        let node = DocumentImpl::<Css3System>::new_element_node(
            doc_handle.clone(),
            "test",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        doc_handle.get_mut().arena.register_node(node);

        let node = doc_handle.get_mut().node_by_id(NodeId::root()).unwrap().to_owned();
        doc_handle.get_mut().arena.register_node(node);
    }

    #[test]
    fn get_node() {
        let mut doc_handle = DocumentBuilderImpl::new_document(None);

        let node = DocumentImpl::<Css3System>::new_element_node(
            doc_handle.clone(),
            "test",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );

        let id = doc_handle.get_mut().arena.register_node(node);

        let binding = doc_handle.get();
        let node = binding.arena.node(id);
        assert!(node.is_some());
        assert_eq!(node.unwrap().get_element_data().unwrap().name, "test");
    }

    // #[test]
    // fn get_node_mut() {
    //     let mut doc_handle = DocumentBuilderImpl::new_document(None);
    //
    //     let node = DocumentImpl::<Css3System>::new_element_node(
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
        let mut doc_handle = DocumentBuilderImpl::new_document(None);

        let parent = DocumentImpl::<Css3System>::new_element_node(
            doc_handle.clone(),
            "parent",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let child = DocumentImpl::<Css3System>::new_element_node(
            doc_handle.clone(),
            "child",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );

        let parent_id = doc_handle.get_mut().arena.register_node(parent);
        let child_id = doc_handle.get_mut().register_node_at(child, parent_id, None);

        let binding = doc_handle.get();
        let parent = binding.node_by_id(parent_id);
        assert!(parent.is_some());
        assert_eq!(parent.unwrap().children().len(), 1);
        assert_eq!(parent.unwrap().children()[0], child_id);

        let child = binding.node_by_id(child_id);
        assert!(child.is_some());
        assert_eq!(child.unwrap().parent, Some(parent_id));
    }
}
