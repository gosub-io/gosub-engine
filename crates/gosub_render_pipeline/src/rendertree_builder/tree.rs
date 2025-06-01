use std::collections::HashMap;
use std::ops::AddAssign;
use std::process::id;
use std::sync::Arc;
use gosub_interface::config::{HasDocument, ModuleConfiguration};
use gosub_interface::document::Document;
use gosub_interface::node::{ElementDataType, Node, NodeType};
use crate::common::style::{StyleProperty, StyleValue, Display as CssDisplay};
use gosub_shared::node::NodeId;

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
pub struct RenderNode<C: HasDocument> {
    pub node_id: RenderNodeId,
    pub children: Vec<RenderNodeId>,
    _marker: std::marker::PhantomData<C>,
}

/// A RenderTree holds both the DOM and the render tree. This tree holds all the visible nodes in
/// the DOM.
#[derive(Clone)]
pub struct RenderTree<C: HasDocument> {
    pub doc: Arc<C::Document>,
    pub arena: HashMap<RenderNodeId, RenderNode<C>>,
    pub root_id: Option<RenderNodeId>,
}

impl<C:HasDocument> RenderTree<C> {
    pub fn new(doc: Arc<C::Document>) -> Self {
        RenderTree {
            doc,
            arena: HashMap::new(),
            root_id: None,
        }
    }

    pub fn count_elements(&self) -> usize {
        self.arena.len()
    }

    pub fn print(&self) {
        match self.root_id {
            Some(root_id) => self.print_node(root_id, 0),
            None => println!("No root node"),
        }
    }

    pub fn get_node_by_id(&self, node_id: RenderNodeId) -> Option<&RenderNode<C>> {
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

    pub fn get_document_node_by_render_id(&self, render_node_id: RenderNodeId) -> Option<&C::Node> {
        let Some(node) = self.arena.get(&render_node_id) else {
            return None;
        };

        self.doc.node_by_id(NodeId::new(node.node_id.to_u64()))
    }

    pub fn parse(&mut self) {
        let root_node = self.doc.get_root();

        let doc = &self.doc;
        match self.build_rendertree(root_node.id()) {
            Some(render_node_id) => self.root_id = Some(render_node_id),
            None => panic!("Failed to build rendertree"),
        }
    }

    fn is_visible(&self, node: &C::Node) -> bool {
        match node.type_of() {
            NodeType::CommentNode => false,
            NodeType::TextNode => true,
            NodeType::ElementNode => {
                let Some(element) = node.get_element_data() else {
                    return false; // Not an element node
                };

                // Check element name
                if INVISIBLE_ELEMENTS.contains(&element.name()) {
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
            NodeType::DocumentNode => false,
            NodeType::DocTypeNode => false,
        }
    }

    fn build_rendertree(&mut self, node_id: NodeId) -> Option<RenderNodeId> {
        let Some(node) = self.doc.node_by_id(node_id) else {
            return None;
        };

        if !self.is_visible(node) {
            return None;
        }

        let mut render_node = RenderNode {
            node_id: RenderNodeId::from(node_id),
            children: Vec::new(),
            _marker: std::marker::PhantomData,
        };

        let children = node.children().clone();
        for child_id in children {
            if let Some(render_child) = self.build_rendertree(*child_id) {
                render_node.children.push(render_child);
            }
        }

        let render_node_id = render_node.node_id;
        self.arena.insert(render_node_id, render_node);

        Some(render_node_id)
    }
}

impl<C: ModuleConfiguration> std::fmt::Debug for RenderTree<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RenderTree")
            .field("root_id", &self.root_id)
            .finish()
    }
}

// Invisible elements in the DOM that should never be rendered
const INVISIBLE_ELEMENTS: [&str; 6] = ["head", "style", "script", "meta", "link", "title"];