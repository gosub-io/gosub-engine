use std::collections::HashMap;
use crate::html5_parser::node::Node;

pub struct NodeArena {
    nodes: HashMap<usize, Node>,        // Current nodes
    next_id: usize,                     // next id to use
}

impl NodeArena {
    // Create a new NodeArena
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            next_id: 0,
        }
    }

    // Get the node with the given id
    pub fn get_node(&self, node_id: usize) -> Option<&Node> {
        self.nodes.get(&node_id)
    }

    // Get the node with the given id as a mutable reference
    pub fn get_mut_node(&mut self, node_id: usize) -> Option<&mut Node> {
        self.nodes.get_mut(&node_id)
    }

    // Add the node to the arena and return its id
    pub fn add_node(&mut self, mut node: Node) -> usize {
        let id = self.next_id;
        self.next_id += 1;

        node.id = id;
        self.nodes.insert(id, node);
        id
    }

    // Add the node as a child the parent node
    pub fn attach_node(&mut self, parent_id: usize, node_id: usize) {
        if let Some(parent_node) = self.nodes.get_mut(&parent_id) {
            parent_node.children.push(node_id);
        }
        if let Some(node) = self.nodes.get_mut(&node_id) {
            node.parent = Some(parent_id);
        }
    }

    // Removes the node with the given id from the arena
    fn remove_node(&mut self, node_id: usize) {
        // Remove children

        if let Some(node) = self.nodes.get_mut(&node_id) {
            for child_id in node.children.clone() {
                self.remove_node(child_id);
            }
        }

        if let Some(node) = self.nodes.remove(&node_id) {
            if let Some(parent_id) = node.parent {
                if let Some(parent_node) = self.nodes.get_mut(&parent_id) {
                    parent_node.children.retain(|&id| id != node_id);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::html5_parser::node::HTML_NAMESPACE;
    use super::*;

    #[test]
    fn test_add_node() {
        let mut arena = NodeArena::new();
        let node = Node::new_element("test", HashMap::new(), HTML_NAMESPACE);
        let id = arena.add_node(node);
        assert_eq!(arena.nodes.len(), 1);
        assert_eq!(arena.next_id, 1);
        assert_eq!(id, 0);
    }

    #[test]
    fn test_get_node() {
        let mut arena = NodeArena::new();
        let node = Node::new_element("test", HashMap::new(), HTML_NAMESPACE);
        let id = arena.add_node(node);
        let node = arena.get_node(id);
        assert!(node.is_some());
        assert_eq!(node.unwrap().name, "test");
    }

    #[test]
    fn test_get_mut_node() {
        let mut arena = NodeArena::new();
        let node = Node::new_element("test", HashMap::new(), HTML_NAMESPACE);
        let id = arena.add_node(node);
        let node = arena.get_mut_node(id);
        assert!(node.is_some());
        assert_eq!(node.unwrap().name, "test");
    }

    #[test]
    fn test_attach_node() {
        let mut arena = NodeArena::new();
        let parent = Node::new_element("parent", HashMap::new(), HTML_NAMESPACE);
        let parent_id = arena.add_node(parent);
        let child = Node::new_element("child", HashMap::new(), HTML_NAMESPACE);
        let child_id = arena.add_node(child);
        arena.attach_node(parent_id, child_id);
        let parent = arena.get_node(parent_id);
        assert!(parent.is_some());
        assert_eq!(parent.unwrap().children.len(), 1);
        assert_eq!(parent.unwrap().children[0], child_id);
        let child = arena.get_node(child_id);
        assert!(child.is_some());
        assert_eq!(child.unwrap().parent, Some(parent_id));
    }

    #[test]
    fn test_remove_node() {
        let mut arena = NodeArena::new();
        let parent = Node::new_element("parent", HashMap::new(), HTML_NAMESPACE);
        let parent_id = arena.add_node(parent);
        let child = Node::new_element("child", HashMap::new(), HTML_NAMESPACE);
        let child_id = arena.add_node(child);
        arena.attach_node(parent_id, child_id);
        arena.remove_node(child_id);
        let parent = arena.get_node(parent_id);
        assert!(parent.is_some());
        assert_eq!(parent.unwrap().children.len(), 0);
        let child = arena.get_node(child_id);
        assert!(child.is_none());
    }

    #[test]
    fn test_remove_child_node() {
        let mut arena = NodeArena::new();
        let parent = Node::new_element("parent", HashMap::new(), HTML_NAMESPACE);
        let parent_id = arena.add_node(parent);
        let child1 = Node::new_element("child1", HashMap::new(), HTML_NAMESPACE);
        let child1_id = arena.add_node(child1);
        let child2 = Node::new_element("child2", HashMap::new(), HTML_NAMESPACE);
        let child2_id = arena.add_node(child2);

        arena.attach_node(parent_id, child1_id);
        arena.attach_node(parent_id, child2_id);
        let parent = arena.get_node(parent_id);
        assert!(parent.is_some());
        assert_eq!(parent.unwrap().children.len(), 2);

        arena.remove_node(child1_id);

        let child = arena.get_node(child1_id);
        assert!(child.is_none());
        let child = arena.get_node(child2_id);
        assert!(child.is_some());
        assert_eq!(child.unwrap().parent, Some(parent_id));

        let parent = arena.get_node(parent_id);
        assert_eq!(parent.unwrap().children.len(), 1);
    }


    #[test]
    fn test_remove_node_with_children() {
        let mut arena = NodeArena::new();
        let parent = Node::new_element("parent", HashMap::new(), HTML_NAMESPACE);
        let parent_id = arena.add_node(parent);
        let child = Node::new_element("child", HashMap::new(), HTML_NAMESPACE);
        let child_id = arena.add_node(child);
        arena.attach_node(parent_id, child_id);
        arena.remove_node(parent_id);
        let parent = arena.get_node(parent_id);
        assert!(parent.is_none());
        let child = arena.get_node(child_id);
        assert!(child.is_none());
    }

    #[test]
    fn test_remove_node_with_children_and_parent() {
        let mut arena = NodeArena::new();
        let parent = Node::new_element("parent", HashMap::new(), HTML_NAMESPACE);
        let parent_id = arena.add_node(parent);
        let child = Node::new_element("child", HashMap::new(), HTML_NAMESPACE);
        let child_id = arena.add_node(child);
        arena.attach_node(parent_id, child_id);
        arena.remove_node(child_id);
        arena.remove_node(parent_id);
        let parent = arena.get_node(parent_id);
        assert!(parent.is_none());
        let child = arena.get_node(child_id);
        assert!(child.is_none());
    }

    #[test]
    fn test_remove_node_with_children_and_parent_and_grandchildren() {
        let mut arena = NodeArena::new();
        let parent = Node::new_element("parent", HashMap::new(), HTML_NAMESPACE);
        let parent_id = arena.add_node(parent);
        let child = Node::new_element("child", HashMap::new(), HTML_NAMESPACE);
        let child_id = arena.add_node(child);
        let grandchild = Node::new_element("grandchild", HashMap::new(), HTML_NAMESPACE);
        let grandchild_id = arena.add_node(grandchild);
        arena.attach_node(parent_id, child_id);
        arena.attach_node(child_id, grandchild_id);
        arena.remove_node(child_id);
        arena.remove_node(parent_id);
        let parent = arena.get_node(parent_id);
        assert!(parent.is_none());
        let child = arena.get_node(child_id);
        assert!(child.is_none());
        let grandchild = arena.get_node(grandchild_id);
        assert!(grandchild.is_none());
    }

    #[test]
    fn test_remove_node_with_children_and_parent_and_grandchildren_and_siblings() {
        let mut arena = NodeArena::new();
        let parent = Node::new_element("parent", HashMap::new(), HTML_NAMESPACE);
        let parent_id = arena.add_node(parent);
        let child = Node::new_element("child", HashMap::new(), HTML_NAMESPACE);
        let child_id = arena.add_node(child);
        let grandchild = Node::new_element("grandchild", HashMap::new(), HTML_NAMESPACE);
        let grandchild_id = arena.add_node(grandchild);
        let sibling = Node::new_element("sibling", HashMap::new(), HTML_NAMESPACE);
        let sibling_id = arena.add_node(sibling);
        arena.attach_node(parent_id, child_id);
        arena.attach_node(child_id, grandchild_id);
        arena.attach_node(parent_id, sibling_id);
        arena.remove_node(child_id);
        arena.remove_node(parent_id);
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
