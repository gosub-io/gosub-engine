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

    // #[test]
    // fn attach_node_with_position() {
    //     let mut arena = NodeArena::new();
    //     let mut document = Document::shared();
    //
    //     let parent = Node::new_element(&document, "parent", HashMap::new(), HTML_NAMESPACE);
    //     let parent_id = arena.register_node(parent);
    //
    //     let child1 = Node::new_element(&document, "child1", HashMap::new(), HTML_NAMESPACE);
    //     let child1_id = arena.register_node(child1);
    //     let child2 = Node::new_element(&document, "child2", HashMap::new(), HTML_NAMESPACE);
    //     let child2_id = arena.register_node(child2);
    //     let child3 = Node::new_element(&document, "child3", HashMap::new(), HTML_NAMESPACE);
    //     let child3_id = arena.register_node(child3);
    //     let child4 = Node::new_element(&document, "child4", HashMap::new(), HTML_NAMESPACE);
    //     let child4_id = arena.register_node(child4);
    //
    //     assert!(document.add_node(child1, parent_id, None));
    //     assert!(document.add_node(child2, parent_id, None));
    //     assert!(document.add_node(child3, parent_id, Some(1)));
    //
    //     let parent = arena.get_node(parent_id);
    //     assert!(parent.is_some());
    //     assert_eq!(parent.unwrap().children.len(), 3);
    //     assert_eq!(parent.unwrap().children[0], child1_id);
    //     assert_eq!(parent.unwrap().children[1], child3_id);
    //     assert_eq!(parent.unwrap().children[2], child2_id);
    //
    //     // Insert at a very large position which doesn't exist
    //     assert!(document.add_node(parent_id, child4_id, Some(123456)));
    //
    //     let parent = arena.get_node(parent_id);
    //     assert!(parent.is_some());
    //     assert_eq!(parent.unwrap().children.len(), 4);
    //     assert_eq!(parent.unwrap().children[0], child1_id);
    //     assert_eq!(parent.unwrap().children[1], child3_id);
    //     assert_eq!(parent.unwrap().children[2], child2_id);
    //     assert_eq!(parent.unwrap().children[3], child4_id);
    // }
    //
    // #[test]
    // fn attach_node_to_itself() {
    //     let mut arena = NodeArena::new();
    //     let document = Document::shared();
    //
    //     let node = Node::new_element(&document, "some_node", HashMap::new(), HTML_NAMESPACE);
    //     let node_id = arena.register_node(node);
    //
    //     assert!(!document.add_node(node_id, node_id, None));
    //
    //     let node = arena.get_node(node_id);
    //     assert!(node.is_some());
    //     assert_eq!(node.unwrap().children.len(), 0);
    // }
    //
    // #[test]
    // fn attach_node_with_loop_pointer() {
    //     let mut arena = NodeArena::new();
    //     let document = Document::shared();
    //     let parent = Node::new_element(&document, "parent", HashMap::new(), HTML_NAMESPACE);
    //     let mut child = Node::new_element(&document, "child", HashMap::new(), HTML_NAMESPACE);
    //
    //     // push the PARENT to the CHILD
    //     let parent_id = arena.register_node(parent);
    //     child.children.push(parent_id);
    //
    //     // try and add the CHILD to the PARENT
    //     let child_id = arena.register_node(child);
    //     assert!(!document.add_node(parent_id, child_id, None));
    //
    //     let parent = arena.get_node(parent_id);
    //     let child = arena.get_node(child_id);
    //     assert!(parent.is_some());
    //     assert!(child.is_some());
    //     assert_eq!(parent.unwrap().children.len(), 0); // parent could not add child, recursive
    //     assert_eq!(child.unwrap().children.len(), 1); // child adding the parent is ok
    // }
    //
    // #[test]
    // fn attach_node_with_indirect_loop_pointer() {
    //     let mut arena = NodeArena::new();
    //     let document = Document::shared();
    //     let parent = Node::new_element(&document, "parent", HashMap::new(), HTML_NAMESPACE);
    //     let child1 = Node::new_element(&document, "child1", HashMap::new(), HTML_NAMESPACE);
    //     let child2 = Node::new_element(&document, "child2", HashMap::new(), HTML_NAMESPACE);
    //
    //     let parent_id = arena.register_node(parent);
    //     let child1_id = arena.register_node(child1);
    //     let child2_id = arena.register_node(child2);
    //
    //     assert!(document.add_node(parent_id, child1_id, None));
    //     assert!(document.add_node(child1_id, child2_id, None));
    //
    //     let parent = arena.get_node(parent_id);
    //     let child1 = arena.get_node(child1_id);
    //     let child2 = arena.get_node(child2_id);
    //     assert_eq!(parent.unwrap().children.len(), 1);
    //     assert_eq!(child1.unwrap().children.len(), 1);
    //     assert_eq!(child2.unwrap().children.len(), 0);
    //
    //     // Add parent to child 2, thus creating a loop
    //     assert!(!document.add_node(child2_id, parent_id, None));
    //
    //     let parent = arena.get_node(parent_id);
    //     let child1 = arena.get_node(child1_id);
    //     let child2 = arena.get_node(child2_id);
    //     assert_eq!(parent.unwrap().children.len(), 1);
    //     assert_eq!(child1.unwrap().children.len(), 1);
    //     assert_eq!(child2.unwrap().children.len(), 0);
    // }
    //
    // #[test]
    // fn remove_child_node() {
    //     let mut arena = NodeArena::new();
    //     let document = Document::shared();
    //
    //     let parent = Node::new_element(&document, "parent", HashMap::new(), HTML_NAMESPACE);
    //     let parent_id = arena.register_node(parent);
    //     let child1 = Node::new_element(&document, "child1", HashMap::new(), HTML_NAMESPACE);
    //     let child1_id = arena.register_node(child1);
    //     let child2 = Node::new_element(&document, "child2", HashMap::new(), HTML_NAMESPACE);
    //     let child2_id = arena.register_node(child2);
    //
    //     arena.attach_node(parent_id, child1_id, None);
    //     arena.attach_node(parent_id, child2_id, None);
    //     let parent = arena.get_node(parent_id);
    //     assert!(parent.is_some());
    //     assert_eq!(parent.unwrap().children.len(), 2);
    //
    //     arena.detach_node(child1_id);
    //
    //     let child = arena.get_node(child1_id);
    //     assert!(child.is_none());
    //     let child = arena.get_node(child2_id);
    //     assert!(child.is_some());
    //     assert_eq!(child.unwrap().parent, Some(parent_id));
    //
    //     let parent = arena.get_node(parent_id);
    //     assert_eq!(parent.unwrap().children.len(), 1);
    // }
    //
    // #[test]
    // fn detach_node_with_children() {
    //     let mut arena = NodeArena::new();
    //     let document = Document::shared();
    //
    //     let parent = Node::new_element(&document, "parent", HashMap::new(), HTML_NAMESPACE);
    //     let parent_id = arena.register_node(parent);
    //     let child = Node::new_element(&document, "child", HashMap::new(), HTML_NAMESPACE);
    //     let child_id = arena.register_node(child);
    //
    //     arena.attach_node(parent_id, child_id, None);
    //     arena.detach_node(parent_id);
    //
    //     let parent = arena.get_node(parent_id);
    //     assert!(parent.is_none());
    //     let child = arena.get_node(child_id);
    //     assert!(child.is_none());
    // }
    //
    // #[test]
    // fn detach_node_with_children_and_parent() {
    //     let mut arena = NodeArena::new();
    //     let document = Document::shared();
    //
    //     let parent = Node::new_element(&document, "parent", HashMap::new(), HTML_NAMESPACE);
    //     let parent_id = arena.register_node(parent);
    //     let child = Node::new_element(&document, "child", HashMap::new(), HTML_NAMESPACE);
    //     let child_id = arena.register_node(child);
    //
    //     arena.attach_node(parent_id, child_id, None);
    //     arena.detach_node(child_id);
    //     arena.detach_node(parent_id);
    //
    //     let parent = arena.get_node(parent_id);
    //     assert!(parent.is_none());
    //     let child = arena.get_node(child_id);
    //     assert!(child.is_none());
    // }
    //
    // #[test]
    // fn detach_node_with_children_and_parent_and_grandchildren() {
    //     let mut arena = NodeArena::new();
    //     let document = Document::shared();
    //
    //     let parent = Node::new_element(&document, "parent", HashMap::new(), HTML_NAMESPACE);
    //     let parent_id = arena.register_node(parent);
    //     let child = Node::new_element(&document, "child", HashMap::new(), HTML_NAMESPACE);
    //     let child_id = arena.register_node(child);
    //     let grandchild = Node::new_element(&document, "grandchild", HashMap::new(), HTML_NAMESPACE);
    //     let grandchild_id = arena.register_node(grandchild);
    //
    //     arena.attach_node(parent_id, child_id, None);
    //     arena.attach_node(child_id, grandchild_id, None);
    //     arena.detach_node(child_id);
    //     arena.detach_node(parent_id);
    //
    //     let parent = arena.get_node(parent_id);
    //     assert!(parent.is_none());
    //     let child = arena.get_node(child_id);
    //     assert!(child.is_none());
    //     let grandchild = arena.get_node(grandchild_id);
    //     assert!(grandchild.is_none());
    // }
    //
    // #[test]
    // fn detach_node_with_children_and_parent_and_grandchildren_and_siblings() {
    //     let mut arena = NodeArena::new();
    //     let document = Document::shared();
    //
    //     let parent = Node::new_element(&document, "parent", HashMap::new(), HTML_NAMESPACE);
    //     let parent_id = arena.register_node(parent);
    //     let child = Node::new_element(&document, "child", HashMap::new(), HTML_NAMESPACE);
    //     let child_id = arena.register_node(child);
    //     let grandchild = Node::new_element(&document, "grandchild", HashMap::new(), HTML_NAMESPACE);
    //     let grandchild_id = arena.register_node(grandchild);
    //     let sibling = Node::new_element(&document, "sibling", HashMap::new(), HTML_NAMESPACE);
    //     let sibling_id = arena.register_node(sibling);
    //
    //     arena.attach_node(parent_id, child_id, None);
    //     arena.attach_node(child_id, grandchild_id, None);
    //     arena.attach_node(parent_id, sibling_id, None);
    //     arena.detach_node(child_id);
    //     arena.detach_node(parent_id);
    //
    //     let parent = arena.get_node(parent_id);
    //     assert!(parent.is_none());
    //     let child = arena.get_node(child_id);
    //     assert!(child.is_none());
    //     let grandchild = arena.get_node(grandchild_id);
    //     assert!(grandchild.is_none());
    //     let sibling = arena.get_node(sibling_id);
    //     assert!(sibling.is_none());
    // }
}
