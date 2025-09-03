use std::collections::HashMap;
use std::ops::AddAssign;
use std::sync::{Arc, RwLock};
use rstar::primitives::GeomWithData;
use gosub_interface::config::HasDocument;
use gosub_shared::node::NodeId;
use crate::layouter::box_model::BoxModel;
use crate::rendertree_builder::{RenderTree, RenderNodeId};
use crate::common::font::FontInfo;
use crate::common::geo::{Coordinate, Dimension};
use crate::common::media::MediaId;

pub mod taffy;
pub mod text;
mod box_model;
mod css_taffy_converter;

/// ID's for layout elements
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LayoutElementId(u64);

impl LayoutElementId {
    pub const fn new(val: u64) -> Self {
        Self(val)
    }
}

impl AddAssign<i32> for LayoutElementId {
    fn add_assign(&mut self, rhs: i32) {
        self.0 += rhs as u64;
    }
}

impl std::fmt::Display for LayoutElementId {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "LayoutElementId({})", self.0)
    }
}

/// Context for an element. It contains all information to paint the element.
#[derive(Debug, Clone)]
pub struct ElementContextText {
    /// Node ID of the text in the DOM
    pub node_id: NodeId,
    pub font_info: FontInfo,
    pub text: String,
    /// Additional offset for the text. This can happen when we have a lineheight and the text needs to be centered in the block
    pub text_offset: Coordinate,
}

#[derive(Debug, Clone)]
pub struct ElementContextSvg {
    /// Node ID of the SVG in the DOM
    pub node_id: NodeId,
    /// Source of the SVG
    pub src: String,
    /// ID of the SVG inside the media store
    pub media_id: MediaId,
    /// Dimension of the SVG. Can be Dimension::ZERO if not known yet
    pub dimension: Dimension,
}

#[derive(Clone, Debug)]
pub struct ElementContextImage {
    /// Node ID of the image in the DOM
    pub node_id: NodeId,
    /// Source of the image
    pub src: String,
    /// ID of the image inside the image store
    pub media_id: MediaId,
    /// Dimension of the image. Can be Dimension::ZERO if not known yet
    pub dimension: Dimension,
}

/// Information about the given element that is needed for different phases of the rendering pipeline. For instance,
/// image or text information.
#[derive(Debug, Clone)]
pub enum ElementContext {
    None,
    Text(ElementContextText),
    Image(ElementContextImage),
    Svg(ElementContextSvg)
}

impl ElementContext {
    pub(crate) fn text(text: &str, font_info: FontInfo, node_id: NodeId, text_offset: Coordinate) -> ElementContext {
        Self::Text(ElementContextText{
            text: text.to_string(),
            font_info,
            node_id,
            text_offset,
        })
    }

    pub fn image(src: &str, media_id: MediaId, dimension: Dimension, node_id: NodeId) -> ElementContext {
        Self::Image(ElementContextImage {
            node_id,
            src: src.to_string(),
            media_id,
            dimension,
        })
    }

    pub fn svg(src: &str, media_id: MediaId, dimension: Dimension, node_id: NodeId) -> ElementContext {
        Self::Svg(ElementContextSvg {
            node_id,
            src: src.to_string(),
            media_id,
            dimension,
        })
    }

}

#[derive(Debug, Clone)]
pub struct LayoutElementNode {
    pub id: LayoutElementId,
    /// ID of the node in the DOM, contains the data, like element name, attributes, etc.
    pub dom_node_id: NodeId,
    /// ID of the node in the render tree. This is normally the same node ID as the dom node ID
    pub render_node_id: RenderNodeId,
    /// Children of this node
    pub children: Vec<LayoutElementId>,
    /// Generated box model for this node
    pub box_model: BoxModel,
    /// Element context. Used by different parts of the render engine
    pub context: ElementContext,
}

pub struct LayoutTree<C: HasDocument> {
    /// Wrapped render tree
    pub render_tree: RenderTree<C>,
    /// Arena of layout nodes
    pub arena : HashMap<LayoutElementId, LayoutElementNode>,
    /// Root node of the layout tree
    pub root_id: LayoutElementId,
    /// Next node ID
    next_node_id: Arc<RwLock<LayoutElementId>>,
    // Root width and height
    pub root_dimension: Dimension,
    /// R* tree for fast spatial queries of layout elements
    rstar_tree: rstar::RTree<GeomWithData<rstar::primitives::Rectangle<[f64; 2]>, LayoutElementId>>,
}

impl<C: HasDocument> LayoutTree<C> {
    pub fn get_node_by_id(&self, node_id: LayoutElementId) -> Option<&LayoutElementNode> {
        self.arena.get(&node_id)
    }

    pub fn get_node_by_id_mut(&mut self, node_id: LayoutElementId) -> Option<&mut LayoutElementNode> {
        self.arena.get_mut(&node_id)
    }

    pub fn next_node_id(&self) -> LayoutElementId {
        let mut nid = self.next_node_id.write().expect("Failed to lock next node ID");
        let id = *nid;
        *nid += 1;
        id
    }
}

impl<C: HasDocument> std::fmt::Debug for LayoutTree<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LayoutTree")
            .field("arena", &self.arena)
            .field("root_id", &self.root_id)
            .field("root_dimension", &self.root_dimension)
            .finish()
    }
}

/// A layout engine should implement this trait and return a layout tree
pub trait CanLayout<C: HasDocument> {
    fn layout(&mut self, render_tree: RenderTree<C>, viewport: Option<Dimension>, dpi_scale_factor: f32) -> LayoutTree<C>;
}