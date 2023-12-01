use gosub_engine::render_tree::{text::TextNode, Position};

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
    pub data: CNodeData,
}

impl Default for CNode {
    fn default() -> Self {
        Self {
            tag: CNodeType::Root,
            position: Position::new(),
            data: CNodeData::Root(true),
        }
    }
}

impl CNode {
    pub fn new_root() -> Self {
        Self::default()
    }

    pub fn new_text(position: &Position, text_node: &TextNode) -> Self {
        Self {
            tag: CNodeType::Text,
            position: (*position).clone(),
            data: CNodeData::Text(CTextNode::from(text_node)),
        }
    }
}
