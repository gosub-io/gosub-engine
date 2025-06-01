use std::cmp::PartialEq;
use std::collections::HashMap;
use std::ops::AddAssign;
use crate::common::document::document::Document;
use crate::common::document::style::{StylePropertyList, StyleValue, StyleProperty, Display};
use crate::rendertree_builder::RenderNodeId;

/// Map of attributes for a html element (a href, src, data-*, etc)
#[derive(Debug, Clone)]
pub struct AttrMap {
    attributes: HashMap<String, String>,
}

impl AttrMap {
    pub fn new() -> AttrMap {
        AttrMap {
            attributes: HashMap::new(),
        }
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

    #[allow(unused)]
    pub fn to_string(&self) -> String {
        let mut result = String::new();

        // Make sure keys are always ordered in the same way
        let keys = self.attributes.keys();
        let mut keys: Vec<&String> = keys.collect();
        keys.sort();

        for key in keys {
            let value = self.attributes.get(key).unwrap();
            result.push_str(&format!("{}=\"{}\" ", key, value));
        }
        result.trim_end().to_string()
    }
}

/// Data for a html element (tag name, attributes, styles etc)
#[derive(Clone, Debug)]
pub struct ElementData {
    /// Element name (ie: P, DIV, IMG etc)
    pub tag_name: String,
    /// Element attributes (src, href, class etc)
    pub attributes: AttrMap,
    /// Is this element self-closing (ie: <img />)
    #[allow(unused)]
    pub self_closing: bool,
    /// Element styles (color, font-size etc)
    pub styles: StylePropertyList,
}

impl ElementData {
    pub fn new(tag_name: String, attributes: Option<AttrMap>, is_self_closing: bool, styles: Option<StylePropertyList>) -> ElementData {
        ElementData {
            tag_name,
            attributes: attributes.unwrap_or(AttrMap::new()),
            self_closing: is_self_closing,
            styles: styles.unwrap_or(StylePropertyList::new()),
        }
    }

    pub fn get_style(&self, key: StyleProperty) -> Option<&StyleValue> {
        self.styles.properties.get(&key)
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
        match self.get_style(StyleProperty::Display) {
            Some(StyleValue::Display(display)) => {
                *display == Display::Inline
            }
            _ => false,
        }
    }

    pub fn is_inline_block_element(&self) -> bool {
        match self.get_style(StyleProperty::Display) {
            Some(StyleValue::Display(display)) => {
                *display == Display::InlineBlock
            }
            _ => false,
        }
    }
}

#[derive(Clone, Debug)]
pub enum NodeType {
    // Comment node (<!-- comment -->)
    Comment(String),
    // Text node (ie: Some text). Does not contain any children
    Text(String, StylePropertyList),
    // Element node (ie: <div></div>)
    Element(ElementData),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd)]
pub struct NodeId(u64);

#[allow(unused)]
impl NodeId {
    pub(crate) fn is_greater_than(&self, node_id: u64) -> bool {
        self.0 > node_id
    }
    pub(crate) fn is_less_than(&self, node_id: u64) -> bool {
        self.0 < node_id
    }
    pub(crate) fn is_less_than_equal(&self, node_id: u64) -> bool {
        self.0 <= node_id
    }
    pub(crate) fn is_greater_than_equal(&self, node_id: u64) -> bool {
        self.0 >= node_id
    }
    pub(crate) fn is_equal(&self, node_id: u64) -> bool {
        self.0 == node_id
    }
}

impl NodeId {
    pub fn to_u64(&self) -> u64 {
        self.0
    }

    pub const fn new(val: u64) -> Self {
        Self(val)
    }
}

/// RenderNodeId and NodeId are interchangeable. This is a convenience function to convert between the two.
impl From<RenderNodeId> for NodeId {
    fn from(node_id: RenderNodeId) -> Self {
        Self(node_id.to_u64())
    }
}

impl AddAssign<i32> for NodeId {
    fn add_assign(&mut self, rhs: i32) {
        self.0 += rhs as u64;
    }
}

impl std::fmt::Display for NodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "NodeID({})", self.0)
    }
}

#[derive(Clone, Debug)]
pub struct Node {
    pub node_id: NodeId,
    pub parent_id: Option<NodeId>,
    pub node_type: NodeType,
    pub children: Vec<NodeId>,
}

impl Node {

    /// Returns true if the node is a block element
    pub fn is_block_element(&self) -> bool {
        match &self.node_type {
            NodeType::Element(data) => {
                match data.get_style(StyleProperty::Display) {
                    Some(StyleValue::Display(display)) => {
                        *display == Display::Block
                    }
                    _ => false,
                }
            }
            _ => false,
        }
    }

    pub fn is_inline_block_element(&self) -> bool {
        match &self.node_type {
            NodeType::Element(data) => {
                match data.get_style(StyleProperty::Display) {
                    Some(StyleValue::Display(display)) => {
                        *display == Display::InlineBlock
                    }
                    _ => false,
                }
            }
            _ => false,
        }
    }

    /// Returns true if the node is an element node and is inline
    pub fn is_inline_element(&self) -> bool {
        match &self.node_type {
            NodeType::Element(data) => {
                match data.get_style(StyleProperty::Display) {
                    Some(StyleValue::Display(display)) => {
                        *display == Display::Inline
                    }
                    _ => false,
                }
            }
            _ => false,
        }
    }

    /// Returns true when the node is a text node
    pub fn is_text(&self) -> bool {
        match &self.node_type {
            NodeType::Text(_, _) => true,
            _ => false,
        }
    }

    pub fn get_style_f32(&self, prop: StyleProperty) -> f32 {
        match &self.node_type {
            NodeType::Element(data) => {
                match data.get_style(prop) {
                    Some(StyleValue::Unit(px, _)) => *px,
                    Some(StyleValue::Number(px)) => *px,
                    _ => 0.0,
                }
            }
            _ => 0.0,
        }
    }
}

impl Node {
    /// Text nodes also have styles. Normally this is taken from the parent element that the text resides in.
    pub fn new_text(doc: &Document, parent_id: Option<NodeId>, text: String, style: Option<StylePropertyList>) -> Node {
        Node {
            node_id: doc.next_node_id(),
            parent_id,
            children: vec![],
            node_type: NodeType::Text(text, style.unwrap_or(StylePropertyList::new())),
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
        style: Option<StylePropertyList>
    ) -> Node {
        Node {
            node_id: doc.next_node_id(),
            parent_id,
            children: vec![],
            node_type: NodeType::Element(
                ElementData::new(tag_name, attributes, self_closing, style)
            ),
        }
    }
}