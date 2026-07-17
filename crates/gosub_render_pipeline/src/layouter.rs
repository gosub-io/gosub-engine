use crate::common::document::node::NodeId as DomNodeId;
use crate::common::font::FontInfo;
use crate::common::geo::{Coordinate, Dimension};
use crate::common::media::MediaId;
use crate::layouter::box_model::BoxModel;
use crate::rendertree_builder::{RenderNodeId, RenderTree};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::ops::AddAssign;
use std::sync::Arc;

mod box_model;
mod css_taffy_converter;
mod inline_run;
pub mod table;
pub mod taffy;
pub mod text;

/// ID's for layout elements
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LayoutElementId(u64);

impl LayoutElementId {
    pub const fn new(val: u64) -> Self {
        Self(val)
    }
    pub fn as_u64(self) -> u64 {
        self.0
    }
}

impl AddAssign<u64> for LayoutElementId {
    fn add_assign(&mut self, rhs: u64) {
        self.0 += rhs;
    }
}

impl std::fmt::Display for LayoutElementId {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "LayoutElementId({})", self.0)
    }
}

#[derive(Debug, Clone)]
pub struct ElementContextText {
    pub node_id: DomNodeId,
    pub font_info: FontInfo,
    pub text: String,
    /// Offset that centers the text in the block when line-height exceeds the font size.
    pub text_offset: Coordinate,
    /// When true (white-space: nowrap), the text is measured at unlimited width and must not wrap.
    pub no_wrap: bool,
    /// The definite container width (CSS px) that Parley received as its max_width during layout.
    /// Renderers must use this - not content_box.width - as the word-wrap limit to avoid metric
    /// mismatches between Parley (layout) and the rendering backend (e.g. Skia).
    pub available_width: f64,
}

#[derive(Debug, Clone)]
pub struct ElementContextSvg {
    pub node_id: DomNodeId,
    pub src: String,
    pub media_id: MediaId,
    /// `Dimension::ZERO` when not known yet.
    pub dimension: Dimension,
}

#[derive(Clone, Debug)]
pub struct ElementContextImage {
    pub node_id: DomNodeId,
    pub src: String,
    pub media_id: MediaId,
    /// `Dimension::ZERO` when not known yet.
    pub dimension: Dimension,
    /// True when `media_id` is a fallback broken-image placeholder (the real image failed to
    /// load). The painter draws the icon at its natural `dimension` in the top-left of the
    /// reserved box instead of stretching it to fill.
    pub placeholder: bool,
    /// The `alt` text to render inside the image box, `Some` only when the image itself shows
    /// nothing meaningful - a broken/placeholder load, or a fully transparent image. Browsers
    /// display alt text in these cases (never over a normally-decoded, visible image).
    pub alt: Option<String>,
}

/// Per-element data (text, image, svg) needed by later phases of the rendering pipeline.
#[derive(Debug, Clone)]
pub enum ElementContext {
    None,
    Text(ElementContextText),
    Image(ElementContextImage),
    Svg(ElementContextSvg),
}

impl ElementContext {
    pub(crate) fn text(
        text: &str,
        font_info: FontInfo,
        node_id: DomNodeId,
        text_offset: Coordinate,
        no_wrap: bool,
    ) -> ElementContext {
        Self::Text(ElementContextText {
            text: text.to_string(),
            font_info,
            node_id,
            text_offset,
            no_wrap,
            available_width: 0.0,
        })
    }

    pub fn image(
        src: &str,
        media_id: MediaId,
        dimension: Dimension,
        node_id: DomNodeId,
        placeholder: bool,
        alt: Option<String>,
    ) -> ElementContext {
        Self::Image(ElementContextImage {
            node_id,
            src: src.to_string(),
            media_id,
            dimension,
            placeholder,
            alt,
        })
    }

    pub fn svg(src: &str, media_id: MediaId, dimension: Dimension, node_id: DomNodeId) -> ElementContext {
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
    /// Holds the element data: name, attributes, etc.
    pub dom_node_id: DomNodeId,
    /// Normally the same node ID as the dom node ID.
    pub render_node_id: RenderNodeId,
    /// `None` for the root. Used to walk up to an element's containing block (e.g. the cage for
    /// `position: sticky`).
    pub parent: Option<LayoutElementId>,
    pub children: Vec<LayoutElementId>,
    pub box_model: BoxModel,
    pub context: ElementContext,
    /// Resolved CSS `background-image`, loaded into the media store during layout.
    pub background_media: Option<BackgroundMedia>,
}

/// A resolved CSS `background-image` and its media kind. The painter finalizes tile geometry once
/// the border box is known. A tiling SVG is rasterized to an `Image` during layout so only one
/// tiling path exists downstream; a `cover`/`contain` SVG stays `Svg`.
#[derive(Debug, Clone, Copy)]
pub enum BackgroundMedia {
    Image {
        media_id: MediaId,
        /// Intrinsic image size in px (for a rasterized SVG tile, the tile's pixel size).
        natural: (f32, f32),
        layout: crate::common::document::pipeline_doc::BgImageLayout,
    },
    Svg(MediaId),
}

#[derive(Clone)]
pub struct LayoutTree {
    pub render_tree: RenderTree,
    pub arena: HashMap<LayoutElementId, LayoutElementNode>,
    pub root_id: LayoutElementId,
    next_node_id: Arc<RwLock<LayoutElementId>>,
    pub root_dimension: Dimension,
}

impl LayoutTree {
    pub fn get_node_by_id(&self, node_id: LayoutElementId) -> Option<&LayoutElementNode> {
        self.arena.get(&node_id)
    }

    pub fn get_node_by_id_mut(&mut self, node_id: LayoutElementId) -> Option<&mut LayoutElementNode> {
        self.arena.get_mut(&node_id)
    }

    pub fn next_node_id(&self) -> LayoutElementId {
        let mut nid = self.next_node_id.write();
        let id = *nid;
        *nid += 1;
        id
    }
}

impl std::fmt::Debug for LayoutTree {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LayoutTree")
            .field("arena", &self.arena)
            .field("root_id", &self.root_id)
            .field("root_dimension", &self.root_dimension)
            .finish()
    }
}

/// A layout engine should implement this trait and return a layout tree
pub trait CanLayout {
    fn layout(&mut self, render_tree: RenderTree, viewport: Option<Dimension>, dpi_scale_factor: f32) -> LayoutTree;
}
