use crate::html5_parser::node::arena::NodeArena;
use crate::html5_parser::node::data::{comment::CommentData, element::ElementData, text::TextData};
use crate::html5_parser::node::NodeTrait;
use crate::html5_parser::node::NodeType;
use crate::html5_parser::node::{Node, NodeData, NodeId};
use crate::html5_parser::parser::quirks::QuirksMode;
use alloc::rc::Rc;
use core::fmt;
use core::fmt::Debug;
use std::cell::RefCell;
use std::collections::HashMap;

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
    doc: Rc<RefCell<Document>>,
    // Host node
    host: NodeId,
}

impl Clone for DocumentFragment {
    fn clone(&self) -> Self {
        Self {
            arena: self.arena.clone(),
            doc: self.doc.clone(),
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
    pub(crate) fn new(doc: Rc<RefCell<Document>>, host: NodeId) -> Self {
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

impl Document {
    pub(crate) fn print_nodes(&self) {
        self.arena.print_nodes();
    }
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

    /// Create DOCUMENT root node
    pub fn create_root(&mut self, document: &Rc<RefCell<Document>>) {
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

    // Add to the document
    pub fn add_node(&mut self, node: Node, parent_id: NodeId) -> NodeId {
        let mut node_named_id: Option<String> = None;
        if let NodeData::Element(element) = &node.data {
            if let Some(named_id) = element.attributes.get("id") {
                node_named_id = Some(named_id.clone());
            }
        }

        let node_type = node.type_of();
        let node_id = self.arena.add_node(node);
        if node_type == NodeType::Element {
            if let Some(node_named_id) = node_named_id {
                self.set_node_named_id(node_id, &node_named_id);
            }
        }
        self.arena.attach_node(parent_id, node_id);
        node_id
    }

    pub fn append(&mut self, node_id: NodeId, parent_id: NodeId) {
        self.arena.attach_node(parent_id, node_id);
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
            NodeData::Element(ElementData {
                name, attributes, ..
            }) => {
                _ = write!(f, "{}<{}", buffer, name);
                for (key, value) in attributes.iter() {
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

#[cfg(test)]
mod tests {
    use crate::html5_parser::node::HTML_NAMESPACE;
    use crate::html5_parser::parser::{Document, Node, NodeData, NodeId};
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    #[ignore]
    #[test]
    fn test_document() {
        let document = Rc::new(RefCell::new(Document::new()));
        let root_id = document.borrow().get_root().id;
        let html_id = document.borrow_mut().add_node(
            Node::new_element(&document, "html", HashMap::new(), HTML_NAMESPACE),
            root_id,
        );
        let head_id = document.borrow_mut().add_node(
            Node::new_element(&document, "head", HashMap::new(), HTML_NAMESPACE),
            html_id,
        );
        let body_id = document.borrow_mut().add_node(
            Node::new_element(&document, "body", HashMap::new(), HTML_NAMESPACE),
            html_id,
        );
        let title_id = document.borrow_mut().add_node(
            Node::new_element(&document, "title", HashMap::new(), HTML_NAMESPACE),
            head_id,
        );
        let title_text_id = document.borrow_mut().add_node(
            Node::new_text(&document, "Hello world"),
            title_id
        );
        let p_id = document.borrow_mut().add_node(
            Node::new_element(&document, "p", HashMap::new(), HTML_NAMESPACE),
            body_id,
        );
        let p_text_id = document.borrow_mut().add_node(Node::new_text(&document, "This is a paragraph"), p_id);
        let p_comment_id = document.borrow_mut().add_node(Node::new_comment(&document, "This is a comment"), p_id);
        let p_text2_id = document.borrow_mut().add_node(Node::new_text(&document, "This is another paragraph"), p_id);
        let p_text3_id = document.borrow_mut().add_node(Node::new_text(&document, "This is a third paragraph"), p_id);
        let p_text4_id = document.borrow_mut().add_node(Node::new_text(&document, "This is a fourth paragraph"), p_id);
        let p_text5_id = document.borrow_mut().add_node(Node::new_text(&document, "This is a fifth paragraph"), p_id);
        let p_text6_id = document.borrow_mut().add_node(Node::new_text(&document, "This is a sixth paragraph"), p_id);
        let p_text7_id = document.borrow_mut().add_node(Node::new_text(&document, "This is a seventh paragraph"), p_id);
        let p_text8_id = document.borrow_mut().add_node(Node::new_text(&document, "This is a eighth paragraph"), p_id);
        let p_text9_id = document.borrow_mut().add_node(Node::new_text(&document, "This is a ninth paragraph"), p_id);

        document.borrow_mut().append(p_text9_id, p_id);
        document.borrow_mut().append(p_text8_id, p_id);
        document.borrow_mut().append(p_text7_id, p_id);
        document.borrow_mut().append(p_text6_id, p_id);
        document.borrow_mut().append(p_text5_id, p_id);
        document.borrow_mut().append(p_text4_id, p_id);
        document.borrow_mut().append(p_text3_id, p_id);
        document.borrow_mut().append(p_text2_id, p_id);
        document.borrow_mut().append(p_comment_id, p_id);
        document.borrow_mut().append(p_text_id, p_id);
        document.borrow_mut().append(p_id, body_id);
        document.borrow_mut().append(title_text_id, title_id);
        document.borrow_mut().append(title_id, head_id);
        document.borrow_mut().append(head_id, html_id);
        document.borrow_mut().append(body_id, html_id);
        document.borrow_mut().append(html_id, root_id);

        assert_eq!(
            format!("{}", document.borrow()),
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
        let document = Rc::new(RefCell::new(Document::new()));
        let node = Node::new_element(&document, "div", attributes.clone(), HTML_NAMESPACE);
        let node_id = NodeId(0);
        let _ = document.borrow_mut().add_node(node, node_id);
        // invalid name (empty)
        document.borrow_mut().set_node_named_id(node_id, "");
        assert!(!document.borrow().get_node_by_id(node_id).unwrap().has_named_id());
        // invalid name (spaces)
        document.borrow_mut().set_node_named_id(node_id, "my id");
        assert!(!document.borrow().get_node_by_id(node_id).unwrap().has_named_id());
        // invalid name (no characters)
        document.borrow_mut().set_node_named_id(node_id, "123");
        assert!(!document.borrow().get_node_by_id(node_id).unwrap().has_named_id());
        // valid name
        document.borrow_mut().set_node_named_id(node_id, "myid");
        assert!(document.borrow().get_node_by_id(node_id).unwrap().has_named_id());
        assert_eq!(
            document.borrow().get_node_by_id(node_id).unwrap().get_named_id(),
            Some("myid".to_owned())
        );
    }

    #[test]
    fn set_named_id_to_non_element() {
        let document = Rc::new(RefCell::new(Document::new()));
        let node = Node::new_text(&document, "sample");
        let node_id = NodeId(0);
        let _ = document.borrow_mut().add_node(node, node_id);

        // even if this is a valid name, nothing will happen since it's not an Element type
        document.borrow_mut().set_node_named_id(node_id, "myid");
        assert!(!document.borrow().get_node_by_id(node_id).unwrap().has_named_id());
    }

    #[test]
    fn duplicate_named_id_elements() {
        let attributes = HashMap::new();

        let document = Rc::new(RefCell::new(Document::new()));

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

        let _ = document.borrow_mut().add_node(node1, NodeId(0));
        let _ = document.borrow_mut().add_node(node2, NodeId(1));

        // two elements here have the same ID, the ID will only be tied to NodeId(0) since
        // the HTML5 spec specifies that every ID must uniquely specify one element in the DOM
        // and we inserted NodeId(0) first
        assert_eq!(document.borrow().get_node_by_named_id("myid").unwrap().id, NodeId(0));

        // however, with that in mind, NodeId(1) will still have id="myid" on the Node itself,
        // but it is not searchable in the DOM. Even if you change the id of NodeId(0), NodeId(1)
        // will still NOT be searchable under get_node_by_named_id. This behaviour can be changed
        // by using a stack/vector/queue/whatever in the HashMap, but since the spec states
        // there should be one unique ID per element, I don't think we should support it
        document.borrow_mut().set_node_named_id(NodeId(0), "otherid");
        assert!(document.borrow().get_node_by_named_id("myid").is_none());
    }
}
