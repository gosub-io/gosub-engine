use crate::node::data::comment::CommentData;
use crate::node::data::doctype::DocTypeData;
use crate::node::data::document::DocumentData;
use crate::node::data::element::ElementData;
use crate::node::data::text::TextData;
use gosub_interface::node::{NodeType, QuirksMode};
use gosub_shared::byte_stream::Location;
use gosub_shared::node::NodeId;
use std::collections::HashMap;

/// Node data variants
#[derive(Debug, Clone, PartialEq)]
pub enum NodeDataTypeInternal {
    Document(DocumentData),
    DocType(DocTypeData),
    Text(TextData),
    Comment(CommentData),
    Element(ElementData),
}

/// A DOM node stored in the arena
pub struct NodeImpl {
    pub id: NodeId,
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    pub data: NodeDataTypeInternal,
    pub registered: bool,
    pub location: Location,
}

impl PartialEq for NodeImpl {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl core::fmt::Debug for NodeImpl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug = f.debug_struct("Node");
        debug.field("id", &self.id);
        debug.field("parent", &self.parent);
        debug.field("children", &self.children);
        debug.finish_non_exhaustive()
    }
}

impl Clone for NodeImpl {
    fn clone(&self) -> Self {
        NodeImpl {
            id: self.id,
            parent: self.parent,
            children: self.children.clone(),
            data: self.data.clone(),
            registered: self.registered,
            location: self.location,
        }
    }
}

impl NodeImpl {
    #[must_use]
    pub fn new(location: Location, data: NodeDataTypeInternal) -> Self {
        Self {
            id: NodeId::default(),
            parent: None,
            children: Vec::new(),
            data,
            registered: false,
            location,
        }
    }

    #[must_use]
    pub fn new_document(location: Location, quirks_mode: QuirksMode) -> Self {
        Self::new(location, NodeDataTypeInternal::Document(DocumentData::new(quirks_mode)))
    }

    #[must_use]
    pub fn new_doctype(location: Location, name: &str, pub_identifier: &str, sys_identifier: &str) -> Self {
        Self::new(
            location,
            NodeDataTypeInternal::DocType(DocTypeData::new(name, pub_identifier, sys_identifier)),
        )
    }

    #[must_use]
    pub fn new_element(
        location: Location,
        name: &str,
        namespace: Option<&str>,
        attributes: HashMap<String, String>,
    ) -> Self {
        Self::new(
            location,
            NodeDataTypeInternal::Element(ElementData::new(name, namespace, attributes, Default::default())),
        )
    }

    #[must_use]
    pub fn new_comment(location: Location, value: &str) -> Self {
        Self::new(location, NodeDataTypeInternal::Comment(CommentData::with_value(value)))
    }

    #[must_use]
    pub fn new_text(location: Location, value: &str) -> Self {
        Self::new(location, NodeDataTypeInternal::Text(TextData::with_value(value)))
    }

    /// Shallow clone: same data, no tree links, not registered
    pub fn new_from_node(org_node: &Self) -> Self {
        Self {
            id: NodeId::default(),
            parent: None,
            children: Vec::new(),
            data: org_node.data.clone(),
            registered: false,
            location: org_node.location,
        }
    }

    pub fn id(&self) -> NodeId {
        self.id
    }

    pub fn set_id(&mut self, id: NodeId) {
        self.id = id;
    }

    pub fn location(&self) -> Location {
        self.location
    }

    pub fn parent_id(&self) -> Option<NodeId> {
        self.parent
    }

    pub fn set_parent(&mut self, parent_id: Option<NodeId>) {
        self.parent = parent_id;
    }

    pub fn set_registered(&mut self, registered: bool) {
        self.registered = registered;
    }

    pub fn is_registered(&self) -> bool {
        self.registered
    }

    pub fn is_root(&self) -> bool {
        self.parent.is_none()
    }

    pub fn children(&self) -> &[NodeId] {
        &self.children
    }

    pub fn type_of(&self) -> NodeType {
        match self.data {
            NodeDataTypeInternal::Document(_) => NodeType::DocumentNode,
            NodeDataTypeInternal::DocType(_) => NodeType::DocTypeNode,
            NodeDataTypeInternal::Text(_) => NodeType::TextNode,
            NodeDataTypeInternal::Comment(_) => NodeType::CommentNode,
            NodeDataTypeInternal::Element(_) => NodeType::ElementNode,
        }
    }

    pub fn is_element_node(&self) -> bool {
        self.type_of() == NodeType::ElementNode
    }

    pub fn is_text_node(&self) -> bool {
        matches!(self.data, NodeDataTypeInternal::Text(_))
    }

    pub fn get_element_data(&self) -> Option<&ElementData> {
        if let NodeDataTypeInternal::Element(data) = &self.data {
            return Some(data);
        }
        None
    }

    pub fn get_element_data_mut(&mut self) -> Option<&mut ElementData> {
        if let NodeDataTypeInternal::Element(data) = &mut self.data {
            return Some(data);
        }
        None
    }

    pub fn get_text_data(&self) -> Option<&TextData> {
        if let NodeDataTypeInternal::Text(data) = &self.data {
            return Some(data);
        }
        None
    }

    pub fn get_text_data_mut(&mut self) -> Option<&mut TextData> {
        if let NodeDataTypeInternal::Text(data) = &mut self.data {
            return Some(data);
        }
        None
    }

    pub fn get_comment_data(&self) -> Option<&CommentData> {
        if let NodeDataTypeInternal::Comment(data) = &self.data {
            return Some(data);
        }
        None
    }

    pub fn get_doctype_data(&self) -> Option<&DocTypeData> {
        if let NodeDataTypeInternal::DocType(data) = &self.data {
            return Some(data);
        }
        None
    }

    pub fn remove(&mut self, node_id: NodeId) {
        self.children.retain(|x| x != &node_id);
    }

    pub fn insert(&mut self, node_id: NodeId, idx: usize) {
        self.children.insert(idx, node_id);
    }

    pub fn push(&mut self, node_id: NodeId) {
        self.children.push(node_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::elements::SPECIAL_HTML_ELEMENTS;
    use crate::node::elements::SPECIAL_MATHML_ELEMENTS;
    use crate::node::elements::SPECIAL_SVG_ELEMENTS;
    use crate::node::HTML_NAMESPACE;
    use crate::node::MATHML_NAMESPACE;
    use crate::node::SVG_NAMESPACE;
    use std::collections::HashMap;

    #[test]
    fn new_document() {
        let node = NodeImpl::new_document(Location::default(), QuirksMode::NoQuirks);
        assert_eq!(node.id, NodeId::default());
        assert_eq!(node.parent, None);
        assert!(node.children.is_empty());
        assert!(matches!(&node.data, NodeDataTypeInternal::Document(_)));
    }

    #[test]
    fn new_element() {
        let mut attributes = HashMap::new();
        attributes.insert("id".to_string(), "test".to_string());

        let node = NodeImpl::new_element(Location::default(), "div", Some(HTML_NAMESPACE), attributes);
        assert_eq!(node.id, NodeId::default());
        assert_eq!(node.parent, None);
        assert!(node.children.is_empty());

        if let NodeDataTypeInternal::Element(data) = &node.data {
            assert_eq!(data.name(), "div");
            assert!(data.attributes().contains_key("id"));
            assert_eq!(data.attributes().get("id").unwrap(), "test");
        } else {
            panic!()
        }
    }

    #[test]
    fn new_comment() {
        let node = NodeImpl::new_comment(Location::default(), "test");
        assert_eq!(node.id, NodeId::default());
        assert_eq!(node.parent, None);
        assert!(node.children.is_empty());
        let NodeDataTypeInternal::Comment(CommentData { value, .. }) = &node.data else {
            panic!()
        };
        assert_eq!(value, "test");
    }

    #[test]
    fn new_text() {
        let node = NodeImpl::new_text(Location::default(), "test");
        assert_eq!(node.id, NodeId::default());
        assert_eq!(node.parent, None);
        assert!(node.children.is_empty());
        let NodeDataTypeInternal::Text(TextData { value }) = &node.data else {
            panic!()
        };
        assert_eq!(value, "test");
    }

    #[test]
    fn is_special() {
        let mut attributes = HashMap::new();
        attributes.insert("id".to_string(), "test".to_string());

        let node = NodeImpl::new_element(Location::default(), "div", Some(HTML_NAMESPACE), attributes);
        assert!(node.get_element_data().unwrap().is_special());
    }

    #[test]
    fn type_of() {
        let node = NodeImpl::new_document(Location::default(), QuirksMode::NoQuirks);
        assert_eq!(node.type_of(), NodeType::DocumentNode);
        let node = NodeImpl::new_text(Location::default(), "test");
        assert_eq!(node.type_of(), NodeType::TextNode);
        let node = NodeImpl::new_comment(Location::default(), "test");
        assert_eq!(node.type_of(), NodeType::CommentNode);
        let mut attributes = HashMap::new();
        attributes.insert("id".to_string(), "test".to_string());
        let node = NodeImpl::new_element(Location::default(), "div", Some(HTML_NAMESPACE), attributes);
        assert_eq!(node.type_of(), NodeType::ElementNode);
    }

    #[test]
    fn special_html_elements() {
        for element in &SPECIAL_HTML_ELEMENTS {
            let mut attributes = HashMap::new();
            attributes.insert("id".to_string(), "test".to_string());
            let node = NodeImpl::new_element(Location::default(), element, Some(HTML_NAMESPACE), attributes);
            assert!(node.get_element_data().unwrap().is_special());
        }
    }

    #[test]
    fn special_mathml_elements() {
        for element in &SPECIAL_MATHML_ELEMENTS {
            let mut attributes = HashMap::new();
            attributes.insert("id".to_string(), "test".to_string());
            let node = NodeImpl::new_element(Location::default(), element, Some(MATHML_NAMESPACE), attributes.clone());
            assert!(node.get_element_data().unwrap().is_special());
        }
    }

    #[test]
    fn special_svg_elements() {
        for element in &SPECIAL_SVG_ELEMENTS {
            let mut attributes = HashMap::new();
            attributes.insert("id".to_string(), "test".to_string());
            let node = NodeImpl::new_element(Location::default(), element, Some(SVG_NAMESPACE), attributes);
            assert!(node.get_element_data().unwrap().is_special());
        }
    }

    #[test]
    fn type_of_node() {
        let node = NodeImpl::new_document(Location::default(), QuirksMode::NoQuirks);
        assert_eq!(node.type_of(), NodeType::DocumentNode);
        let node = NodeImpl::new_text(Location::default(), "test");
        assert_eq!(node.type_of(), NodeType::TextNode);
        let node = NodeImpl::new_comment(Location::default(), "test");
        assert_eq!(node.type_of(), NodeType::CommentNode);
        let mut attributes = HashMap::new();
        attributes.insert("id".to_string(), "test".to_string());
        let node = NodeImpl::new_element(Location::default(), "div", Some(HTML_NAMESPACE), attributes);
        assert_eq!(node.type_of(), NodeType::ElementNode);
    }
}
