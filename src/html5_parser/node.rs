use std::collections::HashMap;

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

/// Node that resembles a DOM node
pub struct Node {
    /// ID of the node, 0 is always the root / document node
    pub id: usize,
    /// parent of the node, if any
    pub parent: Option<usize>,
    /// children of the node
    pub children: Vec<usize>,
    /// name of the node, or empty when it's not a tag
    pub name: String,
    /// namespace of the node
    pub namespace: Option<String>,
    /// actual data of the node
    pub data: NodeData,
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
    /// Create a new document node
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

    /// Create a new element node with the given name and attributes and namespace
    pub fn new_element(name: &str, attributes: HashMap<String, String>, namespace: &str) -> Self {
        Node {
            id: 0,
            parent: None,
            children: vec![],
            data: NodeData::Element {
                name: name.to_string(),
                attributes,
            },
            name: name.to_string(),
            namespace: Some(namespace.into()),
        }
    }

    /// Create a new comment node
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

    /// Create a new text node
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
    pub fn contains_attribute(&self, name: String) -> Result<bool, String> {
        if self.type_of() != NodeType::Element {
            return Err(String::from(ATTRIBUTE_NODETYPE_ERR_MSG));
        }

        let contains: bool = match &self.data {
            NodeData::Element { attributes, .. } => attributes.contains_key(&name),
            _ => false,
        };

        Ok(contains)
    }

    /// Add or update a an attribute
    pub fn insert_attribute(&mut self, name: String, value: String) -> Result<(), String> {
        if self.type_of() != NodeType::Element {
            return Err(String::from(ATTRIBUTE_NODETYPE_ERR_MSG));
        }

        if let NodeData::Element { attributes, .. } = &mut self.data {
            attributes.insert(name, value);
        }

        Ok(())
    }

    /// Remove an attribute. If attribute doesn't exist, nothing happens.
    pub fn remove_attribute(&mut self, name: String) -> Result<(), String> {
        if self.type_of() != NodeType::Element {
            return Err(String::from(ATTRIBUTE_NODETYPE_ERR_MSG));
        }

        if let NodeData::Element { attributes, .. } = &mut self.data {
            attributes.remove(&name);
        }

        Ok(())
    }

    /// Get a constant reference to the attribute value
    /// (or None if attribute doesn't exist)
    pub fn get_attribute(&self, name: String) -> Result<Option<&String>, String> {
        if self.type_of() != NodeType::Element {
            return Err(String::from(ATTRIBUTE_NODETYPE_ERR_MSG));
        }

        let mut value: Option<&String> = None;
        if let NodeData::Element { attributes, .. } = &self.data {
            value = attributes.get(&name);
        }

        Ok(value)
    }

    /// Get a mutable reference to the attribute value
    /// (or None if the attribute doesn't exist)
    pub fn get_mut_attribute(&mut self, name: String) -> Result<Option<&mut String>, String> {
        if self.type_of() != NodeType::Element {
            return Err(String::from(ATTRIBUTE_NODETYPE_ERR_MSG));
        }

        let mut value: Option<&mut String> = None;
        if let NodeData::Element { attributes, .. } = &mut self.data {
            value = attributes.get_mut(&name);
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
        assert!(node.children.is_empty());
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
    fn test_new_comment() {
        let node = Node::new_comment("test");
        assert_eq!(node.id, 0);
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
    fn test_new_text() {
        let node = Node::new_text("test");
        assert_eq!(node.id, 0);
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
    fn test_is_special() {
        let mut attributes = HashMap::new();
        attributes.insert("id".to_string(), "test".to_string());
        let node = Node::new_element("div", attributes, HTML_NAMESPACE);
        assert!(node.is_special());
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
            assert!(node.is_special());
        }
    }

    #[test]
    fn test_special_mathml_elements() {
        for element in SPECIAL_MATHML_ELEMENTS.iter() {
            let mut attributes = HashMap::new();
            attributes.insert("id".to_string(), "test".to_string());
            let node = Node::new_element(element, attributes, MATHML_NAMESPACE);
            assert!(node.is_special());
        }
    }

    #[test]
    fn test_special_svg_elements() {
        for element in SPECIAL_SVG_ELEMENTS.iter() {
            let mut attributes = HashMap::new();
            attributes.insert("id".to_string(), "test".to_string());
            let node = Node::new_element(element, attributes, SVG_NAMESPACE);
            assert!(node.is_special());
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
                attributes,
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
                attributes,
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

    #[test]
    fn test_contains_attribute_non_element() {
        let node = Node::new_document();
        let result = node.contains_attribute("x".to_string());
        assert!(result.is_err())
    }

    #[test]
    fn test_contains_attribute() {
        let mut attr = HashMap::new();
        attr.insert("x".to_string(), "value".to_string());

        let node = Node::new_element("node", attr.clone(), HTML_NAMESPACE);

        match node.contains_attribute("x".to_string()) {
            Err(_) => assert!(false),
            Ok(result) => {
                assert_eq!(result, true);
            }
        }

        match node.contains_attribute("z".to_string()) {
            Err(_) => assert!(false),
            Ok(result) => {
                assert_eq!(result, false);
            }
        }
    }

    #[test]
    fn test_insert_attrubte_non_element() {
        let mut node = Node::new_document();
        let result = node.insert_attribute("name".to_string(), "value".to_string());
        match result {
            Err(_) => assert!(true),
            Ok(_) => assert!(false),
        }
    }

    #[test]
    fn test_insert_attribute() {
        let attr = HashMap::new();
        let mut node = Node::new_element("name", attr.clone(), HTML_NAMESPACE);
        if let Ok(_) = node.insert_attribute("key".to_string(), "value".to_string()) {
            if let Ok(value) = node.get_attribute("key".to_string()) {
                match value {
                    None => assert!(false),
                    Some(val) => assert_eq!(*val, "value".to_string()),
                }
            }
        }
    }

    #[test]
    fn test_remove_attribute_non_element() {
        let mut node = Node::new_document();
        let result = node.remove_attribute("name".to_string());
        match result {
            Err(_) => assert!(true),
            Ok(_) => assert!(false),
        }
    }

    #[test]
    fn test_remove_attribute() {
        let mut attr = HashMap::new();
        attr.insert("key".to_string(), "value".to_string());

        let mut node = Node::new_element("name", attr.clone(), HTML_NAMESPACE);

        if let Ok(_) = node.remove_attribute("key".to_string()) {
            if let Ok(result) = node.contains_attribute("key".to_string()) {
                assert_eq!(result, false);
            }
        }
    }

    #[test]
    fn test_get_attribute_non_element() {
        let node = Node::new_document();

        match node.get_attribute("name".to_string()) {
            Err(_) => assert!(true),
            Ok(_) => assert!(false),
        }
    }

    #[test]
    fn test_get_attribute() {
        let mut attr = HashMap::new();
        attr.insert("key".to_string(), "value".to_string());

        let node = Node::new_element("name", attr.clone(), HTML_NAMESPACE);

        if let Ok(value) = node.get_attribute("key".to_string()) {
            match value {
                None => assert!(false),
                Some(value_str) => assert_eq!(*value_str, "value".to_string()),
            }
        }
    }

    #[test]
    fn test_get_mut_attribute_non_element() {
        let mut node = Node::new_document();

        match node.get_mut_attribute("key".to_string()) {
            Err(_) => assert!(true),
            Ok(_) => assert!(false),
        }
    }

    #[test]
    fn test_get_mut_attribute() {
        let mut attr = HashMap::new();
        attr.insert("key".to_string(), "value".to_string());

        let mut node = Node::new_element("name", attr.clone(), HTML_NAMESPACE);

        if let Ok(value) = node.get_mut_attribute("key".to_string()) {
            match value {
                None => assert!(false),
                Some(value_str) => (*value_str).push_str(" appended"),
            }

            if let Ok(value2) = node.get_attribute("key".to_string()) {
                match value2 {
                    None => assert!(false),
                    Some(value_str) => assert_eq!(*value_str, "value appended"),
                }
            }
        }
    }

    #[test]
    fn test_clear_attributes_non_element() {
        let mut node = Node::new_document();

        match node.clear_attributes() {
            Err(_) => assert!(true),
            Ok(_) => assert!(false),
        }
    }

    #[test]
    fn test_clear_attributes() {
        let mut attr = HashMap::new();
        attr.insert("key".to_string(), "value".to_string());

        let mut node = Node::new_element("name", attr.clone(), HTML_NAMESPACE);
        if let Ok(_) = node.clear_attributes() {
            assert_eq!(node.has_attributes(), false);
        }
    }

    #[test]
    fn test_has_attributes_non_element() {
        // if node is a non-element, will always return false
        let node = Node::new_document();
        assert_eq!(node.has_attributes(), false);
    }

    #[test]
    fn test_has_attributes() {
        let attr = HashMap::new();

        let mut node = Node::new_element("name", attr.clone(), HTML_NAMESPACE);
        assert_eq!(node.has_attributes(), false);

        if let Ok(_) = node.insert_attribute("key".to_string(), "value".to_string()) {
            assert_eq!(node.has_attributes(), true);
        }
    }
}
