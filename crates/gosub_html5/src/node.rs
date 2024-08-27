use crate::node::data::comment::CommentData;
use crate::node::data::doctype::DocTypeData;
use crate::node::data::document::DocumentData;
use crate::node::data::element::ElementData;
use crate::node::data::text::TextData;
use crate::parser::document::{Document, DocumentHandle};
use core::fmt::Debug;
use derive_more::Display;
use gosub_shared::byte_stream::Location;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Weak;

pub const HTML_NAMESPACE: &str = "http://www.w3.org/1999/xhtml";
pub const MATHML_NAMESPACE: &str = "http://www.w3.org/1998/Math/MathML";
pub const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";
pub const XLINK_NAMESPACE: &str = "http://www.w3.org/1999/xlink";
pub const XML_NAMESPACE: &str = "http://www.w3.org/XML/1998/namespace";
pub const XMLNS_NAMESPACE: &str = "http://www.w3.org/2000/xmlns/";

pub mod arena;
pub mod data;

/// Different types of nodes
#[derive(Debug, PartialEq)]
pub enum NodeType {
    Document,
    DocType,
    Text,
    Comment,
    Element,
}

/// Different type of node data
#[derive(Debug, Clone, PartialEq)]
pub enum NodeData {
    /// Represents a document
    Document(DocumentData),
    // Represents a doctype
    DocType(DocTypeData),
    /// Represents a text
    Text(TextData),
    /// Represents a comment
    Comment(CommentData),
    /// Represents an element
    Element(Box<ElementData>),
}

/// Id used to identify a node
#[derive(Clone, Copy, Debug, Default, Display, Eq, Hash, PartialEq, PartialOrd)]
pub struct NodeId(pub(crate) usize);

impl From<NodeId> for usize {
    /// Converts a NodeId into a usize
    fn from(value: NodeId) -> Self {
        value.0
    }
}

impl From<usize> for NodeId {
    /// Converts a usize into a NodeId
    fn from(value: usize) -> Self {
        Self(value)
    }
}

impl From<u64> for NodeId {
    /// Converts a u64 into a NodeId
    fn from(value: u64) -> Self {
        Self(value as usize)
    }
}

impl From<NodeId> for u64 {
    /// Converts a NodeId into a u64
    fn from(value: NodeId) -> Self {
        value.0 as u64
    }
}

impl Default for &NodeId {
    /// Returns the default NodeId, which is 0
    fn default() -> Self {
        &NodeId(0)
    }
}

impl NodeId {
    // TODO: Drop Default derive and only use 0 for the root, or choose another id for the root
    pub const ROOT_NODE: usize = 0;

    /// Returns the root node ID
    pub fn root() -> Self {
        Self(Self::ROOT_NODE)
    }

    /// Returns true when this nodeId is the root node
    pub fn is_root(&self) -> bool {
        self.0 == Self::ROOT_NODE
    }

    /// Returns the next node ID
    #[must_use]
    pub fn next(&self) -> Self {
        if self.0 == usize::MAX {
            return Self(usize::MAX);
        }

        Self(self.0 + 1)
    }

    /// Returns the nodeID as usize
    pub fn as_usize(&self) -> usize {
        self.0
    }

    /// Returns the previous node ID
    #[must_use]
    pub fn prev(&self) -> Self {
        if self.0 == 0 {
            return Self::root();
        }

        Self(self.0 - 1)
    }
}

/// Node structure that resembles a DOM node
pub struct Node {
    /// ID of the node, 0 is always the root / document node
    pub id: NodeId,
    /// parent of the node, if any
    pub parent: Option<NodeId>,
    /// children of the node
    pub children: Vec<NodeId>,
    /// name of the node, or empty when it's not a tag
    pub name: String,
    /// namespace of the node
    pub namespace: Option<String>,
    /// actual data of the node
    pub data: NodeData,
    /// weak pointer to document this node is attached to
    pub document: Weak<RefCell<Document>>,
    // Returns true when the given node is registered into an arena
    pub is_registered: bool,
    // Location of the node in the source code
    pub location: Location,
}

impl Node {
    pub fn is_root(&self) -> bool {
        self.id.is_root()
    }
}

impl PartialEq for Node {
    fn eq(&self, other: &Node) -> bool {
        self.id == other.id
    }
}

impl Debug for Node {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug = f.debug_struct("Node");
        debug.field("id", &self.id);
        debug.field("parent", &self.parent);
        debug.field("children", &self.children);
        debug.field("name", &self.name);
        match &self.namespace {
            Some(namespace) if namespace == HTML_NAMESPACE => debug.field("namespace", &"HTML"),
            Some(namespace) if namespace == XML_NAMESPACE => debug.field("namespace", &"XML"),
            Some(namespace) if namespace == XMLNS_NAMESPACE => debug.field("namespace", &"XMLNS"),
            Some(namespace) if namespace == MATHML_NAMESPACE => debug.field("namespace", &"MATHML"),
            Some(namespace) if namespace == SVG_NAMESPACE => debug.field("namespace", &"SVG"),
            Some(namespace) if namespace == XLINK_NAMESPACE => debug.field("namespace", &"XLINK"),
            None => debug.field("namespace", &"None"),
            _ => debug.field("namespace", &"unknown"),
        };
        debug.field("data", &self.data);
        debug.finish_non_exhaustive()
    }
}

impl Clone for Node {
    fn clone(&self) -> Self {
        Node {
            id: self.id,
            parent: self.parent,
            children: self.children.clone(),
            name: self.name.clone(),
            namespace: self.namespace.clone(),
            data: self.data.clone(),
            document: Weak::clone(&self.document),
            is_registered: self.is_registered,
            location: self.location.clone(),
        }
    }
}

impl Node {
    /// create a new `Node`
    #[must_use]
    pub fn new(data: NodeData, document: &DocumentHandle, location: Location) -> Self {
        let (id, parent, children, name, namespace, is_registered) = <_>::default();
        Self {
            id,
            parent,
            children,
            data,
            name,
            namespace,
            document: document.to_weak(),
            is_registered,
            location,
        }
    }

    /// Create a new document node
    #[must_use]
    pub fn new_document(document: &DocumentHandle, location: Location) -> Self {
        Self::new(NodeData::Document(DocumentData::new()), document, location)
    }

    #[must_use]
    pub fn new_doctype(
        document: &DocumentHandle,
        name: &str,
        pub_identifier: &str,
        sys_identifier: &str,
        loc: Location,
    ) -> Self {
        Self::new(
            NodeData::DocType(DocTypeData::new(name, pub_identifier, sys_identifier)),
            document,
            loc,
        )
    }

    /// Create a new element node with the given name and attributes and namespace
    #[must_use]
    pub fn new_element(
        document: &DocumentHandle,
        name: &str,
        attributes: HashMap<String, String>,
        namespace: &str,
        location: Location,
    ) -> Self {
        Self {
            name: name.to_owned(),
            namespace: Some(namespace.into()),
            ..Self::new(
                NodeData::Element(Box::new(ElementData::with_name_and_attributes(
                    NodeId::default(),
                    name,
                    attributes,
                ))),
                document,
                location,
            )
        }
    }

    /// Creates a new comment node
    #[must_use]
    pub fn new_comment(document: &DocumentHandle, location: Location, value: &str) -> Self {
        Self::new(
            NodeData::Comment(CommentData::with_value(value)),
            document,
            location,
        )
    }

    /// Creates a new text node
    #[must_use]
    pub fn new_text(document: &DocumentHandle, location: Location, value: &str) -> Self {
        Self::new(
            NodeData::Text(TextData::with_value(value)),
            document,
            location,
        )
    }

    /// Returns true if the given node is a "formatting" node
    pub fn is_formatting(&self) -> bool {
        self.namespace == Some(HTML_NAMESPACE.into())
            && FORMATTING_HTML_ELEMENTS.contains(&self.name.as_str())
    }

    /// Returns true if the given node is "special" node based on the namespace and name
    pub fn is_special(&self) -> bool {
        if self.namespace == Some(HTML_NAMESPACE.into())
            && SPECIAL_HTML_ELEMENTS.contains(&self.name.as_str())
        {
            return true;
        }
        if self.namespace == Some(MATHML_NAMESPACE.into())
            && SPECIAL_MATHML_ELEMENTS.contains(&self.name.as_str())
        {
            return true;
        }
        if self.namespace == Some(SVG_NAMESPACE.into())
            && SPECIAL_SVG_ELEMENTS.contains(&self.name.as_str())
        {
            return true;
        }

        false
    }

    /// Returns true if this node is registered into an arena
    pub fn is_registered(&self) -> bool {
        self.is_registered
    }

    /// This will only compare against the tag, namespace and data same except element data.
    /// for element data compaare against the tag, namespace and attributes without order.
    /// Both nodes could still have other parents and children.
    pub fn matches_tag_and_attrs_without_order(&self, other: &Self) -> bool {
        if self.name != other.name || self.namespace != other.namespace {
            return false;
        }

        if self.type_of() != other.type_of() {
            return false;
        }

        match self.type_of() {
            NodeType::Element => {
                let mut self_attributes = None;
                let mut other_attributes = None;
                if let NodeData::Element(element) = &self.data {
                    self_attributes = Some(element.attributes.clone());
                }
                if let NodeData::Element(element) = &other.data {
                    other_attributes = Some(element.attributes.clone());
                }
                self_attributes.eq(&other_attributes)
            }
            _ => self.data == other.data,
        }
    }

    /// Returns true when the given node is of the given namespace
    pub fn is_namespace(&self, namespace: &str) -> bool {
        self.namespace == Some(namespace.into())
    }

    /// Returns true if the given node is a html integration point
    /// See: https://html.spec.whatwg.org/multipage/parsing.html#html-integration-point
    pub(crate) fn is_html_integration_point(&self) -> bool {
        let namespace = self.namespace.clone().unwrap_or_default();

        if namespace == MATHML_NAMESPACE && self.name == "annotation-xml" {
            if let NodeData::Element(element) = &self.data {
                if let Some(value) = element.attributes.get("encoding") {
                    if value.eq_ignore_ascii_case("text/html") {
                        return true;
                    }
                    if value.eq_ignore_ascii_case("application/xhtml+xml") {
                        return true;
                    }
                }
            }
        }

        namespace == SVG_NAMESPACE
            && ["foreignObject", "desc", "title"].contains(&self.name.as_str())
    }

    /// Returns true if the given node is a mathml integration point
    /// See: https://html.spec.whatwg.org/multipage/parsing.html#mathml-text-integration-point
    pub(crate) fn is_mathml_integration_point(&self) -> bool {
        let namespace = self.namespace.clone().unwrap_or_default();

        namespace == MATHML_NAMESPACE
            && ["mi", "mo", "mn", "ms", "mtext"].contains(&self.name.as_str())
    }

    /// Returns true if the node is an element node
    pub fn is_element(&self) -> bool {
        if let NodeData::Element(_) = &self.data {
            return true;
        }

        false
    }

    pub fn is_text(&self) -> bool {
        if let NodeData::Text(_) = &self.data {
            return true;
        }

        false
    }

    pub fn as_text(&self) -> &TextData {
        if let NodeData::Text(text) = &self.data {
            return text;
        }

        panic!("Node is not a text");
    }

    pub fn as_element(&self) -> &ElementData {
        if let NodeData::Element(element) = &self.data {
            return element;
        }

        panic!("Node is not an element");
    }

    pub fn as_element_mut(&mut self) -> &mut ElementData {
        if let NodeData::Element(ref mut element) = self.data {
            element
        } else {
            panic!("Node is not an element");
        }
    }

    /// Returns true when the given attribute has been set on the node
    pub fn has_attribute(&self, name: &str) -> bool {
        if let NodeData::Element(element) = &self.data {
            return element.attributes.contains_key(name);
        }

        false
    }

    /// Returns the given attribute value or None when the attribute is not found
    pub fn get_attribute(&self, name: &str) -> Option<&String> {
        if let NodeData::Element(element) = &self.data {
            return element.attributes.get(name);
        }

        None
    }
}

pub trait NodeTrait {
    /// Returns the token type of the given token
    fn type_of(&self) -> NodeType;
}

// Each node implements the NodeTrait and has a type_of that will return the node type.
impl NodeTrait for Node {
    /// Returns the token type of the given token
    fn type_of(&self) -> NodeType {
        match self.data {
            NodeData::Document { .. } => NodeType::Document,
            NodeData::DocType { .. } => NodeType::DocType,
            NodeData::Text { .. } => NodeType::Text,
            NodeData::Comment { .. } => NodeType::Comment,
            NodeData::Element { .. } => NodeType::Element,
        }
    }
}

/// HTML elements that are considered formatting elements
pub static FORMATTING_HTML_ELEMENTS: [&str; 14] = [
    "a", "b", "big", "code", "em", "font", "i", "nobr", "s", "small", "strike", "strong", "tt", "u",
];

/// HTML elements that are considered special elements
pub static SPECIAL_HTML_ELEMENTS: [&str; 83] = [
    "address",
    "applet",
    "area",
    "article",
    "aside",
    "base",
    "basefont",
    "bgsound",
    "blockquote",
    "body",
    "br",
    "button",
    "caption",
    "center",
    "col",
    "colgroup",
    "dd",
    "details",
    "dir",
    "div",
    "dl",
    "dt",
    "embed",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "frame",
    "frameset",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "head",
    "header",
    "hgroup",
    "hr",
    "html",
    "iframe",
    "img",
    "input",
    "keygen",
    "li",
    "link",
    "listing",
    "main",
    "marquee",
    "menu",
    "meta",
    "nav",
    "noembed",
    "noframes",
    "noscript",
    "object",
    "ol",
    "p",
    "param",
    "plaintext",
    "pre",
    "script",
    "search",
    "section",
    "select",
    "source",
    "style",
    "summary",
    "table",
    "tbody",
    "td",
    "template",
    "textarea",
    "tfoot",
    "th",
    "thead",
    "title",
    "tr",
    "track",
    "ul",
    "wbr",
    "xmp",
];

/// MathML elements that are considered special elements
pub static SPECIAL_MATHML_ELEMENTS: [&str; 6] = ["mi", "mo", "mn", "ms", "mtext", "annotation-xml"];

/// SVG elements that are considered special elements
pub static SPECIAL_SVG_ELEMENTS: [&str; 3] = ["foreignObject", "desc", "title"];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::document::Document;

    #[test]
    fn new_document() {
        let document = Document::shared(None);
        let node = Node::new_document(&document, Location::default());
        assert_eq!(node.id, NodeId::default());
        assert_eq!(node.parent, None);
        assert!(node.children.is_empty());
        assert_eq!(node.name, "".to_string());
        assert_eq!(node.namespace, None);
        match &node.data {
            NodeData::Document(_) => (),
            _ => panic!(),
        }
    }

    #[test]
    fn new_element() {
        let mut attributes = HashMap::new();
        attributes.insert("id".to_string(), "test".to_string());
        let document = Document::shared(None);
        let node = Node::new_element(
            &document,
            "div",
            attributes.clone(),
            HTML_NAMESPACE,
            Location::default(),
        );
        assert_eq!(node.id, NodeId::default());
        assert_eq!(node.parent, None);
        assert!(node.children.is_empty());
        assert_eq!(node.name, "div".to_string());
        assert_eq!(node.namespace, Some(HTML_NAMESPACE.into()));
        let NodeData::Element(element) = &node.data else {
            panic!()
        };
        assert_eq!(element.name, "div");
        assert!(element.attributes.contains_key("id"));
        assert_eq!(element.attributes.get("id").unwrap(), "test");
    }

    #[test]
    fn new_comment() {
        let document = Document::shared(None);
        let node = Node::new_comment(&document, Location::default(), "test");
        assert_eq!(node.id, NodeId::default());
        assert_eq!(node.parent, None);
        assert!(node.children.is_empty());
        assert_eq!(node.name, "".to_string());
        assert_eq!(node.namespace, None);
        let NodeData::Comment(CommentData { value, .. }) = &node.data else {
            panic!()
        };
        assert_eq!(value, "test");
    }

    #[test]
    fn new_text() {
        let document = Document::shared(None);
        let node = Node::new_text(&document, Location::default(), "test");
        assert_eq!(node.id, NodeId::default());
        assert_eq!(node.parent, None);
        assert!(node.children.is_empty());
        assert_eq!(node.name, "".to_string());
        assert_eq!(node.namespace, None);
        let NodeData::Text(TextData { value }) = &node.data else {
            panic!()
        };
        assert_eq!(value, "test");
    }

    #[test]
    fn is_special() {
        let mut attributes = HashMap::new();
        attributes.insert("id".to_string(), "test".to_string());
        let document = Document::shared(None);
        let node = Node::new_element(
            &document,
            "div",
            attributes,
            HTML_NAMESPACE,
            Location::default(),
        );
        assert!(node.is_special());
    }

    #[test]
    fn type_of() {
        let document = Document::shared(None);
        let node = Node::new_document(&document, Location::default());
        assert_eq!(node.type_of(), NodeType::Document);
        let node = Node::new_text(&document, Location::default(), "test");
        assert_eq!(node.type_of(), NodeType::Text);
        let node = Node::new_comment(&document, Location::default(), "test");
        assert_eq!(node.type_of(), NodeType::Comment);
        let mut attributes = HashMap::new();
        attributes.insert("id".to_string(), "test".to_string());
        let node = Node::new_element(
            &document,
            "div",
            attributes,
            HTML_NAMESPACE,
            Location::default(),
        );
        assert_eq!(node.type_of(), NodeType::Element);
    }

    #[test]
    fn special_html_elements() {
        let document = Document::shared(None);
        for element in SPECIAL_HTML_ELEMENTS.iter() {
            let mut attributes = HashMap::new();
            attributes.insert("id".to_string(), "test".to_string());
            let node = Node::new_element(
                &document,
                element,
                attributes,
                HTML_NAMESPACE,
                Location::default(),
            );
            assert!(node.is_special());
        }
    }

    #[test]
    fn special_mathml_elements() {
        let document = Document::shared(None);
        for element in SPECIAL_MATHML_ELEMENTS.iter() {
            let mut attributes = HashMap::new();
            attributes.insert("id".to_string(), "test".to_string());
            let node = Node::new_element(
                &document,
                element,
                attributes,
                MATHML_NAMESPACE,
                Location::default(),
            );
            assert!(node.is_special());
        }
    }

    #[test]
    fn special_svg_elements() {
        let document = Document::shared(None);
        for element in SPECIAL_SVG_ELEMENTS.iter() {
            let mut attributes = HashMap::new();
            attributes.insert("id".to_string(), "test".to_string());
            let node = Node::new_element(
                &document,
                element,
                attributes,
                SVG_NAMESPACE,
                Location::default(),
            );
            assert!(node.is_special());
        }
    }

    #[test]
    fn type_of_node() {
        let document = Document::shared(None);
        let node = Node::new_document(&document, Location::default());
        assert_eq!(node.type_of(), NodeType::Document);
        let node = Node::new_text(&document, Location::default(), "test");
        assert_eq!(node.type_of(), NodeType::Text);
        let node = Node::new_comment(&document, Location::default(), "test");
        assert_eq!(node.type_of(), NodeType::Comment);
        let mut attributes = HashMap::new();
        attributes.insert("id".to_string(), "test".to_string());
        let node = Node::new_element(
            &document,
            "div",
            attributes,
            HTML_NAMESPACE,
            Location::default(),
        );
        assert_eq!(node.type_of(), NodeType::Element);
    }
}
