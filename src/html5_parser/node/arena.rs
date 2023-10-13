use crate::html5_parser::node::Node;
use std::collections::HashMap;

use super::NodeId;

/// The node arena is the single source for nodes in a document (or fragment).
#[derive(Debug, Clone, PartialEq)]
pub struct NodeArena {
    /// Current nodes stored as <id, node>
    nodes: HashMap<NodeId, Node>,
    /// Order of nodes
    order: Vec<NodeId>,
    /// Next node ID to use
    next_id: NodeId,
}

impl Clone for NodeId {
    fn clone(&self) -> Self {
        Self(self.0)
    }
}

impl NodeArena {
    /// Prints the list of nodes in sequential order. This makes debugging a bit easier, but should
    /// be removed.
    pub(crate) fn print_nodes(&self) {
        for id in self.order.iter() {
            println!("({}): {:?}", id, self.nodes.get(id).expect("node"));
        }
    }
}

impl NodeArena {
    /// Create a new NodeArena
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            next_id: Default::default(),
            order: Vec::new(),
        }
    }

    /// Get the node with the given id
    pub fn get_node(&self, node_id: NodeId) -> Option<&Node> {
        self.nodes.get(&node_id)
    }

    /// Get the node with the given id as a mutable reference
    pub fn get_node_mut(&mut self, node_id: NodeId) -> Option<&mut Node> {
        self.nodes.get_mut(&node_id)
    }

    /// Add the node to the arena and return its id
    pub fn add_node(&mut self, mut node: Node) -> NodeId {
        let id = self.next_id;
        self.next_id = id.next();

        node.id = id;
        self.nodes.insert(id, node);
        self.order.push(id);
        id
    }

    /// Add the node as a child the parent node
    pub fn attach_node(&mut self, parent_id: NodeId, node_id: NodeId) -> bool {
        //check if any children of node have parent as child
        if parent_id == node_id || has_child_recursive(self, node_id, parent_id) {
            return false;
        }
        if let Some(parent_node) = self.nodes.get_mut(&parent_id) {
            parent_node.children.push(node_id);
        }
        if let Some(node) = self.nodes.get_mut(&node_id) {
            node.parent = Some(parent_id);
        }

        true
    }

    /// Removes the node with the given id from the arena
    fn detach_node(&mut self, node_id: NodeId) {
        // Remove children
        if let Some(node) = self.nodes.get_mut(&node_id) {
            for child_id in node.children.clone() {
                self.detach_node(child_id);
            }
        }

        if let Some(node) = self.nodes.remove(&node_id) {
            self.order.retain(|&id| id != node_id);

            if let Some(parent_id) = node.parent {
                if let Some(parent_node) = self.nodes.get_mut(&parent_id) {
                    parent_node.children.retain(|&id| id != node_id);
                }
            }
        }
    }
}

impl Default for NodeArena {
    fn default() -> Self {
        Self::new()
    }
}

/// Returns true when the parent node has the child node as a child, or if any of the children of
/// the parent node have the child node as a child.
fn has_child_recursive(arena: &mut NodeArena, parent_id: NodeId, child_id: NodeId) -> bool {
    let node = arena.get_node_mut(parent_id).cloned();
    if node.is_none() {
        return false;
    }

    let node = node.unwrap();
    for id in node.children.iter() {
        if *id == child_id {
            return true;
        }
        let child = arena.get_node_mut(*id).cloned();
        if has_child(arena, child, child_id) {
            return true;
        }
    }
    false
}

fn has_child(arena: &mut NodeArena, parent: Option<Node>, child_id: NodeId) -> bool {
    let parent_node = if let Some(node) = parent {
        node
    } else {
        return false;
    };

    if parent_node.children.is_empty() {
        return false;
    }

    for id in parent_node.children {
        if id == child_id {
            return true;
        }
        let node = arena.get_node_mut(id).cloned();
        if has_child(arena, node, child_id) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html5_parser::node::HTML_NAMESPACE;
    use crate::html5_parser::parser::document::Document;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[test]
    fn add_node() {
        let mut arena = NodeArena::new();
        let document = Rc::new(RefCell::new(Document::new()));
        let node = Node::new_element(&document, "test", HashMap::new(), HTML_NAMESPACE);
        let id = arena.add_node(node);
        assert_eq!(arena.nodes.len(), 1);
        assert_eq!(arena.next_id, 1.into());
        assert_eq!(id, NodeId::default());
    }

    #[test]
    fn get_node() {
        let mut arena = NodeArena::new();
        let document = Rc::new(RefCell::new(Document::new()));
        let node = Node::new_element(&document, "test", HashMap::new(), HTML_NAMESPACE);
        let id = arena.add_node(node);
        let node = arena.get_node(id);
        assert!(node.is_some());
        assert_eq!(node.unwrap().name, "test");
    }

    #[test]
    fn get_node_mut() {
        let mut arena = NodeArena::new();
        let document = Rc::new(RefCell::new(Document::new()));
        let node = Node::new_element(&document, "test", HashMap::new(), HTML_NAMESPACE);
        let id = arena.add_node(node);
        let node = arena.get_node_mut(id);
        assert!(node.is_some());
        assert_eq!(node.unwrap().name, "test");
    }

    #[test]
    fn attach_node() {
        let mut arena = NodeArena::new();
        let document = Rc::new(RefCell::new(Document::new()));

        let parent = Node::new_element(&document, "parent", HashMap::new(), HTML_NAMESPACE);
        let parent_id = arena.add_node(parent);

        let child = Node::new_element(&document, "child", HashMap::new(), HTML_NAMESPACE);
        let child_id = arena.add_node(child);

        assert!(arena.attach_node(parent_id, child_id));

        let parent = arena.get_node(parent_id);
        assert!(parent.is_some());
        assert_eq!(parent.unwrap().children.len(), 1);
        assert_eq!(parent.unwrap().children[0], child_id);

        let child = arena.get_node(child_id);
        assert!(child.is_some());
        assert_eq!(child.unwrap().parent, Some(parent_id));
    }

    #[test]
    fn attach_node_to_itself() {
        let mut arena = NodeArena::new();
        let document = Rc::new(RefCell::new(Document::new()));

        let node = Node::new_element(&document, "some_node", HashMap::new(), HTML_NAMESPACE);
        let node_id = arena.add_node(node);

        assert!(!arena.attach_node(node_id, node_id));

        let node = arena.get_node(node_id);
        assert!(node.is_some());
        assert_eq!(node.unwrap().children.len(), 0);
    }

    #[test]
    fn attach_node_with_loop_pointer() {
        let mut arena = NodeArena::new();
        let document = Rc::new(RefCell::new(Document::new()));
        let parent = Node::new_element(&document, "parent", HashMap::new(), HTML_NAMESPACE);
        let mut child = Node::new_element(&document, "child", HashMap::new(), HTML_NAMESPACE);

        // push the PARENT to the CHILD
        let parent_id = arena.add_node(parent);
        child.children.push(parent_id);

        // try and add the CHILD to the PARENT
        let child_id = arena.add_node(child);
        assert!(!arena.attach_node(parent_id, child_id));

        let parent = arena.get_node(parent_id);
        let child = arena.get_node(child_id);
        assert!(parent.is_some());
        assert!(child.is_some());
        assert_eq!(parent.unwrap().children.len(), 0); // parent could not add child, recursive
        assert_eq!(child.unwrap().children.len(), 1); // child adding the parent is ok
    }

    #[test]
    fn attach_node_with_indirect_loop_pointer() {
        let mut arena = NodeArena::new();
        let document = Rc::new(RefCell::new(Document::new()));
        let parent = Node::new_element(&document, "parent", HashMap::new(), HTML_NAMESPACE);
        let child1 = Node::new_element(&document, "child1", HashMap::new(), HTML_NAMESPACE);
        let child2 = Node::new_element(&document, "child2", HashMap::new(), HTML_NAMESPACE);

        let parent_id = arena.add_node(parent);
        let child1_id = arena.add_node(child1);
        let child2_id = arena.add_node(child2);

        assert!(arena.attach_node(parent_id, child1_id));
        assert!(arena.attach_node(child1_id, child2_id));

        let parent = arena.get_node(parent_id);
        let child1 = arena.get_node(child1_id);
        let child2 = arena.get_node(child2_id);
        assert_eq!(parent.unwrap().children.len(), 1);
        assert_eq!(child1.unwrap().children.len(), 1);
        assert_eq!(child2.unwrap().children.len(), 0);

        // Add parent to child 2, thus creating a loop
        assert!(!arena.attach_node(child2_id, parent_id));

        let parent = arena.get_node(parent_id);
        let child1 = arena.get_node(child1_id);
        let child2 = arena.get_node(child2_id);
        assert_eq!(parent.unwrap().children.len(), 1);
        assert_eq!(child1.unwrap().children.len(), 1);
        assert_eq!(child2.unwrap().children.len(), 0);
    }

    #[test]
    fn detach_node() {
        let mut arena = NodeArena::new();
        let document = Rc::new(RefCell::new(Document::new()));

        let parent = Node::new_element(&document, "parent", HashMap::new(), HTML_NAMESPACE);
        let parent_id = arena.add_node(parent);
        let child = Node::new_element(&document, "child", HashMap::new(), HTML_NAMESPACE);
        let child_id = arena.add_node(child);

        arena.attach_node(parent_id, child_id);
        arena.detach_node(child_id);

        let parent = arena.get_node(parent_id);
        assert!(parent.is_some());
        assert_eq!(parent.unwrap().children.len(), 0);

        let child = arena.get_node(child_id);
        assert!(child.is_none());
    }

    #[test]
    fn remove_child_node() {
        let mut arena = NodeArena::new();
        let document = Rc::new(RefCell::new(Document::new()));

        let parent = Node::new_element(&document, "parent", HashMap::new(), HTML_NAMESPACE);
        let parent_id = arena.add_node(parent);
        let child1 = Node::new_element(&document, "child1", HashMap::new(), HTML_NAMESPACE);
        let child1_id = arena.add_node(child1);
        let child2 = Node::new_element(&document, "child2", HashMap::new(), HTML_NAMESPACE);
        let child2_id = arena.add_node(child2);

        arena.attach_node(parent_id, child1_id);
        arena.attach_node(parent_id, child2_id);
        let parent = arena.get_node(parent_id);
        assert!(parent.is_some());
        assert_eq!(parent.unwrap().children.len(), 2);

        arena.detach_node(child1_id);

        let child = arena.get_node(child1_id);
        assert!(child.is_none());
        let child = arena.get_node(child2_id);
        assert!(child.is_some());
        assert_eq!(child.unwrap().parent, Some(parent_id));

        let parent = arena.get_node(parent_id);
        assert_eq!(parent.unwrap().children.len(), 1);
    }

    #[test]
    fn detach_node_with_children() {
        let mut arena = NodeArena::new();
        let document = Rc::new(RefCell::new(Document::new()));

        let parent = Node::new_element(&document, "parent", HashMap::new(), HTML_NAMESPACE);
        let parent_id = arena.add_node(parent);
        let child = Node::new_element(&document, "child", HashMap::new(), HTML_NAMESPACE);
        let child_id = arena.add_node(child);

        arena.attach_node(parent_id, child_id);
        arena.detach_node(parent_id);

        let parent = arena.get_node(parent_id);
        assert!(parent.is_none());
        let child = arena.get_node(child_id);
        assert!(child.is_none());
    }

    #[test]
    fn detach_node_with_children_and_parent() {
        let mut arena = NodeArena::new();
        let document = Rc::new(RefCell::new(Document::new()));

        let parent = Node::new_element(&document, "parent", HashMap::new(), HTML_NAMESPACE);
        let parent_id = arena.add_node(parent);
        let child = Node::new_element(&document, "child", HashMap::new(), HTML_NAMESPACE);
        let child_id = arena.add_node(child);

        arena.attach_node(parent_id, child_id);
        arena.detach_node(child_id);
        arena.detach_node(parent_id);

        let parent = arena.get_node(parent_id);
        assert!(parent.is_none());
        let child = arena.get_node(child_id);
        assert!(child.is_none());
    }

    #[test]
    fn detach_node_with_children_and_parent_and_grandchildren() {
        let mut arena = NodeArena::new();
        let document = Rc::new(RefCell::new(Document::new()));

        let parent = Node::new_element(&document, "parent", HashMap::new(), HTML_NAMESPACE);
        let parent_id = arena.add_node(parent);
        let child = Node::new_element(&document, "child", HashMap::new(), HTML_NAMESPACE);
        let child_id = arena.add_node(child);
        let grandchild = Node::new_element(&document, "grandchild", HashMap::new(), HTML_NAMESPACE);
        let grandchild_id = arena.add_node(grandchild);

        arena.attach_node(parent_id, child_id);
        arena.attach_node(child_id, grandchild_id);
        arena.detach_node(child_id);
        arena.detach_node(parent_id);

        let parent = arena.get_node(parent_id);
        assert!(parent.is_none());
        let child = arena.get_node(child_id);
        assert!(child.is_none());
        let grandchild = arena.get_node(grandchild_id);
        assert!(grandchild.is_none());
    }

    #[test]
    fn detach_node_with_children_and_parent_and_grandchildren_and_siblings() {
        let mut arena = NodeArena::new();
        let document = Rc::new(RefCell::new(Document::new()));

        let parent = Node::new_element(&document, "parent", HashMap::new(), HTML_NAMESPACE);
        let parent_id = arena.add_node(parent);
        let child = Node::new_element(&document, "child", HashMap::new(), HTML_NAMESPACE);
        let child_id = arena.add_node(child);
        let grandchild = Node::new_element(&document, "grandchild", HashMap::new(), HTML_NAMESPACE);
        let grandchild_id = arena.add_node(grandchild);
        let sibling = Node::new_element(&document, "sibling", HashMap::new(), HTML_NAMESPACE);
        let sibling_id = arena.add_node(sibling);

        arena.attach_node(parent_id, child_id);
        arena.attach_node(child_id, grandchild_id);
        arena.attach_node(parent_id, sibling_id);
        arena.detach_node(child_id);
        arena.detach_node(parent_id);

        let parent = arena.get_node(parent_id);
        assert!(parent.is_none());
        let child = arena.get_node(child_id);
        assert!(child.is_none());
        let grandchild = arena.get_node(grandchild_id);
        assert!(grandchild.is_none());
        let sibling = arena.get_node(sibling_id);
        assert!(sibling.is_none());
    }
}
