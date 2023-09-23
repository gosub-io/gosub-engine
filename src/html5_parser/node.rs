use std::collections::HashMap;

pub const HTML_NAMESPACE: &str = "http://www.w3.org/1999/xhtml";
pub const MATHML_NAMESPACE: &str = "http://www.w3.org/1998/Math/MathML";
pub const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";
pub const XLINK_NAMESPACE: &str = "http://www.w3.org/1999/xlink";
pub const XML_NAMESPACE: &str = "http://www.w3.org/XML/1998/namespace";
pub const XMLNS_NAMESPACE: &str = "http://www.w3.org/2000/xmlns/";

// Different types of nodes
#[derive(Debug, PartialEq)]
pub enum NodeType {
    Document,
    Text,
    Comment,
    Element,
}

// Different type of node data
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

// Node that resembles a DOM node
pub struct Node {
    pub id: usize,                 // ID of the node, 0 is always the root / document node
    pub parent: Option<usize>,     // parent of the node, if any
    pub children: Vec<usize>,      // children of the node
    pub name: String,              // name of the node, or empty when it's not a tag
    pub namespace: Option<String>, // namespace of the node
    pub data: NodeData,            // actual data of the node
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
        }
    }
}

impl Node {
    // Create a new document node
    pub fn new_document() -> Self {
        Node {
            id: 0,
            parent: None,
            children: vec![],
            data: NodeData::Document {},
            name: "".to_string(),
            namespace: None,
        }
    }

    // Create a new element node with the given name and attributes and namespace
    pub fn new_element(name: &str, attributes: HashMap<String, String>, namespace: &str) -> Self {
        Node {
            id: 0,
            parent: None,
            children: vec![],
            data: NodeData::Element {
                name: name.to_string(),
                attributes: attributes,
            },
            name: name.to_string(),
            namespace: Some(namespace.into()),
        }
    }

    // Create a new comment node
    pub fn new_comment(value: &str) -> Self {
        Node {
            id: 0,
            parent: None,
            children: vec![],
            data: NodeData::Comment {
                value: value.to_string(),
            },
            name: "".to_string(),
            namespace: None,
        }
    }

    // Create a new text node
    pub fn new_text(value: &str) -> Self {
        Node {
            id: 0,
            parent: None,
            children: vec![],
            data: NodeData::Text {
                value: value.to_string(),
            },
            name: "".to_string(),
            namespace: None,
        }
    }

    // Returns true if the given node is "special" node based on the namespace and name
    pub fn is_special(&self) -> bool {
        if self.namespace == Some(HTML_NAMESPACE.into()) {
            if SPECIAL_HTML_ELEMENTS.contains(&self.name.as_str()) {
                return true;
            }
        }
        if self.namespace == Some(MATHML_NAMESPACE.into()) {
            if SPECIAL_MATHML_ELEMENTS.contains(&self.name.as_str()) {
                return true;
            }
        }
        if self.namespace == Some(SVG_NAMESPACE.into()) {
            if SPECIAL_SVG_ELEMENTS.contains(&self.name.as_str()) {
                return true;
            }
        }

        false
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

pub static SPECIAL_HTML_ELEMENTS: [&str; 81] = [
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
mod test {
    use super::*;

    #[test]
    fn test_new_document() {
        let node = Node::new_document();
        assert_eq!(node.id, 0);
        assert_eq!(node.parent, None);
        assert_eq!(node.children, vec![]);
        assert_eq!(node.name, "".to_string());
        assert_eq!(node.namespace, None);
        assert_eq!(node.data, NodeData::Document {});
    }

    #[test]
    fn test_new_element() {
        let mut attributes = HashMap::new();
        attributes.insert("id".to_string(), "test".to_string());
        let node = Node::new_element("div", attributes.clone(), HTML_NAMESPACE);
        assert_eq!(node.id, 0);
        assert_eq!(node.parent, None);
        assert_eq!(node.children, vec![]);
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
    fn test_new_comment() {
        let node = Node::new_comment("test");
        assert_eq!(node.id, 0);
        assert_eq!(node.parent, None);
        assert_eq!(node.children, vec![]);
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
    fn test_new_text() {
        let node = Node::new_text("test");
        assert_eq!(node.id, 0);
        assert_eq!(node.parent, None);
        assert_eq!(node.children, vec![]);
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
    fn test_is_special() {
        let mut attributes = HashMap::new();
        attributes.insert("id".to_string(), "test".to_string());
        let node = Node::new_element("div", attributes, HTML_NAMESPACE);
        assert_eq!(node.is_special(), true);
    }

    #[test]
    fn test_type_of() {
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
    fn test_special_html_elements() {
        for element in SPECIAL_HTML_ELEMENTS.iter() {
            let mut attributes = HashMap::new();
            attributes.insert("id".to_string(), "test".to_string());
            let node = Node::new_element(element, attributes, HTML_NAMESPACE);
            assert_eq!(node.is_special(), true);
        }
    }

    #[test]
    fn test_special_mathml_elements() {
        for element in SPECIAL_MATHML_ELEMENTS.iter() {
            let mut attributes = HashMap::new();
            attributes.insert("id".to_string(), "test".to_string());
            let node = Node::new_element(element, attributes, MATHML_NAMESPACE);
            assert_eq!(node.is_special(), true);
        }
    }

    #[test]
    fn test_special_svg_elements() {
        for element in SPECIAL_SVG_ELEMENTS.iter() {
            let mut attributes = HashMap::new();
            attributes.insert("id".to_string(), "test".to_string());
            let node = Node::new_element(element, attributes, SVG_NAMESPACE);
            assert_eq!(node.is_special(), true);
        }
    }

    #[test]
    fn test_type_of_node() {
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
    fn test_type_of_node_data() {
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
                attributes: attributes,
            }
        );
    }

    #[test]
    fn test_type_of_node_data_element() {
        let mut attributes = HashMap::new();
        attributes.insert("id".to_string(), "test".to_string());
        let node = Node::new_element("div", attributes.clone(), HTML_NAMESPACE);
        assert_eq!(
            node.data,
            NodeData::Element {
                name: "div".to_string(),
                attributes: attributes,
            }
        );
    }

    #[test]
    fn test_type_of_node_data_text() {
        let node = Node::new_text("test");
        assert_eq!(
            node.data,
            NodeData::Text {
                value: "test".to_string(),
            }
        );
    }

    #[test]
    fn test_type_of_node_data_comment() {
        let node = Node::new_comment("test");
        assert_eq!(
            node.data,
            NodeData::Comment {
                value: "test".to_string(),
            }
        );
    }

    #[test]
    fn test_type_of_node_data_document() {
        let node = Node::new_document();
        assert_eq!(node.data, NodeData::Document {});
    }
}
