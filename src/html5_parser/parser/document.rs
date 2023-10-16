use crate::html5_parser::node::arena::NodeArena;
use crate::html5_parser::node::data::{comment::CommentData, text::TextData};
use crate::html5_parser::node::NodeType;
use crate::html5_parser::node::{Node, NodeData, NodeId};
use crate::html5_parser::node::{NodeTrait, HTML_NAMESPACE};
use crate::html5_parser::parser::quirks::QuirksMode;
use alloc::rc::Rc;
use core::fmt;
use core::fmt::Debug;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt::Display;
use std::ops::{Deref, DerefMut};

#[derive(PartialEq, Debug, Copy, Clone)]
pub enum DocumentType {
    HTML,
    IframeSrcDoc,
}

#[derive(PartialEq)]
pub struct DocumentFragment {
    // Node elements inside this fragment
    arena: NodeArena,
    // Document contents owner
    pub doc: DocumentHandle,
    // Host node
    host: NodeId,
}

impl Clone for DocumentFragment {
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
    pub(crate) fn new(doc: DocumentHandle, host: NodeId) -> Self {
        Self {
            arena: NodeArena::new(),
            doc,
            host,
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct Document {
    arena: NodeArena,
    named_id_elements: HashMap<String, NodeId>, // HTML elements with ID (e.g., <div id="myid">)
    pub doctype: DocumentType,                  // Document type
    pub quirks_mode: QuirksMode,                // Quirks mode
}

impl Default for Document {
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
    // Creates a new document
    pub fn new() -> Self {
        let arena = NodeArena::new();
        Self {
            arena,
            named_id_elements: HashMap::new(),
            doctype: DocumentType::HTML,
            quirks_mode: QuirksMode::NoQuirks,
        }
    }

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

    /// Create DOCUMENT root node
    pub fn create_root(&mut self, document: &DocumentHandle) {
        // previously this used to be in the constructor, but now that
        // we require a document pointer with every node creation, this
        // was separated.

        self.arena.add_node(Node::new_document(document));
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

    // Insert node to the parent node at the given position in the children (or none to add at the end)
    pub fn insert_node(
        &mut self,
        node: Node,
        parent_id: NodeId,
        position: Option<usize>,
    ) -> NodeId {
        let mut node_named_id: Option<String> = None;
        if let NodeData::Element(element) = &node.data {
            if let Some(named_id) = element.attributes.get("id") {
                node_named_id = Some(named_id.clone());
            }
        }

        let node_id = self.arena.add_node(node);

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
        self.arena.attach_node(parent_id, node_id, position);

        node_id
    }

    // Add node to the parent node at the end of its current children
    pub fn add_node(&mut self, node: Node, parent_id: NodeId) -> NodeId {
        self.insert_node(node, parent_id, None)
    }

    /// Insert a node at position in the children of parent_id
    pub fn insert(&mut self, node_id: NodeId, parent_id: NodeId, position: Option<usize>) {
        self.arena.attach_node(parent_id, node_id, position);
    }

    // Append node directly at the end of the children of parent_id
    pub fn append(&mut self, node_id: NodeId, parent_id: NodeId) {
        self.arena.attach_node(parent_id, node_id, None);
    }

    pub fn relocate(&mut self, node_id: NodeId, parent_id: NodeId) {
        // Remove the node from its current parent (if any)
        let cur_parent_id = self.arena.get_node(node_id).expect("node not found").parent;
        if let Some(parent_node_id) = cur_parent_id {
            let cur_parent = self
                .arena
                .get_node_mut(parent_node_id)
                .expect("node not found");
            cur_parent.children.retain(|&x| x != node_id);
        }

        // Add the node to the new parent as a child, and update the node's parent
        self.arena
            .get_node_mut(parent_id)
            .unwrap()
            .children
            .push(node_id);
        self.arena.get_node_mut(node_id).unwrap().parent = Some(parent_id);
    }

    /// return the root node
    pub fn get_root(&self) -> &Node {
        self.arena
            .get_node(NodeId::root())
            .expect("Root node not found !?")
    }
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
    pub fn get(&self) -> impl Deref<Target = Document> + '_ {
        self.0.borrow()
    }

    pub fn get_mut(&mut self) -> impl DerefMut<Target = Document> + '_ {
        self.0.borrow_mut()
    }

    fn add_element(&mut self, parent_id: NodeId, name: &str) -> NodeId {
        let node = Node::new_element(self, name, HashMap::new(), HTML_NAMESPACE);
        self.get_mut().add_node(node, parent_id)
    }

    pub fn add_node_before(
        &mut self,
        node: Node,
        parent_id: NodeId,
        child_position: Option<usize>,
    ) -> NodeId {
        self.get_mut().insert_node(node, parent_id, child_position)
    }

    pub fn add_node(&mut self, node: Node, parent_id: NodeId) -> NodeId {
        self.get_mut().add_node(node, parent_id)
    }

    fn add_text(&mut self, parent_id: NodeId, text: &str) -> NodeId {
        let node = Node::new_text(self, text);
        self.get_mut().add_node(node, parent_id)
    }
}

#[cfg(test)]
mod tests {
    use crate::html5_parser::node::HTML_NAMESPACE;
    use crate::html5_parser::parser::{Document, Node, NodeData, NodeId};
    use std::collections::HashMap;

    #[ignore]
    #[test]
    fn test_document() {
        let mut document = Document::shared();
        let root_id = document.get().get_root().id;
        let html_id = document.add_element(root_id, "html");
        let head_id = document.add_element(html_id, "head");
        let body_id = document.add_element(html_id, "body");
        let title_id = document.add_element(head_id, "title");
        let title_text_id = document.add_text(title_id, "Hello world");
        let p_id = document.add_element(body_id, "p");
        let p_text_id = document.add_text(p_id, "This is a paragraph");
        let p_comment_id = document.add_text(p_id, "This is a comment");
        let p_text2_id = document.add_text(p_id, "This is another paragraph");
        let p_text3_id = document.add_text(p_id, "This is a third paragraph");
        let p_text4_id = document.add_text(p_id, "This is a fourth paragraph");
        let p_text5_id = document.add_text(p_id, "This is a fifth paragraph");
        let p_text6_id = document.add_text(p_id, "This is a sixth paragraph");
        let p_text7_id = document.add_text(p_id, "This is a seventh paragraph");
        let p_text8_id = document.add_text(p_id, "This is a eighth paragraph");
        let p_text9_id = document.add_text(p_id, "This is a ninth paragraph");

        document.get_mut().append(p_text9_id, p_id);
        document.get_mut().append(p_text8_id, p_id);
        document.get_mut().append(p_text7_id, p_id);
        document.get_mut().append(p_text6_id, p_id);
        document.get_mut().append(p_text5_id, p_id);
        document.get_mut().append(p_text4_id, p_id);
        document.get_mut().append(p_text3_id, p_id);
        document.get_mut().append(p_text2_id, p_id);
        document.get_mut().append(p_comment_id, p_id);
        document.get_mut().append(p_text_id, p_id);
        document.get_mut().append(p_id, body_id);
        document.get_mut().append(title_text_id, title_id);
        document.get_mut().append(title_id, head_id);
        document.get_mut().append(head_id, html_id);
        document.get_mut().append(body_id, html_id);
        document.get_mut().append(html_id, root_id);

        assert_eq!(
            format!("{}", document),
            r#"Document
    └─ <html>
    └─ <head>
        └─ <title>
        └─ Hello world
    └─ <body>
        └─ <p>
        └─ This is a paragraph
        └─ <!-- This is a comment -->
        └─ This is another paragraph
        └─ This is a third paragraph
        └─ This is a fourth paragraph
        └─ This is a fifth paragraph
        └─ This is a sixth paragraph
        └─ This is a seventh paragraph
        └─ This is a eighth paragraph
        └─ This is a ninth paragraph
        "#
        );
    }

    #[test]
    fn set_named_id_to_element() {
        let attributes = HashMap::new();
        let mut document = Document::shared();
        let node = Node::new_element(&document, "div", attributes.clone(), HTML_NAMESPACE);
        let node_id = NodeId::from(0);
        let _ = document.get_mut().add_node(node, node_id);
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
        let _ = document.get_mut().add_node(node, node_id);

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

        let _ = document.get_mut().add_node(node1, NodeId::from(0));
        let _ = document.get_mut().add_node(node2, NodeId::from(1));

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

        document.get_mut().add_node(node1, NodeId::from(0));
        document.get_mut().add_node(node2, NodeId::from(0));

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
