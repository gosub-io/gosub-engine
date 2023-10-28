use crate::html5::node::Node;
use std::collections::HashMap;

use super::NodeId;

/// The node arena is the single source for nodes in a document (or fragment).
#[derive(Debug, Clone, PartialEq)]
pub struct NodeArena {
    /// Current nodes stored as <id, node>
    nodes: HashMap<NodeId, Node>,
    /// Order of nodes
    ///
    /// Note that the order of nodes isn't directly needed for functionality, but merely present
    /// for debugging purposes.
    order: Vec<NodeId>,
    /// Next node ID to use
    next_id: NodeId,
}

impl Clone for NodeId {
    fn clone(&self) -> Self {
        *self
    }
}

impl NodeArena {
    /// Creates a new NodeArena
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            next_id: Default::default(),
            order: Vec::new(),
        }
    }

    /// Gets the node with the given id
    pub fn get_node(&self, node_id: NodeId) -> Option<&Node> {
        self.nodes.get(&node_id)
    }

    /// Get the node with the given id as a mutable reference
    pub fn get_node_mut(&mut self, node_id: NodeId) -> Option<&mut Node> {
        self.nodes.get_mut(&node_id)
    }

    /// Registered an unregistered node into the arena
    pub fn register_node(&mut self, mut node: Node) -> NodeId {
        if node.is_registered {
            panic!("Node is already attached to an arena");
        }

        let id = self.next_id;
        self.next_id = id.next();

        node.is_registered = true;
        node.id = id;

        self.nodes.insert(id, node);
        self.order.push(id);
        id
    }

    /// Prints the list of nodes in sequential order. This makes debugging a bit easier, but should
    /// be removed.
    pub(crate) fn print_nodes(&self) {
        for id in self.order.iter() {
            println!("({}): {:?}", id, self.nodes.get(id).expect("node"));
        }
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
    use crate::html5::node::HTML_NAMESPACE;
    use crate::html5::parser::document::Document;

    #[test]
    fn register_node() {
        let mut doc = Document::shared();

        let node = Node::new_element(&doc, "test", HashMap::new(), HTML_NAMESPACE);
        let mut document = doc.get_mut();
        let id = document.arena.register_node(node);

        assert_eq!(document.arena.nodes.len(), 1);
        assert_eq!(document.arena.next_id, 1.into());
        assert_eq!(id, NodeId::default());
    }

    #[test]
    #[should_panic]
    fn register_node_twice() {
        let mut doc = Document::shared();

        let node = Node::new_element(&doc, "test", HashMap::new(), HTML_NAMESPACE);
        let mut document = doc.get_mut();
        document.arena.register_node(node);

        let node = document.get_node_by_id(NodeId(0)).unwrap().to_owned();
        document.arena.register_node(node);
    }

    #[test]
    fn get_node() {
        let mut doc = Document::shared();
        let node = Node::new_element(&doc, "test", HashMap::new(), HTML_NAMESPACE);

        let mut document = doc.get_mut();
        let id = document.arena.register_node(node);
        let node = document.arena.get_node(id);
        assert!(node.is_some());
        assert_eq!(node.unwrap().name, "test");
    }

    #[test]
    fn get_node_mut() {
        let mut doc = Document::shared();
        let node = Node::new_element(&doc, "test", HashMap::new(), HTML_NAMESPACE);

        let mut document = doc.get_mut();

        let node_id = document.arena.register_node(node);
        let node = document.arena.get_node_mut(node_id);
        assert!(node.is_some());
        assert_eq!(node.unwrap().name, "test");
    }

    #[test]
    fn register_node_through_document() {
        let mut doc = Document::shared();

        let parent = Node::new_element(&doc, "parent", HashMap::new(), HTML_NAMESPACE);
        let child = Node::new_element(&doc, "child", HashMap::new(), HTML_NAMESPACE);

        let mut document = doc.get_mut();
        let parent_id = document.arena.register_node(parent);
        let child_id = document.add_node(child, parent_id, None);

        let parent = document.get_node_by_id(parent_id);
        assert!(parent.is_some());
        assert_eq!(parent.unwrap().children.len(), 1);
        assert_eq!(parent.unwrap().children[0], child_id);

        let child = document.get_node_by_id(child_id);
        assert!(child.is_some());
        assert_eq!(child.unwrap().parent, Some(parent_id));
    }
}
