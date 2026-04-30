use crate::node::node_impl::NodeImpl;
use gosub_shared::node::NodeId;
use std::collections::HashMap;

/// The node arena is the single source for nodes in a document (or fragment).
#[derive(Debug, Clone)]
pub struct NodeArena {
    nodes: HashMap<NodeId, NodeImpl>,
    next_id: NodeId,
}

impl PartialEq for NodeArena {
    fn eq(&self, other: &Self) -> bool {
        self.next_id == other.next_id && self.nodes == other.nodes
    }
}

impl NodeArena {
    #[must_use]
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            next_id: NodeId::default(),
        }
    }

    #[must_use]
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub(crate) fn get_next_id(&mut self) -> NodeId {
        let node_id = self.next_id;
        self.next_id = node_id.next();
        node_id
    }

    pub(crate) fn peek_next_id(&self) -> NodeId {
        self.next_id
    }

    #[must_use]
    pub fn node_ref(&self, node_id: NodeId) -> Option<&NodeImpl> {
        self.nodes.get(&node_id)
    }

    #[must_use]
    pub fn node_ref_mut(&mut self, node_id: NodeId) -> Option<&mut NodeImpl> {
        self.nodes.get_mut(&node_id)
    }

    #[must_use]
    pub fn node(&self, node_id: NodeId) -> Option<NodeImpl> {
        self.nodes.get(&node_id).cloned()
    }

    pub fn delete_node(&mut self, node_id: NodeId) {
        self.nodes.remove(&node_id);
    }

    pub fn update_node(&mut self, node: NodeImpl) {
        self.nodes.insert(node.id(), node);
    }

    pub fn register_node_with_node_id(&mut self, mut node: NodeImpl, node_id: NodeId) {
        assert!(!node.is_registered(), "Node is already attached to an arena");
        node.set_id(node_id);
        node.set_registered(true);
        self.nodes.insert(node_id, node);
    }

    pub fn register_node(&mut self, mut node: NodeImpl) -> NodeId {
        assert!(!node.is_registered(), "Node is already attached to an arena");
        let id = self.next_id;
        self.next_id = id.next();
        node.set_id(id);
        node.set_registered(true);
        self.nodes.insert(id, node);
        id
    }

    #[must_use]
    pub fn nodes(&self) -> &HashMap<NodeId, NodeImpl> {
        &self.nodes
    }
}

impl Default for NodeArena {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::HTML_NAMESPACE;
    use gosub_shared::byte_stream::Location;
    use std::collections::HashMap;

    #[test]
    fn register_node() {
        let mut arena = NodeArena::new();
        let node = NodeImpl::new_element(Location::default(), "test", Some(HTML_NAMESPACE), HashMap::new());
        let id = arena.register_node(node);
        assert_eq!(arena.nodes.len(), 1);
        assert_eq!(id, NodeId::from(0_usize));
    }

    #[test]
    #[should_panic(expected = "Node is already attached to an arena")]
    fn register_node_twice() {
        let mut arena = NodeArena::new();
        let node = NodeImpl::new_element(Location::default(), "test", Some(HTML_NAMESPACE), HashMap::new());
        let id = arena.register_node(node);
        let node = arena.node(id).unwrap();
        arena.register_node(node);
    }

    #[test]
    fn get_node() {
        let mut arena = NodeArena::new();
        let node = NodeImpl::new_element(Location::default(), "test", Some(HTML_NAMESPACE), HashMap::new());
        let id = arena.register_node(node);
        let node = arena.node(id);
        assert!(node.is_some());
        assert_eq!(node.unwrap().get_element_data().unwrap().name, "test");
    }
}
