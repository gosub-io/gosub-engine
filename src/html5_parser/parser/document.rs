use crate::html5_parser::node::NodeTrait;
use crate::html5_parser::node::NodeType;
use crate::html5_parser::node::{Node, NodeData, NodeId};
use crate::html5_parser::node_arena::NodeArena;
use crate::html5_parser::parser::quirks::QuirksMode;
use crate::html5_parser::parser::HashMap;
use std::fmt;

#[derive(PartialEq, Debug, Copy, Clone)]
pub enum DocumentType {
    HTML,
    IframeSrcDoc,
}

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
        let mut arena = NodeArena::new();
        arena.add_node(Node::new_document());
        Self {
            arena,
            named_id_elements: HashMap::new(),
            doctype: DocumentType::HTML,
            quirks_mode: QuirksMode::NoQuirks,
        }
    }

    /// Fetches a node by id or returns None when no node with this ID is found
    pub fn get_node_by_id(&self, node_id: NodeId) -> Option<&Node> {
        self.arena.get_node(node_id)
    }

    /// Fetches a mutable node by id or returns None when no node with this ID is found
    pub fn get_mut_node_by_id(&mut self, node_id: NodeId) -> Option<&mut Node> {
        self.arena.get_mut_node(node_id)
    }

    /// Fetches a node by named id (string) or returns None when no node with this ID is found
    pub fn get_node_by_named_id(&self, named_id: &str) -> Option<&Node> {
        let node_id = self.named_id_elements.get(named_id)?;
        self.arena.get_node(*node_id)
    }

    /// Fetches a mutable node by named id (string) or returns None when no node with this ID is found
    pub fn get_mut_node_by_named_id(&mut self, named_id: &str) -> Option<&mut Node> {
        let node_id = self.named_id_elements.get(named_id)?;
        self.arena.get_mut_node(*node_id)
    }

    // according to HTML5 spec: 3.2.3.1
    // https://www.w3.org/TR/2011/WD-html5-20110405/elements.html#the-id-attribute
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
        if let Some(node) = self.get_mut_node_by_id(node_id) {
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
        if let Ok(Some(named_id)) = node.get_attribute("id") {
            node_named_id = Some(named_id.clone());
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

    // // append a node to another parent
    // pub fn append(&mut self, node_id: NodeId, parent_id: NodeId) {
    //     self.arena.attach_node(parent_id, node_id);
    // }

    // return the root node
    pub fn get_root(&self) -> &Node {
        self.arena
            .get_node(NodeId::root())
            .expect("Root node not found !?")
    }
}

impl Document {
    /// Print a node and all its children in a tree-like structure
    pub fn print_tree(&self, node: &Node, prefix: String, last: bool, f: &mut fmt::Formatter<'_>) {
        let mut buffer = prefix.clone();
        if last {
            buffer.push_str("└─ ");
        } else {
            buffer.push_str("├─ ");
        }

        match &node.data {
            NodeData::Document => {
                _ = writeln!(f, "{}Document", buffer);
            }
            NodeData::Text { value } => {
                _ = writeln!(f, "{}\"{}\"", buffer, value);
            }
            NodeData::Comment { value } => {
                _ = writeln!(f, "{}<!-- {} -->", buffer, value);
            }
            NodeData::Element { name, attributes } => {
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
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.print_tree(self.get_root(), "".to_string(), true, f);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::html5_parser::node::HTML_NAMESPACE;
    use crate::html5_parser::parser::{Document, Node, NodeId};
    use std::collections::HashMap;

    #[ignore]
    #[test]
    fn test_document() {
        let mut document = super::Document::new();
        let root_id = document.get_root().id;
        let html_id = document.add_node(
            super::Node::new_element("html", HashMap::new(), HTML_NAMESPACE),
            root_id,
        );
        let head_id = document.add_node(
            super::Node::new_element("head", HashMap::new(), HTML_NAMESPACE),
            html_id,
        );
        let body_id = document.add_node(
            super::Node::new_element("body", HashMap::new(), HTML_NAMESPACE),
            html_id,
        );
        let title_id = document.add_node(
            super::Node::new_element("title", HashMap::new(), HTML_NAMESPACE),
            head_id,
        );
        let title_text_id = document.add_node(super::Node::new_text("Hello world"), title_id);
        let p_id = document.add_node(
            super::Node::new_element("p", HashMap::new(), HTML_NAMESPACE),
            body_id,
        );
        let p_text_id = document.add_node(super::Node::new_text("This is a paragraph"), p_id);
        let p_comment_id = document.add_node(super::Node::new_comment("This is a comment"), p_id);
        let p_text2_id =
            document.add_node(super::Node::new_text("This is another paragraph"), p_id);
        let p_text3_id =
            document.add_node(super::Node::new_text("This is a third paragraph"), p_id);
        let p_text4_id =
            document.add_node(super::Node::new_text("This is a fourth paragraph"), p_id);
        let p_text5_id =
            document.add_node(super::Node::new_text("This is a fifth paragraph"), p_id);
        let p_text6_id =
            document.add_node(super::Node::new_text("This is a sixth paragraph"), p_id);
        let p_text7_id =
            document.add_node(super::Node::new_text("This is a seventh paragraph"), p_id);
        let p_text8_id =
            document.add_node(super::Node::new_text("This is a eighth paragraph"), p_id);
        let p_text9_id =
            document.add_node(super::Node::new_text("This is a ninth paragraph"), p_id);

        document.append(p_text9_id, p_id);
        document.append(p_text8_id, p_id);
        document.append(p_text7_id, p_id);
        document.append(p_text6_id, p_id);
        document.append(p_text5_id, p_id);
        document.append(p_text4_id, p_id);
        document.append(p_text3_id, p_id);
        document.append(p_text2_id, p_id);
        document.append(p_comment_id, p_id);
        document.append(p_text_id, p_id);
        document.append(p_id, body_id);
        document.append(title_text_id, title_id);
        document.append(title_id, head_id);
        document.append(head_id, html_id);
        document.append(body_id, html_id);
        document.append(html_id, root_id);

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
        let node = Node::new_element("div", attributes.clone(), HTML_NAMESPACE);
        let mut doc = Document::new();
        let node_id = NodeId(1);
        let _ = doc.add_node(node, node_id);
        // invalid name (empty)
        doc.set_node_named_id(node_id, "");
        assert!(!doc.get_node_by_id(node_id).unwrap().has_named_id());
        // invalid name (spaces)
        doc.set_node_named_id(node_id, "my id");
        assert!(!doc.get_node_by_id(node_id).unwrap().has_named_id());
        // invalid name (no characters)
        doc.set_node_named_id(node_id, "123");
        assert!(!doc.get_node_by_id(node_id).unwrap().has_named_id());
        // valid name
        doc.set_node_named_id(node_id, "myid");
        assert!(doc.get_node_by_id(node_id).unwrap().has_named_id());
        assert_eq!(
            doc.get_node_by_id(node_id).unwrap().get_named_id(),
            Some("myid".to_owned())
        );
    }

    #[test]
    fn set_named_id_to_non_element() {
        let node = Node::new_text("sample");
        let mut doc = Document::new();
        let node_id = NodeId(1);
        let _ = doc.add_node(node, node_id);

        // even if this is a valid name, nothing will happen since it's not an Element type
        doc.set_node_named_id(node_id, "myid");
        assert!(!doc.get_node_by_id(node_id).unwrap().has_named_id());
    }

    #[test]
    fn duplicate_named_id_elements() {
        let attributes = HashMap::new();

        let mut node1 = Node::new_element("div", attributes.clone(), HTML_NAMESPACE);
        let mut node2 = Node::new_element("div", attributes.clone(), HTML_NAMESPACE);

        let _ = node1.insert_attribute("id", "myid");
        let _ = node2.insert_attribute("id", "myid");

        let mut doc = Document::new();
        let _ = doc.add_node(node1, NodeId(1));
        let _ = doc.add_node(node2, NodeId(2));

        // two elements here have the same ID, the ID will only be tied to NodeId(1) since
        // the HTML5 spec specifies that every ID must uniquely specify one element in the DOM
        // and we inserted NodeId(1) first
        assert_eq!(doc.get_node_by_named_id("myid").unwrap().id, NodeId(1));

        // however, with that in mind, NodeId(2) will still have id="myid" on the Node itself,
        // but it is not searchable in the DOM. Even if you change the id of NodeId(1), NodeId(2)
        // will still NOT be searchable under get_node_by_named_id. This behaviour can be changed
        // by using a stack/vector/queue/whatever in the HashMap, but since the spec states
        // there should be one unique ID per element, I don't think we should support it
        doc.set_node_named_id(NodeId(1), "otherid");
        assert!(doc.get_node_by_named_id("myid").is_none());
    }
}
