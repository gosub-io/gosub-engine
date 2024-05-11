use gosub_rendering::render_tree::properties::{Position, Rectangle};
use gosub_rendering::render_tree::{text::TextNode, Node};

use crate::wrapper::{text::CTextNode, CNodeType};

#[repr(C, u32)]
pub enum CNodeData {
    Root(bool),
    Text(CTextNode),
}

#[repr(C)]
pub struct CNode {
    pub tag: CNodeType,
    pub position: Position,
    pub margin: Rectangle,
    pub padding: Rectangle,
    pub data: CNodeData,
}

impl Default for CNode {
    fn default() -> Self {
        Self {
            tag: CNodeType::Root,
            position: Position::new(),
            margin: Rectangle::new(),
            padding: Rectangle::new(),
            data: CNodeData::Root(true),
        }
    }
}

impl CNode {
    #[must_use]
    pub fn new_root() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn new_text(node: &Node, text_node: &TextNode) -> Self {
        Self {
            tag: CNodeType::Text,
            position: node.position.clone(),
            margin: node.margin.clone(),
            padding: node.padding.clone(),
            data: CNodeData::Text(CTextNode::from(text_node)),
        }
    }
}
