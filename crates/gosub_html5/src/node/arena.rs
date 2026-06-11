use crate::node::node_impl::NodeImpl;
use gosub_shared::node::NodeId;

/// The node arena is the single source for nodes in a document (or fragment).
/// Node ids are sequential, so nodes are stored in a `Vec` indexed by id;
/// deleted nodes leave a `None` slot behind (ids are never reused).
#[derive(Debug, Clone)]
pub struct NodeArena {
    nodes: Vec<Option<NodeImpl>>,
    next_id: NodeId,
    /// Number of `Some` entries in `nodes`
    count: usize,
}

impl PartialEq for NodeArena {
    fn eq(&self, other: &Self) -> bool {
        self.next_id == other.next_id && self.count == other.count && self.nodes == other.nodes
    }
}

impl NodeArena {
    #[must_use]
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            next_id: NodeId::default(),
            count: 0,
        }
    }

    #[must_use]
    pub fn node_count(&self) -> usize {
        self.count
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
        self.nodes.get(node_id.as_usize()).and_then(Option::as_ref)
    }

    #[must_use]
    pub fn node_ref_mut(&mut self, node_id: NodeId) -> Option<&mut NodeImpl> {
        self.nodes.get_mut(node_id.as_usize()).and_then(Option::as_mut)
    }

    #[must_use]
    pub fn node(&self, node_id: NodeId) -> Option<NodeImpl> {
        self.node_ref(node_id).cloned()
    }

    pub fn delete_node(&mut self, node_id: NodeId) {
        if let Some(slot) = self.nodes.get_mut(node_id.as_usize()) {
            if slot.take().is_some() {
                self.count -= 1;
            }
        }
    }

    /// Place `node` in the slot for `id`, growing the slot vector when needed.
    fn put(&mut self, id: NodeId, node: NodeImpl) {
        let idx = id.as_usize();
        if idx >= self.nodes.len() {
            self.nodes.resize_with(idx + 1, || None);
        }
        if self.nodes[idx].replace(node).is_none() {
            self.count += 1;
        }
    }

    pub fn update_node(&mut self, node: NodeImpl) {
        self.put(node.id(), node);
    }

    pub fn register_node_with_node_id(&mut self, mut node: NodeImpl, node_id: NodeId) {
        assert!(!node.is_registered(), "Node is already attached to an arena");
        node.set_id(node_id);
        node.set_registered(true);
        self.put(node_id, node);
    }

    pub fn register_node(&mut self, mut node: NodeImpl) -> NodeId {
        assert!(!node.is_registered(), "Node is already attached to an arena");
        let id = self.next_id;
        self.next_id = id.next();
        node.set_id(id);
        node.set_registered(true);
        self.put(id, node);
        id
    }

    /// Iterate over all registered nodes with their ids.
    pub fn nodes(&self) -> impl Iterator<Item = (NodeId, &NodeImpl)> {
        self.nodes
            .iter()
            .enumerate()
            .filter_map(|(idx, slot)| slot.as_ref().map(|node| (NodeId::from(idx), node)))
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
        assert_eq!(arena.node_count(), 1);
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

    #[test]
    fn delete_node_leaves_tombstone() {
        let mut arena = NodeArena::new();
        let node = NodeImpl::new_element(Location::default(), "test", Some(HTML_NAMESPACE), HashMap::new());
        let id = arena.register_node(node);
        assert_eq!(arena.node_count(), 1);
        arena.delete_node(id);
        assert_eq!(arena.node_count(), 0);
        assert!(arena.node_ref(id).is_none());
        // ids are not reused
        let node = NodeImpl::new_element(Location::default(), "test2", Some(HTML_NAMESPACE), HashMap::new());
        let id2 = arena.register_node(node);
        assert_ne!(id, id2);
    }
}
