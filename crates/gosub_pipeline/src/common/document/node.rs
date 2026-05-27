use crate::common::document::document::Document;
use crate::common::document::style::{Display, NodeStyle, StyleProperty, Value};
use std::collections::HashMap;

pub use gosub_shared::node::NodeId;

/// Map of attributes for a html element (a href, src, data-*, etc)
#[derive(Debug, Clone)]
pub struct AttrMap {
    attributes: HashMap<String, String>,
}

impl Default for AttrMap {
    fn default() -> Self {
        Self::new()
    }
}

impl AttrMap {
    pub fn new() -> AttrMap {
        AttrMap { attributes: HashMap::new() }
    }

    pub fn get(&self, key: &str) -> Option<&String> {
        self.attributes.get(key)
    }

    pub fn set(&mut self, key: &str, value: &str) {
        self.attributes.insert(key.to_string(), value.to_string());
    }

    #[allow(unused)]
    pub fn all(&self) -> &HashMap<String, String> {
        &self.attributes
    }
}

impl std::fmt::Display for AttrMap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut keys: Vec<&String> = self.attributes.keys().collect();
        keys.sort();
        let parts: Vec<String> = keys
            .iter()
            .map(|k| format!("{}=\"{}\"", k, self.attributes[*k]))
            .collect();
        write!(f, "{}", parts.join(" "))
    }
}

/// Data for a html element (tag name, attributes, styles etc)
#[derive(Clone, Debug)]
pub struct ElementData {
    pub tag_name: String,
    pub attributes: AttrMap,
    #[allow(unused)]
    pub self_closing: bool,
    /// Own CSS properties for this element (only explicitly-set values; no inheritance).
    pub styles: NodeStyle,
}

impl ElementData {
    pub fn new(
        tag_name: String,
        attributes: Option<AttrMap>,
        is_self_closing: bool,
        styles: Option<NodeStyle>,
    ) -> ElementData {
        ElementData {
            tag_name,
            attributes: attributes.unwrap_or_default(),
            self_closing: is_self_closing,
            styles: styles.unwrap_or_default(),
        }
    }

    pub fn get_style(&self, key: &StyleProperty) -> Option<&Value> {
        self.styles.get_own(key)
    }

    #[allow(unused)]
    pub fn get_attribute(&self, key: &str) -> Option<&String> {
        self.attributes.get(key)
    }

    #[allow(unused)]
    pub fn set_attribute(&mut self, key: &str, value: &str) {
        self.attributes.set(key, value);
    }

    #[allow(unused)]
    pub fn is_self_closing(&self) -> bool {
        self.self_closing
    }

    pub fn is_inline_element(&self) -> bool {
        matches!(self.get_style(&StyleProperty::Display), Some(Value::Display(Display::Inline)))
    }

    pub fn is_inline_block_element(&self) -> bool {
        matches!(
            self.get_style(&StyleProperty::Display),
            Some(Value::Display(Display::InlineBlock))
        )
    }
}

#[derive(Clone, Debug)]
pub enum NodeType {
    Comment(String),
    /// Text node — no own style; all properties inherited via the document parent chain.
    Text(String),
    Element(ElementData),
}

#[derive(Clone, Debug)]
pub struct Node {
    pub node_id: NodeId,
    pub parent_id: Option<NodeId>,
    pub node_type: NodeType,
    pub children: Vec<NodeId>,
}

impl Node {
    pub fn is_block_element(&self) -> bool {
        match &self.node_type {
            NodeType::Element(data) => {
                matches!(data.get_style(&StyleProperty::Display), Some(Value::Display(Display::Block)))
            }
            _ => false,
        }
    }

    pub fn is_inline_block_element(&self) -> bool {
        match &self.node_type {
            NodeType::Element(data) => matches!(
                data.get_style(&StyleProperty::Display),
                Some(Value::Display(Display::InlineBlock))
            ),
            _ => false,
        }
    }

    pub fn is_inline_element(&self) -> bool {
        match &self.node_type {
            NodeType::Element(data) => {
                matches!(data.get_style(&StyleProperty::Display), Some(Value::Display(Display::Inline)))
            }
            _ => false,
        }
    }

    pub fn is_text(&self) -> bool {
        matches!(&self.node_type, NodeType::Text(_))
    }

    pub fn get_style_f32(&self, prop: &StyleProperty) -> f32 {
        match &self.node_type {
            NodeType::Element(data) => match data.get_style(prop) {
                Some(Value::Unit(px, _)) => *px,
                Some(Value::Number(px)) => *px,
                _ => 0.0,
            },
            _ => 0.0,
        }
    }
}

impl Node {
    pub fn new_text(doc: &Document, parent_id: Option<NodeId>, text: String) -> Node {
        Node {
            node_id: doc.next_node_id(),
            parent_id,
            children: vec![],
            node_type: NodeType::Text(text),
        }
    }

    pub fn new_comment(doc: &Document, parent_id: Option<NodeId>, comment: String) -> Node {
        Node {
            node_id: doc.next_node_id(),
            parent_id,
            children: vec![],
            node_type: NodeType::Comment(comment),
        }
    }

    pub fn new_element(
        doc: &Document,
        parent_id: Option<NodeId>,
        tag_name: String,
        attributes: Option<AttrMap>,
        self_closing: bool,
        style: Option<NodeStyle>,
    ) -> Node {
        Node {
            node_id: doc.next_node_id(),
            parent_id,
            children: vec![],
            node_type: NodeType::Element(ElementData::new(tag_name, attributes, self_closing, style)),
        }
    }
}
