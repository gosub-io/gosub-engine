use derive_more::Display;
use std::collections::HashMap;
use crate::html5_parser::element_class::ElementClass;

pub const HTML_NAMESPACE: &str = "http://www.w3.org/1999/xhtml";
pub const MATHML_NAMESPACE: &str = "http://www.w3.org/1998/Math/MathML";
pub const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";
pub const XLINK_NAMESPACE: &str = "http://www.w3.org/1999/xlink";
pub const XML_NAMESPACE: &str = "http://www.w3.org/XML/1998/namespace";
pub const XMLNS_NAMESPACE: &str = "http://www.w3.org/2000/xmlns/";

const ATTRIBUTE_NODETYPE_ERR_MSG: &str = "Node type must be Element to access attributes";

/// Different types of nodes
#[derive(Debug, PartialEq)]
pub enum NodeType {
    Document,
    Text,
    Comment,
    Element,
}

/// Different type of node data
#[derive(Debug, PartialEq, Clone)]
pub enum NodeData {
    Document,
    Text {
        value: String,
    },
    Comment {
        value: String,
    },
    Element {
        name: String,
        attributes: HashMap<String, String>,
    },
}

/// Id used to identify a node
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq, Display)]
pub struct NodeId(pub usize);

impl From<NodeId> for usize {
    fn from(value: NodeId) -> Self {
        value.0
    }
}

impl From<usize> for NodeId {
    fn from(value: usize) -> Self {
        Self(value)
    }
}

impl Default for &NodeId {
    fn default() -> Self {
        &NodeId(0)
    }
}

impl NodeId {
    // TODO: Drop Default derive and only use 0 for the root, or choose another id for the root
    pub const ROOT_NODE: usize = 0;

    pub fn root() -> Self {
        Self(Self::ROOT_NODE)
    }

    pub fn is_positive(&self) -> bool {
        self.0 > 0
    }

    pub fn is_root(&self) -> bool {
        self.0 == Self::ROOT_NODE
    }

    pub fn next(&self) -> Self {
        // Might panic
        Self(self.0 + 1)
    }

    pub fn prev(&self) -> Self {
        // Might panic
        Self(self.0 - 1)
    }
}

/// Node that resembles a DOM node
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
    /// CSS classes (only relevant for NodeType::Element, otherwise None)
    pub classes: Option<ElementClass>,
}

impl Node {
    // This will only compare against the tag, namespace and attributes. Both nodes could still have
    // other parents and children.
    pub fn matches_tag_and_attrs(&self, other: &Self) -> bool {
        self.name == other.name && self.namespace == other.namespace && self.data == other.data
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
            classes: self.classes.clone(),
        }
    }
}

impl Node {
    /// Create a new document node
    pub fn new_document() -> Self {
        Node {
            id: Default::default(),
            parent: None,
            children: vec![],
            data: NodeData::Document {},
            name: "".to_string(),
            namespace: None,
            classes: None,
        }
    }

    /// Create a new element node with the given name and attributes and namespace
    pub fn new_element(name: &str, attributes: HashMap<String, String>, namespace: &str) -> Self {
        Node {
            id: Default::default(),
            parent: None,
            children: vec![],
            data: NodeData::Element {
                name: name.to_string(),
                attributes,
            },
            name: name.to_string(),
            namespace: Some(namespace.into()),
            classes: Some(ElementClass::new()),
        }
    }

    /// Create a new comment node
    pub fn new_comment(value: &str) -> Self {
        Node {
            id: Default::default(),
            parent: None,
            children: vec![],
            data: NodeData::Comment {
                value: value.to_string(),
            },
            name: "".to_string(),
            namespace: None,
            classes: None,
        }
    }

    /// Create a new text node
    pub fn new_text(value: &str) -> Self {
        Node {
            id: Default::default(),
            parent: None,
            children: vec![],
            data: NodeData::Text {
                value: value.to_string(),
            },
            name: "".to_string(),
            namespace: None,
            classes: None,
        }
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

    /// Check if an attribute exists
    pub fn contains_attribute(&self, name: &str) -> Result<bool, String> {
        if self.type_of() != NodeType::Element {
            return Err(ATTRIBUTE_NODETYPE_ERR_MSG.into());
        }

        let contains: bool = match &self.data {
            NodeData::Element { attributes, .. } => attributes.contains_key(name),
            _ => false,
        };

        Ok(contains)
    }

    /// Add or update a an attribute
    pub fn insert_attribute(&mut self, name: &str, value: &str) -> Result<(), String> {
        if self.type_of() != NodeType::Element {
            return Err(ATTRIBUTE_NODETYPE_ERR_MSG.into());
        }

        if let NodeData::Element { attributes, .. } = &mut self.data {
            attributes.insert(name.to_owned(), value.to_owned());
        }

        Ok(())
    }

    /// Remove an attribute. If attribute doesn't exist, nothing happens.
    pub fn remove_attribute(&mut self, name: &str) -> Result<(), String> {
        if self.type_of() != NodeType::Element {
            return Err(ATTRIBUTE_NODETYPE_ERR_MSG.into());
        }

        if let NodeData::Element { attributes, .. } = &mut self.data {
            attributes.remove(name);
        }

        Ok(())
    }

    /// Get a constant reference to the attribute value
    /// (or None if attribute doesn't exist)
    pub fn get_attribute(&self, name: &str) -> Result<Option<&String>, String> {
        if self.type_of() != NodeType::Element {
            return Err(ATTRIBUTE_NODETYPE_ERR_MSG.into());
        }

        let mut value: Option<&String> = None;
        if let NodeData::Element { attributes, .. } = &self.data {
            value = attributes.get(name);
        }

        Ok(value)
    }

    /// Get a mutable reference to the attribute value
    /// (or None if the attribute doesn't exist)
    pub fn get_mut_attribute(&mut self, name: &str) -> Result<Option<&mut String>, String> {
        if self.type_of() != NodeType::Element {
            return Err(ATTRIBUTE_NODETYPE_ERR_MSG.into());
        }

        let mut value: Option<&mut String> = None;
        if let NodeData::Element { attributes, .. } = &mut self.data {
            value = attributes.get_mut(name);
        }

        Ok(value)
    }

    /// Remove all attributes
    pub fn clear_attributes(&mut self) -> Result<(), String> {
        if self.type_of() != NodeType::Element {
            return Err(String::from(ATTRIBUTE_NODETYPE_ERR_MSG));
        }

        if let NodeData::Element { attributes, .. } = &mut self.data {
            attributes.clear();
        }

        Ok(())
    }

    /// Check if node has any attributes
    /// (NOTE: if node is not Element type, returns false anyways)
    pub fn has_attributes(&self) -> bool {
        if self.type_of() != NodeType::Element {
            return false;
        }

        let mut has_attr: bool = false;
        if let NodeData::Element { attributes, .. } = &self.data {
            has_attr = !attributes.is_empty();
        }

        has_attr
    }
}

pub trait NodeTrait {
    // Return the token type of the given token
    fn type_of(&self) -> NodeType;
}

// Each node implements the NodeTrait and has a type_of that will return the node type.
impl NodeTrait for Node {
    fn type_of(&self) -> NodeType {
        match self.data {
            NodeData::Document { .. } => NodeType::Document,
            NodeData::Text { .. } => NodeType::Text,
            NodeData::Comment { .. } => NodeType::Comment,
            NodeData::Element { .. } => NodeType::Element,
        }
    }
}

pub static FORMATTING_HTML_ELEMENTS: [&str; 14] = [
    "a", "b", "big", "code", "em", "font", "i", "nobr", "s", "small", "strike", "strong", "tt", "u",
];

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

pub static SPECIAL_MATHML_ELEMENTS: [&str; 6] = ["mi", "mo", "mn", "ms", "mtext", "annotation-xml"];

pub static SPECIAL_SVG_ELEMENTS: [&str; 3] = ["foreignObject", "desc", "title"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_document() {
        let node = Node::new_document();
        assert_eq!(node.id, NodeId::default());
        assert_eq!(node.parent, None);
        assert!(node.children.is_empty());
        assert_eq!(node.name, "".to_string());
        assert_eq!(node.namespace, None);
        assert_eq!(node.data, NodeData::Document {});
    }

    #[test]
    fn new_element() {
        let mut attributes = HashMap::new();
        attributes.insert("id".to_string(), "test".to_string());
        let node = Node::new_element("div", attributes.clone(), HTML_NAMESPACE);
        assert_eq!(node.id, NodeId::default());
        assert_eq!(node.parent, None);
        assert!(node.children.is_empty());
        assert_eq!(node.name, "div".to_string());
        assert_eq!(node.namespace, Some(HTML_NAMESPACE.into()));
        assert_eq!(
            node.data,
            NodeData::Element {
                name: "div".to_string(),
                attributes: attributes.clone(),
            }
        );
    }

    #[test]
    fn new_comment() {
        let node = Node::new_comment("test");
        assert_eq!(node.id, NodeId::default());
        assert_eq!(node.parent, None);
        assert!(node.children.is_empty());
        assert_eq!(node.name, "".to_string());
        assert_eq!(node.namespace, None);
        assert_eq!(
            node.data,
            NodeData::Comment {
                value: "test".to_string(),
            }
        );
    }

    #[test]
    fn new_text() {
        let node = Node::new_text("test");
        assert_eq!(node.id, NodeId::default());
        assert_eq!(node.parent, None);
        assert!(node.children.is_empty());
        assert_eq!(node.name, "".to_string());
        assert_eq!(node.namespace, None);
        assert_eq!(
            node.data,
            NodeData::Text {
                value: "test".to_string(),
            }
        );
    }

    #[test]
    fn is_special() {
        let mut attributes = HashMap::new();
        attributes.insert("id".to_string(), "test".to_string());
        let node = Node::new_element("div", attributes, HTML_NAMESPACE);
        assert!(node.is_special());
    }

    #[test]
    fn type_of() {
        let node = Node::new_document();
        assert_eq!(node.type_of(), NodeType::Document);
        let node = Node::new_text("test");
        assert_eq!(node.type_of(), NodeType::Text);
        let node = Node::new_comment("test");
        assert_eq!(node.type_of(), NodeType::Comment);
        let mut attributes = HashMap::new();
        attributes.insert("id".to_string(), "test".to_string());
        let node = Node::new_element("div", attributes, HTML_NAMESPACE);
        assert_eq!(node.type_of(), NodeType::Element);
    }

    #[test]
    fn special_html_elements() {
        for element in SPECIAL_HTML_ELEMENTS.iter() {
            let mut attributes = HashMap::new();
            attributes.insert("id".to_string(), "test".to_string());
            let node = Node::new_element(element, attributes, HTML_NAMESPACE);
            assert!(node.is_special());
        }
    }

    #[test]
    fn special_mathml_elements() {
        for element in SPECIAL_MATHML_ELEMENTS.iter() {
            let mut attributes = HashMap::new();
            attributes.insert("id".to_string(), "test".to_string());
            let node = Node::new_element(element, attributes, MATHML_NAMESPACE);
            assert!(node.is_special());
        }
    }

    #[test]
    fn special_svg_elements() {
        for element in SPECIAL_SVG_ELEMENTS.iter() {
            let mut attributes = HashMap::new();
            attributes.insert("id".to_string(), "test".to_string());
            let node = Node::new_element(element, attributes, SVG_NAMESPACE);
            assert!(node.is_special());
        }
    }

    #[test]
    fn type_of_node() {
        let node = Node::new_document();
        assert_eq!(node.type_of(), NodeType::Document);
        let node = Node::new_text("test");
        assert_eq!(node.type_of(), NodeType::Text);
        let node = Node::new_comment("test");
        assert_eq!(node.type_of(), NodeType::Comment);
        let mut attributes = HashMap::new();
        attributes.insert("id".to_string(), "test".to_string());
        let node = Node::new_element("div", attributes, HTML_NAMESPACE);
        assert_eq!(node.type_of(), NodeType::Element);
    }

    #[test]
    fn type_of_node_data() {
        let node = Node::new_document();
        assert_eq!(node.data, NodeData::Document {});
        let node = Node::new_text("test");
        assert_eq!(
            node.data,
            NodeData::Text {
                value: "test".to_string(),
            }
        );
        let node = Node::new_comment("test");
        assert_eq!(
            node.data,
            NodeData::Comment {
                value: "test".to_string(),
            }
        );
        let mut attributes = HashMap::new();
        attributes.insert("id".to_string(), "test".to_string());
        let node = Node::new_element("div", attributes.clone(), HTML_NAMESPACE);
        assert_eq!(
            node.data,
            NodeData::Element {
                name: "div".to_string(),
                attributes,
            }
        );
    }

    #[test]
    fn type_of_node_data_element() {
        let mut attributes = HashMap::new();
        attributes.insert("id".to_string(), "test".to_string());
        let node = Node::new_element("div", attributes.clone(), HTML_NAMESPACE);
        assert_eq!(
            node.data,
            NodeData::Element {
                name: "div".to_string(),
                attributes,
            }
        );
    }

    #[test]
    fn type_of_node_data_text() {
        let node = Node::new_text("test");
        assert_eq!(
            node.data,
            NodeData::Text {
                value: "test".to_string(),
            }
        );
    }

    #[test]
    fn type_of_node_data_comment() {
        let node = Node::new_comment("test");
        assert_eq!(
            node.data,
            NodeData::Comment {
                value: "test".to_string(),
            }
        );
    }

    #[test]
    fn type_of_node_data_document() {
        let node = Node::new_document();
        assert_eq!(node.data, NodeData::Document {});
    }

    #[test]
    fn contains_attribute_non_element() {
        let node = Node::new_document();
        let result = node.contains_attribute("x");
        assert!(result.is_err())
    }

    #[test]
    fn contains_attribute() {
        let mut attr = HashMap::new();
        attr.insert("x".to_string(), "value".to_string());

        let node = Node::new_element("node", attr.clone(), HTML_NAMESPACE);

        assert!(node.contains_attribute("x").unwrap());
        assert!(!node.contains_attribute("z").unwrap());
    }

    #[test]
    fn insert_attrubte_non_element() {
        let mut node = Node::new_document();
        let result = node.insert_attribute("name", "value");
        assert!(result.is_err());
    }

    #[test]
    fn insert_attribute() {
        let attr = HashMap::new();
        let mut node = Node::new_element("name", attr.clone(), HTML_NAMESPACE);

        assert!(node.insert_attribute("key", "value").is_ok());
        let value = node.get_attribute("key").unwrap().unwrap();
        assert_eq!(value, "value");
    }

    #[test]
    fn remove_attribute_non_element() {
        let mut node = Node::new_document();
        let result = node.remove_attribute("name");
        assert!(result.is_err());
    }

    #[test]
    fn remove_attribute() {
        let mut attr = HashMap::new();
        attr.insert("key".to_string(), "value".to_string());

        let mut node = Node::new_element("name", attr.clone(), HTML_NAMESPACE);

        assert!(node.remove_attribute("key").is_ok());
        let result = node.contains_attribute("key").unwrap();
        assert!(!result);
    }

    #[test]
    fn get_attribute_non_element() {
        let node = Node::new_document();
        let result = node.get_attribute("name");
        assert!(result.is_err());
    }

    #[test]
    fn get_attribute() {
        let mut attr = HashMap::new();
        attr.insert("key".to_string(), "value".to_string());

        let node = Node::new_element("name", attr.clone(), HTML_NAMESPACE);

        let value = node.get_attribute("key").unwrap().unwrap();
        assert_eq!(value, "value");
    }

    #[test]
    fn get_mut_attribute_non_element() {
        let mut node = Node::new_document();
        let result = node.get_mut_attribute("key");
        assert!(result.is_err());
    }

    #[test]
    fn get_mut_attribute() {
        let mut attr = HashMap::new();
        attr.insert("key".to_string(), "value".to_string());

        let mut node = Node::new_element("name", attr.clone(), HTML_NAMESPACE);

        let value = node.get_mut_attribute("key").unwrap().unwrap();
        value.push_str(" appended");

        let value = node.get_attribute("key").unwrap().unwrap();
        assert_eq!(value, "value appended");
    }

    #[test]
    fn clear_attributes_non_element() {
        let mut node = Node::new_document();
        let result = node.clear_attributes();
        assert!(result.is_err());
    }

    #[test]
    fn clear_attributes() {
        let mut attr = HashMap::new();
        attr.insert("key".to_string(), "value".to_string());

        let mut node = Node::new_element("name", attr.clone(), HTML_NAMESPACE);
        assert!(node.clear_attributes().is_ok());
        assert!(!node.has_attributes());
    }

    #[test]
    fn has_attributes_non_element() {
        // if node is a non-element, will always return false
        let node = Node::new_document();
        assert!(!node.has_attributes());
    }

    #[test]
    fn has_attributes() {
        let attr = HashMap::new();

        let mut node = Node::new_element("name", attr.clone(), HTML_NAMESPACE);
        assert_eq!(node.has_attributes(), false);

        assert!(node.insert_attribute("key", "value").is_ok());
        assert!(node.has_attributes());
    }
}
