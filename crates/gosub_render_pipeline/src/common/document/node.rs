use crate::common::document::style::{Display, NodeStyle, StyleProperty, Value};
use cow_utils::CowUtils;
use std::collections::HashMap;

pub use gosub_shared::node::NodeId;

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

#[derive(Clone, Debug)]
pub struct ElementData {
    pub tag_name: String,
    pub attributes: AttrMap,
    /// Own CSS properties for this element (only explicitly-set values; no inheritance).
    pub styles: NodeStyle,
}

impl ElementData {
    pub fn new(tag_name: String, attributes: Option<AttrMap>, styles: Option<NodeStyle>) -> ElementData {
        ElementData {
            tag_name,
            attributes: attributes.unwrap_or_default(),
            styles: styles.unwrap_or_default(),
        }
    }

    pub fn get_style(&self, key: &StyleProperty) -> Option<&Value> {
        self.styles.get_own(key)
    }

    pub fn get_attribute(&self, key: &str) -> Option<&String> {
        self.attributes.get(key)
    }

    pub fn is_inline_element(&self) -> bool {
        matches!(
            self.get_style(&StyleProperty::Display),
            None | Some(Value::Display(Display::Inline))
                | Some(Value::Display(Display::InlineFlex))
                | Some(Value::Display(Display::InlineGrid))
        )
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
    /// Text node - no own style; all properties inherited via the document parent chain.
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
                matches!(
                    data.get_style(&StyleProperty::Display),
                    Some(Value::Display(Display::Block))
                )
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
            NodeType::Element(data) => match data.get_style(&StyleProperty::Display) {
                Some(Value::Display(Display::Inline)) => true,
                // The CSS initial value is `inline`, but UA stylesheets make most structural
                // elements `block` - defaulting to inline would group <li>, <h2>, <div> etc.
                // into inline flows, so fall back to the tag's intrinsic type.
                None => is_intrinsically_inline(&data.tag_name),
                _ => false,
            },
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

/// Inline-by-spec HTML elements. Block-level tags return false so a missing `display` (e.g. a
/// UA-stylesheet gap) never drops them into an inline formatting context.
fn is_intrinsically_inline(tag: &str) -> bool {
    matches!(
        tag.cow_to_ascii_lowercase().as_ref(),
        "a" | "abbr"
            | "acronym"
            | "b"
            | "bdo"
            | "big"
            | "br"
            | "button"
            | "cite"
            | "code"
            | "dfn"
            | "em"
            | "i"
            | "img"
            | "input"
            | "kbd"
            | "label"
            | "map"
            | "object"
            | "output"
            | "q"
            | "samp"
            | "select"
            | "small"
            | "span"
            | "strong"
            | "sub"
            | "sup"
            | "textarea"
            | "time"
            | "tt"
            | "u"
            | "var"
    )
}
