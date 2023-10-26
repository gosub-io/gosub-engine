use crate::html5::node::arena::NodeArena;
use crate::html5::node::data::{comment::CommentData, text::TextData};
use crate::html5::node::NodeTrait;
use crate::html5::node::NodeType;
use crate::html5::node::{Node, NodeData, NodeId};
use crate::html5::parser::quirks::QuirksMode;
use alloc::rc::Rc;
use core::fmt;
use core::fmt::Debug;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt::Display;
use std::ops::{Deref, DerefMut};

/// Type of the given document
#[derive(PartialEq, Debug, Copy, Clone)]
pub enum DocumentType {
    /// HTML document
    HTML,
    /// Iframe source document
    IframeSrcDoc,
}

/// Defines a document fragment which can be attached to for instance a <template> element
#[derive(PartialEq)]
pub struct DocumentFragment {
    /// Node elements inside this fragment
    arena: NodeArena,
    /// Document handle of the parent
    pub doc: DocumentHandle,
    /// Host node on which this fragment is attached
    host: NodeId,
}

impl Clone for DocumentFragment {
    /// Clones the document fragment
    fn clone(&self) -> Self {
        Self {
            arena: self.arena.clone(),
            doc: Document::clone(&self.doc),
            host: self.host,
        }
    }
}

impl Debug for DocumentFragment {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "DocumentFragment")
    }
}

impl DocumentFragment {
    /// Creates a new document fragment and attaches it to "host" node inside "doc"
    pub(crate) fn new(doc: DocumentHandle, host: NodeId) -> Self {
        Self {
            arena: NodeArena::new(),
            doc,
            host,
        }
    }
}

/// Defines a document
#[derive(Debug, PartialEq)]
pub struct Document {
    /// Holds and owns all nodes in the document
    pub(crate) arena: NodeArena,
    /// HTML elements with ID (e.g., <div id="myid">)
    named_id_elements: HashMap<String, NodeId>,
    /// Document type of this document
    pub doctype: DocumentType,
    /// Quirks mode of this document
    pub quirks_mode: QuirksMode,
}

impl Default for Document {
    /// Returns a default document
    fn default() -> Self {
        Self {
            arena: NodeArena::new(),
            named_id_elements: HashMap::new(),
            doctype: DocumentType::HTML,
            quirks_mode: QuirksMode::NoQuirks,
        }
    }
}

impl Document {
    /// Creates a new document
    pub fn new() -> Self {
        let arena = NodeArena::new();
        Self {
            arena,
            named_id_elements: HashMap::new(),
            doctype: DocumentType::HTML,
            quirks_mode: QuirksMode::NoQuirks,
        }
    }

    /// Returns a shared reference-counted handle for the document
    pub fn shared() -> DocumentHandle {
        DocumentHandle(Rc::new(RefCell::new(Self::new())))
    }

    /// Fast clone of a lightweight reference-counted handle for the document.  This is a shallow
    /// clone, and different handles will see the same underlying document.
    pub fn clone(handle: &DocumentHandle) -> DocumentHandle {
        DocumentHandle(Rc::clone(&handle.0))
    }

    pub(crate) fn print_nodes(&self) {
        self.arena.print_nodes();
    }

    /// Creates the document root node
    pub fn create_root(&mut self, document: &DocumentHandle) {
        // previously this used to be in the constructor, but now that
        // we require a document pointer with every node creation, this
        // was separated.

        let node = Node::new_document(document);
        self.arena.register_node(node);
    }

    /// Fetches a node by id or returns None when no node with this ID is found
    pub fn get_node_by_id(&self, node_id: NodeId) -> Option<&Node> {
        self.arena.get_node(node_id)
    }

    /// Fetches a mutable node by id or returns None when no node with this ID is found
    pub fn get_node_by_id_mut(&mut self, node_id: NodeId) -> Option<&mut Node> {
        self.arena.get_node_mut(node_id)
    }

    /// Fetches a node by named id (string) or returns None when no node with this ID is found
    pub fn get_node_by_named_id(&self, named_id: &str) -> Option<&Node> {
        let node_id = self.named_id_elements.get(named_id)?;
        self.arena.get_node(*node_id)
    }

    /// Fetches a mutable node by named id (string) or returns None when no node with this ID is found
    pub fn get_node_by_named_id_mut(&mut self, named_id: &str) -> Option<&mut Node> {
        let node_id = self.named_id_elements.get(named_id)?;
        self.arena.get_node_mut(*node_id)
    }

    /// according to HTML5 spec: 3.2.3.1
    /// https://www.w3.org/TR/2011/WD-html5-20110405/elements.html#the-id-attribute
    fn validate_named_id(&self, named_id: &str) -> bool {
        if named_id.contains(char::is_whitespace) {
            return false;
        }

        if named_id.is_empty() {
            return false;
        }

        // must contain at least one character, but
        // doesn't specify it should *start* with a character
        if !named_id.contains(char::is_alphabetic) {
            return false;
        }

        true
    }

    /// Set a new named ID on a node (also updates the underlying node's attribute)
    /// ID will NOT be set if it doesn't pass validation
    pub fn set_node_named_id(&mut self, node_id: NodeId, named_id: &str) {
        if !self.validate_named_id(named_id) {
            return;
        }

        // if ID already exists in DOM tree, do nothing
        if self.named_id_elements.contains_key(named_id) {
            return;
        }

        let mut old_named_id: Option<String> = None;
        if let Some(node) = self.get_node_by_id_mut(node_id) {
            if node.type_of() != NodeType::Element {
                return;
            }

            old_named_id = node.get_named_id();

            node.set_named_id(named_id);
            self.named_id_elements.insert(named_id.to_owned(), node_id);
        }

        if let Some(old_named_id) = old_named_id {
            self.named_id_elements.remove(&old_named_id);
        }
    }

    pub fn add_new_node(&mut self, node: Node) -> NodeId {
        let mut node_named_id: Option<String> = None;
        if let NodeData::Element(element) = &node.data {
            if let Some(named_id) = element.attributes.get("id") {
                node_named_id = Some(named_id.clone());
            }
        }

        // Register the node if needed
        let node_id = if !node.is_registered {
            self.arena.register_node(node)
        } else {
            node.id
        };

        // TODO: this will also be removed like above note
        if let Some(named_id) = node_named_id {
            self.set_node_named_id(node_id, &named_id);
        }

        // update the node's ID (it uses default ID when first created)
        if let Some(node) = self.get_node_by_id_mut(node_id) {
            if let NodeData::Element(element) = &mut node.data {
                element.set_id(node_id);
            }
        }

        node_id
    }

    /// Inserts a node to the parent node at the given position in the children (or none
    /// to add at the end). Will automatically register the node if not done so already
    pub fn add_node(&mut self, node: Node, parent_id: NodeId, position: Option<usize>) -> NodeId {
        let node_id = self.add_new_node(node);

        self.attach_node_to_parent(node_id, parent_id, position);

        node_id
    }

    /// Relocates a node to another parent node
    pub fn relocate(&mut self, node_id: NodeId, parent_id: NodeId) {
        let node = self.arena.get_node_mut(node_id).unwrap();
        if !node.is_registered {
            panic!("Node is not registered to the arena");
        }

        if node.parent.is_some() && node.parent.unwrap() == parent_id {
            // Nothing to do when we want to relocate to its own parent
            return;
        }

        self.detach_node_from_parent(node_id);
        self.attach_node_to_parent(node_id, parent_id, None);
    }

    /// Adds the node as a child the parent node. If position is given, it will be inserted as a
    /// child at that given position
    pub fn attach_node_to_parent(
        &mut self,
        node_id: NodeId,
        parent_id: NodeId,
        position: Option<usize>,
    ) -> bool {
        //check if any children of node have parent as child
        if parent_id == node_id || self.has_cyclic_reference(node_id, parent_id) {
            return false;
        }

        if let Some(parent_node) = self.get_node_by_id_mut(parent_id) {
            // Make sure position can never be larger than the number of children in the parent
            if let Some(mut position) = position {
                if position > parent_node.children.len() {
                    position = parent_node.children.len();
                }
                parent_node.children.insert(position, node_id);
            } else {
                // No position given, add to end of the children list
                parent_node.children.push(node_id);
            }
        }

        let node = self.arena.get_node_mut(node_id).unwrap();
        node.parent = Some(parent_id);

        true
    }

    /// Separates the given node from its parent node (if any)
    pub fn detach_node_from_parent(&mut self, node_id: NodeId) {
        let parent = self.get_node_by_id(node_id).expect("node not found").parent;

        if let Some(parent_id) = parent {
            let parent_node = self
                .get_node_by_id_mut(parent_id)
                .expect("parent node not found");
            parent_node.children.retain(|&id| id != node_id);

            let node = self.get_node_by_id_mut(node_id).expect("node not found");
            node.parent = None;
        }
    }

    /// returns the root node
    pub fn get_root(&self) -> &Node {
        self.arena
            .get_node(NodeId::root())
            .expect("Root node not found !?")
    }

    /// Returns true when the given parent_id is a child of the node_id
    pub fn has_cyclic_reference(&self, node_id: NodeId, parent_id: NodeId) -> bool {
        has_child_recursive(&self.arena, node_id, parent_id)
    }
}

/// Returns true when the parent node has the child node as a child, or if any of the children of
/// the parent node have the child node as a child.
fn has_child_recursive(arena: &NodeArena, parent_id: NodeId, child_id: NodeId) -> bool {
    let node = arena.get_node(parent_id).cloned();
    if node.is_none() {
        return false;
    }

    let node = node.unwrap();
    for id in node.children.iter() {
        if *id == child_id {
            return true;
        }
        let child = arena.get_node(*id).cloned();
        if has_child(arena, child, child_id) {
            return true;
        }
    }
    false
}

fn has_child(arena: &NodeArena, parent: Option<Node>, child_id: NodeId) -> bool {
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
        let node = arena.get_node(id).cloned();
        if has_child(arena, node, child_id) {
            return true;
        }
    }

    false
}

impl Document {
    /// Print a node and all its children in a tree-like structure
    pub fn print_tree(&self, node: &Node, prefix: String, last: bool, f: &mut fmt::Formatter) {
        let mut buffer = prefix.clone();
        if last {
            buffer.push_str("└─ ");
        } else {
            buffer.push_str("├─ ");
        }

        // buffer.push_str(&format!("({:?}) ", node.id.as_usize()));

        match &node.data {
            NodeData::Document(_) => {
                _ = writeln!(f, "{}Document", buffer);
            }
            NodeData::Text(TextData { value, .. }) => {
                _ = writeln!(f, "{}\"{}\"", buffer, value);
            }
            NodeData::Comment(CommentData { value, .. }) => {
                _ = writeln!(f, "{}<!-- {} -->", buffer, value);
            }
            NodeData::Element(element) => {
                _ = write!(f, "{}<{}", buffer, element.name);
                for (key, value) in element.attributes.iter() {
                    _ = write!(f, " {}={}", key, value);
                }
                _ = writeln!(f, ">");
            }
        }

        if prefix.len() > 40 {
            _ = writeln!(f, "...");
            return;
        }

        let mut buffer = prefix;
        if last {
            buffer.push_str("   ");
        } else {
            buffer.push_str("│  ");
        }

        let len = node.children.len();
        for (i, child) in node.children.iter().enumerate() {
            let child = self.arena.get_node(*child).expect("Child not found");
            self.print_tree(child, buffer.clone(), i == len - 1, f);
        }
    }
}

impl fmt::Display for Document {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.print_tree(self.get_root(), "".to_string(), true, f);
        Ok(())
    }
}

#[derive(Debug)]
pub struct DocumentHandle(Rc<RefCell<Document>>);

impl Display for DocumentHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.borrow())
    }
}

impl PartialEq for DocumentHandle {
    fn eq(&self, other: &Self) -> bool {
        self.0.eq(&other.0)
    }
}

// NOTE: it is preferred to use Document::clone() when
// copying a DocumentHandle reference. However, for
// any structs using this handle that use #[derive(Clone)],
// this implementation is required.
impl Clone for DocumentHandle {
    fn clone(&self) -> DocumentHandle {
        DocumentHandle(Rc::clone(&self.0))
    }
}

impl Eq for DocumentHandle {}

impl DocumentHandle {
    /// Retrieves a immutable reference to the document
    pub fn get(&self) -> impl Deref<Target = Document> + '_ {
        self.0.borrow()
    }

    /// Retrieves a mutable reference to the document
    pub fn get_mut(&mut self) -> impl DerefMut<Target = Document> + '_ {
        self.0.borrow_mut()
    }

    /// Attaches a node to the parent node at the given position in the children (or none
    /// to add at the end).
    pub fn attach_node_to_parent(
        &mut self,
        node_id: NodeId,
        parent_id: NodeId,
        position: Option<usize>,
    ) -> bool {
        self.get_mut()
            .attach_node_to_parent(node_id, parent_id, position)
    }

    /// Separates the given node from its parent node (if any)
    pub fn detach_node_from_parent(&mut self, node_id: NodeId) {
        self.get_mut().detach_node_from_parent(node_id)
    }

    /// Inserts a node to the parent node at the given position in the children (or none
    /// to add at the end). Will automatically register the node if not done so already
    /// Returns the node ID of the inserted node
    pub fn add_node(&mut self, node: Node, parent_id: NodeId, position: Option<usize>) -> NodeId {
        self.get_mut().add_node(node, parent_id, position)
    }

    /// Relocates a node to another parent node
    pub fn relocate(&mut self, node_id: NodeId, parent_id: NodeId) {
        self.get_mut().relocate(node_id, parent_id)
    }

    /// Returns true when there is a cyclic reference from the given node_id to the parent_id
    pub fn has_cyclic_reference(&self, node_id: NodeId, parent_id: NodeId) -> bool {
        self.get().has_cyclic_reference(node_id, parent_id)
    }
}

#[cfg(test)]
mod tests {
    use crate::html5::node::HTML_NAMESPACE;
    use crate::html5::parser::{Document, Node, NodeData, NodeId};
    use std::collections::HashMap;

    #[test]
    fn relocate() {
        let mut document = Document::shared();
        let document_clone = Document::clone(&document);
        document.get_mut().create_root(&document_clone);

        let parent = Node::new_element(&document, "parent", HashMap::new(), HTML_NAMESPACE);
        let node1 = Node::new_element(&document, "div1", HashMap::new(), HTML_NAMESPACE);
        let node2 = Node::new_element(&document, "div2", HashMap::new(), HTML_NAMESPACE);
        let node3 = Node::new_element(&document, "div3", HashMap::new(), HTML_NAMESPACE);
        let node3_1 = Node::new_element(&document, "div3_1", HashMap::new(), HTML_NAMESPACE);

        let parent_id = document.get_mut().add_node(parent, NodeId::from(0), None);
        let node1_id = document.get_mut().add_node(node1, parent_id, None);
        let node2_id = document.get_mut().add_node(node2, parent_id, None);
        let node3_id = document.get_mut().add_node(node3, parent_id, None);
        let node3_1_id = document.get_mut().add_node(node3_1, node3_id, None);

        assert_eq!(
            format!("{}", document),
            r#"└─ Document
   └─ <parent>
      ├─ <div1>
      ├─ <div2>
      └─ <div3>
         └─ <div3_1>
"#
        );

        document.get_mut().relocate(node3_1_id, node1_id);
        assert_eq!(
            format!("{}", document),
            r#"└─ Document
   └─ <parent>
      ├─ <div1>
      │  └─ <div3_1>
      ├─ <div2>
      └─ <div3>
"#
        );

        document.get_mut().relocate(node1_id, node2_id);
        assert_eq!(
            format!("{}", document),
            r#"└─ Document
   └─ <parent>
      ├─ <div2>
      │  └─ <div1>
      │     └─ <div3_1>
      └─ <div3>
"#
        );
    }

    #[test]
    fn set_named_id_to_element() {
        let attributes = HashMap::new();
        let mut document = Document::shared();
        let node = Node::new_element(&document, "div", attributes.clone(), HTML_NAMESPACE);
        let node_id = NodeId::from(0);
        let _ = document.get_mut().add_node(node, node_id, None);
        // invalid name (empty)
        document.get_mut().set_node_named_id(node_id, "");
        assert!(!document
            .get()
            .get_node_by_id(node_id)
            .unwrap()
            .has_named_id());
        // invalid name (spaces)
        document.get_mut().set_node_named_id(node_id, "my id");
        assert!(!document
            .get()
            .get_node_by_id(node_id)
            .unwrap()
            .has_named_id());
        // invalid name (no characters)
        document.get_mut().set_node_named_id(node_id, "123");
        assert!(!document
            .get()
            .get_node_by_id(node_id)
            .unwrap()
            .has_named_id());
        // valid name
        document.get_mut().set_node_named_id(node_id, "myid");
        assert!(document
            .get()
            .get_node_by_id(node_id)
            .unwrap()
            .has_named_id());
        assert_eq!(
            document
                .get()
                .get_node_by_id(node_id)
                .unwrap()
                .get_named_id(),
            Some("myid".to_owned())
        );
    }

    #[test]
    fn set_named_id_to_non_element() {
        let mut document = Document::shared();
        let node = Node::new_text(&document, "sample");
        let node_id = NodeId::from(0);
        let _ = document.get_mut().add_node(node, node_id, None);

        // even if this is a valid name, nothing will happen since it's not an Element type
        document.get_mut().set_node_named_id(node_id, "myid");
        assert!(!document
            .get()
            .get_node_by_id(node_id)
            .unwrap()
            .has_named_id());
    }

    #[test]
    fn duplicate_named_id_elements() {
        let attributes = HashMap::new();

        let mut document = Document::shared();

        let mut node1 = Node::new_element(&document, "div", attributes.clone(), HTML_NAMESPACE);
        let mut node2 = Node::new_element(&document, "div", attributes.clone(), HTML_NAMESPACE);

        match &mut node1.data {
            NodeData::Element(element) => {
                element.attributes.insert("id", "myid");
            }
            _ => panic!(),
        }

        match &mut node2.data {
            NodeData::Element(element) => {
                element.attributes.insert("id", "myid");
            }
            _ => panic!(),
        }

        let _ = document.get_mut().add_node(node1, NodeId::from(0), None);
        let _ = document.get_mut().add_node(node2, NodeId::from(1), None);

        // two elements here have the same ID, the ID will only be tied to NodeId(0) since
        // the HTML5 spec specifies that every ID must uniquely specify one element in the DOM
        // and we inserted NodeId(0) first
        assert_eq!(
            document.get().get_node_by_named_id("myid").unwrap().id,
            NodeId::from(0)
        );

        // however, with that in mind, NodeId(1) will still have id="myid" on the Node itself,
        // but it is not searchable in the DOM. Even if you change the id of NodeId(0), NodeId(1)
        // will still NOT be searchable under get_node_by_named_id. This behaviour can be changed
        // by using a stack/vector/queue/whatever in the HashMap, but since the spec states
        // there should be one unique ID per element, I don't think we should support it
        document
            .get_mut()
            .set_node_named_id(NodeId::from(0), "otherid");
        assert!(document.get().get_node_by_named_id("myid").is_none());
    }

    #[test]
    fn verify_node_ids_in_element_data() {
        let mut document = Document::shared();
        let document_clone = Document::clone(&document);
        document.get_mut().create_root(&document_clone);

        let node1 = Node::new_element(&document, "div", HashMap::new(), HTML_NAMESPACE);
        let node2 = Node::new_element(&document, "div", HashMap::new(), HTML_NAMESPACE);

        document.get_mut().add_node(node1, NodeId::from(0), None);
        document.get_mut().add_node(node2, NodeId::from(0), None);

        let doc_ptr = document.get();

        let get_node1 = doc_ptr.get_node_by_id(NodeId::from(1)).unwrap();
        let get_node2 = doc_ptr.get_node_by_id(NodeId::from(2)).unwrap();

        let NodeData::Element(element1) = &get_node1.data else {
            panic!()
        };

        assert_eq!(element1.node_id, NodeId::from(1));

        let NodeData::Element(element2) = &get_node2.data else {
            panic!()
        };

        assert_eq!(element2.node_id, NodeId::from(2));
    }
}
