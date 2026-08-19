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
pub mod control_icons;
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

/// A native form control widget: kind, initial state, and the intrinsic size that stands in for
/// what browsers get from their native theme layer.
#[derive(Debug, Clone)]
pub struct ElementContextFormControl {
    pub node_id: DomNodeId,
    pub control: FormControl,
    pub font_info: FontInfo,
    /// Intrinsic content-box size for whichever axis CSS leaves unconstrained (no aspect ratio).
    pub dimension: Dimension,
    pub disabled: bool,
}

#[derive(Debug, Clone)]
pub enum FormControl {
    /// `<input>` text-like types and `<textarea>`.
    TextField {
        /// The markup value; the painter reads what the user typed from the document.
        value: String,
        placeholder: String,
        /// Password bullets.
        masked: bool,
        /// `<textarea>`: wrap and top-align.
        multiline: bool,
        /// Which axes the user may drag-resize (CSS `resize`), with the grip icon to show.
        resize: Resize,
        grip: Option<MediaId>,
    },
    /// `<input type=button/submit/reset/file>`; `<button>` renders its children normally.
    Button {
        label: String,
    },
    /// Checked state comes from the document at paint time; the icons cover all four looks.
    Checkbox(control_icons::ToggleIcons),
    Radio(control_icons::ToggleIcons),
    /// `fraction` = the markup value's position in min..max (0..1); the painter recomputes it
    /// from the live value while the user drags.
    Range {
        min: f64,
        max: f64,
        fraction: f64,
    },
    /// `None` = indeterminate.
    Progress {
        fraction: Option<f64>,
    },
    Meter {
        fraction: f64,
        level: MeterLevel,
    },
    /// Raw `value` attribute, e.g. `#0066cc`.
    ColorSwatch {
        value: String,
    },
    /// Closed dropdown showing the selected option's label.
    Select {
        label: String,
        chevron: Option<MediaId>,
    },
}

/// One row of an open `<select>` dropdown: an option, or an `<optgroup>` label.
#[derive(Debug, Clone)]
pub enum PopupRow {
    Option {
        node_id: DomNodeId,
        label: String,
        disabled: bool,
    },
    Group {
        label: String,
    },
}

impl PopupRow {
    /// An enabled option: can be hovered, keyboard-selected and committed.
    pub fn selectable(&self) -> bool {
        matches!(self, PopupRow::Option { disabled: false, .. })
    }
    pub fn option_id(&self) -> Option<DomNodeId> {
        match self {
            PopupRow::Option { node_id, .. } => Some(*node_id),
            PopupRow::Group { .. } => None,
        }
    }
}

/// An open `<select>` dropdown: a synthetic, out-of-flow element positioned under its select
/// (see [`LayoutTree::popup`]). Which row is selected/hovered is read from the document at paint
/// time.
#[derive(Debug, Clone)]
pub struct ElementContextSelectPopup {
    pub select: DomNodeId,
    pub rows: Vec<PopupRow>,
    pub font_info: FontInfo,
    pub row_height: f64,
    /// Rows the popup shows at once; the list scrolls when it has more.
    pub visible_rows: usize,
    /// Soft drop shadow, rendered as a blurred SVG under the popup.
    pub shadow: Option<MediaId>,
    /// Checkmark drawn on the committed option's row.
    pub check: Option<MediaId>,
    /// Inset between the border and the first/last row.
    pub pad_y: f64,
}

/// Popup chrome per the design guide.
pub const SELECT_POPUP_ROW_HEIGHT: f64 = 30.0;
pub const SELECT_POPUP_MAX_HEIGHT: f64 = 320.0;
pub const SELECT_POPUP_MIN_HEIGHT: f64 = 240.0;
/// How far the shadow reaches past the popup box (left/right, top, bottom).
pub const SELECT_POPUP_SHADOW: (f64, f64, f64) = (12.0, 8.0, 16.0);
/// Inset between the popup border and its first/last row.
pub const SELECT_POPUP_PAD_Y: f64 = 4.0;

/// Where a dropdown with `rows` rows opens for a select at `anchor` (border box, page px) given
/// the viewport `(top, height)` in page px: `(opens above, rows shown)`. Below unless the space
/// there is short of the minimum height and there is more room above; rows are whatever fits in
/// the lesser of max-height and that space.
pub fn popup_placement(anchor: crate::common::geo::Rect, viewport: (f64, f64), rows: usize) -> (bool, usize) {
    let (_, st, sb) = SELECT_POPUP_SHADOW;
    let chrome = SELECT_POPUP_PAD_Y * 2.0 + 2.0;
    let (vp_top, vp_h) = (viewport.0, viewport.1.max(1.0));
    let below = (vp_top + vp_h - (anchor.y + anchor.height) - sb - 4.0).max(0.0);
    let above = (anchor.y - vp_top - st - 4.0).max(0.0);
    let wanted = SELECT_POPUP_ROW_HEIGHT * rows as f64 + chrome;
    let open_above = below < SELECT_POPUP_MIN_HEIGHT.min(wanted) && above > below;
    let space = if open_above { above } else { below };
    let avail = SELECT_POPUP_MAX_HEIGHT
        .min(space)
        .max(SELECT_POPUP_ROW_HEIGHT * 2.0 + chrome);
    let visible = (((avail - chrome) / SELECT_POPUP_ROW_HEIGHT).floor() as usize).clamp(1, rows.max(1));
    (open_above, visible)
}

impl ElementContextSelectPopup {
    pub fn scrolls(&self) -> bool {
        self.rows.len() > self.visible_rows
    }

    /// Width of the scrollbar strip at the popup's right edge (0 when the list fits).
    pub fn scrollbar_width(&self) -> f64 {
        if self.scrolls() {
            10.0
        } else {
            0.0
        }
    }

    /// Highest `first_row` value.
    pub fn max_first_row(&self) -> usize {
        self.rows.len().saturating_sub(self.visible_rows)
    }

    /// Scrollbar track and thumb for a given `first_row`, inside the popup's padding box.
    pub fn scrollbar(
        &self,
        inner: crate::common::geo::Rect,
        first_row: usize,
    ) -> Option<(crate::common::geo::Rect, crate::common::geo::Rect)> {
        if !self.scrolls() {
            return None;
        }
        let bar_w = self.scrollbar_width();
        let track = crate::common::geo::Rect::new(inner.x + inner.width - bar_w, inner.y, bar_w, inner.height);
        let thumb_h = (inner.height * self.visible_rows as f64 / self.rows.len() as f64).max(12.0);
        let travel = inner.height - thumb_h;
        let thumb_y = track.y + travel * (first_row as f64 / self.max_first_row().max(1) as f64);
        Some((
            track,
            crate::common::geo::Rect::new(track.x + 1.0, thumb_y, bar_w - 2.0, thumb_h),
        ))
    }
}

/// CSS `resize` on a text control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resize {
    None,
    Both,
    Horizontal,
    Vertical,
}

impl Resize {
    pub fn from_keyword(k: &str) -> Resize {
        match k {
            "both" => Resize::Both,
            "horizontal" | "inline" => Resize::Horizontal,
            "vertical" | "block" => Resize::Vertical,
            _ => Resize::None,
        }
    }
}

/// Meter color band: green / yellow / red.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeterLevel {
    Optimum,
    Suboptimum,
    Critical,
}

/// Per-element data (text, image, svg, form control) needed by later phases of the rendering pipeline.
#[derive(Debug, Clone)]
pub enum ElementContext {
    None,
    Text(ElementContextText),
    Image(ElementContextImage),
    Svg(ElementContextSvg),
    FormControl(ElementContextFormControl),
    SelectPopup(ElementContextSelectPopup),
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
    /// The open `<select>` dropdown, if any: not attached to the tree, gets its own top layer.
    pub popup: Option<LayoutElementId>,
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
