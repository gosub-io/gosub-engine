use std::collections::HashMap;
use std::ops::AddAssign;
use std::sync::Arc;
use crate::common::document::document::Document;
use crate::common::document::node::{Node, NodeType, NodeId};
use crate::common::document::style::{StyleProperty, StyleValue, Display as CssDisplay};

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
        Self(node_id.to_u64())
    }
}

impl AddAssign<i32> for RenderNodeId {
    fn add_assign(&mut self, rhs: i32) {
        self.0 += rhs as u64;
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


/// A RenderTree holds both the DOM and the render tree. This tree holds all the visible nodes in
/// the DOM.
#[derive(Clone)]
pub struct RenderTree {
    pub doc: Arc<Document>,
    pub arena: HashMap<RenderNodeId, RenderNode>,
    pub root_id: Option<RenderNodeId>,
}

impl std::fmt::Debug for RenderTree {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RenderTree")
            // .field("arena", &self.arena)
            .field("root_id", &self.root_id)
            .finish()
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

    pub(crate) fn get_document_node_by_render_id(&self, render_node_id: RenderNodeId) -> Option<&Node> {
        let Some(node) = self.arena.get(&render_node_id) else {
            return None;
        };

        let Some(doc_node) = self.doc.get_node_by_id(NodeId::new(node.node_id.to_u64())) else {
            return None;
        };

        Some(doc_node)
    }
}

const INVISIBLE_ELEMENTS: [&str; 6] = [ "head",  "style",  "script",  "meta",  "link",  "title" ];

impl RenderTree {
    pub fn new(doc: Arc<Document>) -> Self {
        RenderTree {
            doc: doc.clone(),
            arena: HashMap::new(),
            root_id: None,
        }
    }

    pub fn parse(&mut self) {
        let Some(root_id) = self.doc.root_id else {
            panic!("Document has no root node");
        };

        let doc = &self.doc;
        match self.build_rendertree(root_id) {
            Some(render_node_id) => self.root_id = Some(render_node_id),
            None => panic!("Failed to build rendertree"),
        }
    }

    fn is_visible(&self, node: &Node) -> bool {
        match &node.node_type {
            NodeType::Comment(..) => false,
            NodeType::Text(..) => true,
            NodeType::Element(element) => {
                // Check element name
                if INVISIBLE_ELEMENTS.contains(&element.tag_name.as_str()) {
                    return false;
                }

                match element.get_style(StyleProperty::Display) {
                    Some(StyleValue::Display(display)) => {
                        if *display == CssDisplay::None {
                            return false;
                        }
                    }
                    _ => {}
                }

                true
            }
        }
    }

    fn build_rendertree(&mut self, node_id: NodeId) -> Option<RenderNodeId> {
        let Some(node) = self.doc.get_node_by_id(node_id) else {
            return None;
        };

        if !self.is_visible(node) {
            return None;
        }

        let mut render_node = RenderNode {
            node_id: RenderNodeId::from(node_id),
            children: Vec::new(),
        };

        let children = node.children.clone();
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