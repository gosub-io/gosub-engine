use cow_utils::CowUtils;
use gosub_interface::style::Display;
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
    /// The element's computed `display`, when its own cascade gave it one.
    ///
    /// The whole style of an element lives on its `ComputedStyle`, read through the document.
    /// This one property is carried on the node as well because the layouter's inline-vs-block
    /// grouping walks nodes rather than the document, and because it needs to tell "the cascade
    /// said `inline`" from "the cascade said nothing": the user-agent sheet is incomplete, so an
    /// element with no `display` of its own falls back to what its tag name means.
    pub display: Option<Display>,
}

impl ElementData {
    pub fn new(tag_name: String, attributes: Option<AttrMap>, display: Option<Display>) -> ElementData {
        ElementData {
            tag_name,
            attributes: attributes.unwrap_or_default(),
            display,
        }
    }

    pub fn get_attribute(&self, key: &str) -> Option<&String> {
        self.attributes.get(key)
    }

    pub fn is_inline_element(&self) -> bool {
        matches!(
            self.display,
            None | Some(Display::Inline) | Some(Display::InlineFlex) | Some(Display::InlineGrid)
        )
    }

    pub fn is_inline_block_element(&self) -> bool {
        self.display == Some(Display::InlineBlock)
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
    fn display(&self) -> Option<Display> {
        match &self.node_type {
            NodeType::Element(data) => data.display,
            _ => None,
        }
    }

    pub fn is_block_element(&self) -> bool {
        self.display() == Some(Display::Block)
    }

    pub fn is_inline_block_element(&self) -> bool {
        self.display() == Some(Display::InlineBlock)
    }

    pub fn is_inline_element(&self) -> bool {
        match &self.node_type {
            NodeType::Element(data) => match data.display {
                Some(Display::Inline) => true,
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
