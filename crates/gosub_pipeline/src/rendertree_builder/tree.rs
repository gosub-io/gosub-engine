use crate::common::document::node::Node;
use crate::common::document::pipeline_doc::{PipelineDocument, PipelineNodeKind};
use gosub_shared::node::NodeId;
use std::collections::HashMap;
use std::ops::AddAssign;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RenderNodeId(u64);

impl RenderNodeId {
    pub const fn new(val: u64) -> Self {
        Self(val)
    }

    pub fn to_u64(&self) -> u64 {
        self.0
    }
}

impl From<NodeId> for RenderNodeId {
    fn from(node_id: NodeId) -> Self {
        Self(u64::from(node_id))
    }
}

impl From<RenderNodeId> for NodeId {
    fn from(id: RenderNodeId) -> Self {
        NodeId::from(id.0)
    }
}

impl AddAssign<u64> for RenderNodeId {
    fn add_assign(&mut self, rhs: u64) {
        self.0 += rhs;
    }
}

impl std::fmt::Display for RenderNodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "RenderNodeID({})", self.0)
    }
}

#[derive(Clone)]
pub struct RenderNode {
    pub node_id: RenderNodeId,
    pub children: Vec<RenderNodeId>,
}

/// A RenderTree holds a filtered view of the DOM — only the nodes that should be rendered.
#[derive(Clone)]
pub struct RenderTree {
    pub doc: Arc<dyn PipelineDocument>,
    pub arena: HashMap<RenderNodeId, RenderNode>,
    pub root_id: Option<RenderNodeId>,
}

impl std::fmt::Debug for RenderTree {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RenderTree").field("root_id", &self.root_id).finish()
    }
}

impl RenderTree {
    pub fn count_elements(&self) -> usize {
        self.arena.len()
    }

    pub fn print(&self) {
        match self.root_id {
            Some(root_id) => self.print_node(root_id, 0),
            None => println!("No root node"),
        }
    }

    pub fn get_node_by_id(&self, node_id: RenderNodeId) -> Option<&RenderNode> {
        self.arena.get(&node_id)
    }

    pub fn get_document_node_by_render_id(&self, render_id: RenderNodeId) -> Option<Node> {
        self.doc.get_node_by_id(NodeId::from(render_id))
    }

    fn print_node(&self, node_id: RenderNodeId, level: usize) {
        let Some(node) = self.get_node_by_id(node_id) else {
            return;
        };

        let indent = " ".repeat(level * 4);
        println!("{}{}", indent, node.node_id);
        for child_id in &node.children {
            self.print_node(*child_id, level + 1);
        }
    }
}

const INVISIBLE_ELEMENTS: [&str; 6] = ["head", "style", "script", "meta", "link", "title"];

impl RenderTree {
    pub fn new(doc: Arc<dyn PipelineDocument>) -> Self {
        RenderTree {
            doc,
            arena: HashMap::new(),
            root_id: None,
        }
    }

    pub fn parse(&mut self) {
        self.arena.clear();
        self.root_id = None;

        let Some(root_id) = self.doc.root() else {
            panic!("Document has no root node");
        };

        match self.build_rendertree(root_id) {
            Some(render_node_id) => self.root_id = Some(render_node_id),
            None => panic!("Failed to build rendertree"),
        }
    }

    fn is_visible(&self, id: NodeId) -> bool {
        match self.doc.node_kind(id) {
            PipelineNodeKind::Comment => false,
            PipelineNodeKind::Text => true,
            PipelineNodeKind::Element => {
                if let Some(tag) = self.doc.tag_name(id) {
                    if INVISIBLE_ELEMENTS.contains(&tag.as_str()) {
                        return false;
                    }
                }
                !self.doc.is_display_none(id)
            }
        }
    }

    fn build_rendertree(&mut self, node_id: NodeId) -> Option<RenderNodeId> {
        if !self.is_visible(node_id) {
            return None;
        }

        let mut render_node = RenderNode {
            node_id: RenderNodeId::from(node_id),
            children: Vec::new(),
        };

        let children = self.doc.children(node_id);
        for child_id in children {
            if let Some(render_child) = self.build_rendertree(child_id) {
                render_node.children.push(render_child);
            }
        }

        let render_node_id = render_node.node_id;
        self.arena.insert(render_node_id, render_node);

        Some(render_node_id)
    }
}
