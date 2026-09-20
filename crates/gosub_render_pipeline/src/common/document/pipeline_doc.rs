use crate::common::document::node::{AttrMap, ElementData, Node, NodeType};
use crate::common::document::presentation_hints;
use crate::painter::commands::color::Color;
use crate::painter::commands::gradient::{ColorStop, Gradient, LinearGradient, Tiling};
use cow_utils::CowUtils;
use gosub_interface::config::HasDocument;
use gosub_interface::css3::{CssOrigin, CssProperty, CssPropertyMap, CssSystem, CssValue};
use gosub_interface::document::Document as _;
use gosub_interface::node::NodeType as GosubNodeType;
use gosub_interface::style::{ComputedStyle, Display, LengthPercentage, Prop};
use gosub_shared::node::NodeId;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;

// ── Gradient parsing ──────────────────────────────────────────────────────────

/// Parses `linear-gradient(...)` args: an optional leading direction (`to <side>[ <side>]` or an
/// `<angle>`) then two or more stops. Positionless stops are spread evenly between neighbours.
fn parse_linear_gradient<S: CssSystem>(args: &[S::Value]) -> Option<Gradient> {
    let mut groups: Vec<Vec<&S::Value>> = Vec::new();
    let mut current: Vec<&S::Value> = Vec::new();
    for a in args {
        if a.is_comma() {
            groups.push(std::mem::take(&mut current));
        } else {
            current.push(a);
        }
    }
    groups.push(current);

    // An optional direction occupies the first group when it carries no colour.
    let mut angle_deg = 180.0_f32; // CSS default direction is `to bottom`.
    let mut first_stop = 0;
    if let Some(first) = groups.first() {
        if let Some(angle) = parse_gradient_direction::<S>(first) {
            angle_deg = angle;
            first_stop = 1;
        }
    }

    let mut colors: Vec<Color> = Vec::new();
    let mut offsets: Vec<Option<f32>> = Vec::new();
    for group in groups.iter().skip(first_stop) {
        // Named colours and `transparent` tokenise as plain identifiers, so `as_color()` misses
        // them - fall back to string parsing, which `#e6e6e6 25%, transparent 25%` relies on.
        let color = group
            .iter()
            .find_map(|v| v.as_color())
            .map(|(r, g, b, a)| Color::from_rgba(r / 255.0, g / 255.0, b / 255.0, a / 255.0))
            .or_else(|| group.iter().find_map(|v| v.as_string()).and_then(Color::try_from_css));
        let Some(color) = color else {
            continue;
        };
        colors.push(color);
        offsets.push(group.iter().find_map(|v| v.as_percentage()).map(|p| p / 100.0));
    }
    let n = colors.len();
    if n < 2 {
        return None;
    }

    // Anchor the endpoints, then linearly interpolate any interior gaps.
    if offsets[0].is_none() {
        offsets[0] = Some(0.0);
    }
    if offsets[n - 1].is_none() {
        offsets[n - 1] = Some(1.0);
    }
    let mut i = 0;
    while i < n {
        if offsets[i].is_some() {
            i += 1;
            continue;
        }
        let start = i - 1; // resolved (endpoints are anchored)
        let mut end = i;
        while end < n && offsets[end].is_none() {
            end += 1;
        }
        let a = offsets[start].unwrap_or(0.0);
        let b = offsets.get(end).and_then(|o| *o).unwrap_or(1.0);
        let steps = (end - start) as f32;
        for (k, slot) in offsets.iter_mut().enumerate().take(end).skip(start + 1) {
            *slot = Some(a + (b - a) * ((k - start) as f32) / steps);
        }
        i = end;
    }

    // Clamp to [0,1] and keep positions non-decreasing (CSS gradient rule).
    let mut running = 0.0_f32;
    let stops = colors
        .into_iter()
        .zip(offsets)
        .map(|(color, off)| {
            let off = off.unwrap_or(0.0).clamp(0.0, 1.0).max(running);
            running = off;
            ColorStop { offset: off, color }
        })
        .collect();

    Some(Gradient::Linear(LinearGradient {
        angle_deg,
        stops,
        tiling: None,
    }))
}

/// Gradient-line angle in CSS degrees, or `None` if the group is a colour stop rather than a
/// direction (so the gradient uses the default `to bottom`).
fn parse_gradient_direction<S: CssSystem>(group: &[&S::Value]) -> Option<f32> {
    // Angle form: `45deg`, `0.25turn`, `1.5rad`, `100grad`.
    if let Some((v, unit)) = group.first().and_then(|first| first.as_unit()) {
        return match unit {
            "deg" => Some(v),
            "grad" => Some(v * 0.9),
            "rad" => Some(v.to_degrees()),
            "turn" => Some(v * 360.0),
            _ => None,
        };
    }
    // Keyword form: `to <side> [<side>]`.
    let words: Vec<String> = group
        .iter()
        .filter_map(|v| v.as_string())
        .map(|s| s.cow_to_lowercase().into_owned())
        .collect();
    if words.first().map(String::as_str) != Some("to") {
        return None;
    }
    let has = |k: &str| words.iter().any(|w| w == k);
    Some(match (has("top"), has("right"), has("bottom"), has("left")) {
        (true, false, false, false) => 0.0,
        (false, true, false, false) => 90.0,
        (false, false, false, true) => 270.0,
        (true, true, false, false) => 45.0,
        (false, true, true, false) => 135.0,
        (false, false, true, true) => 225.0,
        (true, false, false, true) => 315.0,
        // `to bottom` and any unrecognised combination fall back to a downward gradient.
        _ => 180.0,
    })
}

/// All `linear-gradient(...)` layers of a `background-image` property, in source order (the
/// first listed layer paints on top). Non-gradient layers (`url()`, `none`) are skipped.
fn property_gradient_layers<S: CssSystem>(p: &S::Property) -> Vec<LinearGradient> {
    let mut out = Vec::new();
    let mut push_fn = |name: &str, args: &[S::Value]| {
        if name.eq_ignore_ascii_case("linear-gradient") {
            if let Some(Gradient::Linear(g)) = parse_linear_gradient::<S>(args) {
                out.push(g);
            }
        }
    };
    if let Some((name, args)) = p.as_function() {
        push_fn(name, args);
        return out;
    }
    if let Some(list) = p.as_list() {
        for v in list {
            if let Some((name, args)) = v.as_function() {
                push_fn(name, args);
            }
        }
    }
    out
}

/// One resolved token from a `background-size`/`-position`/`-repeat` value.
enum BgTok {
    /// A `<length>` in px (bare `0` included).
    Len(f32),
    /// A `<percentage>` (0..100). A percentage *size* still falls back to "fill the box"; a
    /// percentage *position* is resolved against the box at paint time.
    Pct(f32),
    /// A keyword (`cover`, `center`, `no-repeat`, ...), lowercased.
    Kw(String),
}

fn value_bg_tok<S: CssSystem>(v: &S::Value) -> Option<BgTok> {
    if let Some((val, unit)) = v.as_unit() {
        if unit.eq_ignore_ascii_case("px") {
            return Some(BgTok::Len(val));
        }
    }
    if let Some(pct) = v.as_percentage() {
        return Some(BgTok::Pct(pct));
    }
    if let Some(n) = v.as_number() {
        if n == 0.0 {
            return Some(BgTok::Len(0.0)); // bare `0`
        }
    }
    v.as_string()
        .map(|s| BgTok::Kw(s.cow_to_ascii_lowercase().into_owned()))
}

fn prop_bg_tok<S: CssSystem>(p: &S::Property) -> Option<BgTok> {
    if let Some((val, unit)) = p.as_unit() {
        if unit.eq_ignore_ascii_case("px") {
            return Some(BgTok::Len(val));
        }
    }
    if let Some(pct) = p.as_percentage() {
        return Some(BgTok::Pct(pct));
    }
    if let Some(n) = p.as_number() {
        if n == 0.0 {
            return Some(BgTok::Len(0.0));
        }
    }
    p.as_string()
        .map(|s| BgTok::Kw(s.cow_to_ascii_lowercase().into_owned()))
}

/// Split a `background-*` longhand into comma-separated groups (one per `<bg-layer>`).
/// A scalar property (e.g. `background-repeat: repeat`) is a single group.
fn bg_token_groups<S: CssSystem>(p: &S::Property) -> Vec<Vec<BgTok>> {
    if let Some(list) = p.as_list() {
        let mut groups: Vec<Vec<BgTok>> = vec![Vec::new()];
        for v in list {
            if v.is_comma() {
                groups.push(Vec::new());
            } else if let Some(t) = value_bg_tok::<S>(v) {
                // `groups` is seeded with one Vec and only grows, so `last_mut` is always Some;
                // handle it without `expect` (which the workspace lints deny).
                if let Some(last) = groups.last_mut() {
                    last.push(t);
                }
            }
        }
        return groups;
    }
    match prop_bg_tok::<S>(p) {
        Some(t) => vec![vec![t]],
        None => Vec::new(),
    }
}

/// `background-size` group -> tile size in px, or `None` for `auto`/`cover`/`contain`/`%`
/// (which mean "fill the box", i.e. no tiling).
fn resolve_bg_size(group: &[BgTok]) -> Option<(f32, f32)> {
    let mut dims = Vec::new();
    for t in group {
        match t {
            BgTok::Len(v) => dims.push(*v),
            // Percentage- and keyword-sized backgrounds need the box size to resolve; treat
            // them as "fill the box" for now (no tiling).
            BgTok::Pct(_) | BgTok::Kw(_) => return None,
        }
    }
    match dims.as_slice() {
        [w] => Some((*w, *w)),
        [w, h, ..] => Some((*w, *h)),
        _ => None,
    }
}

/// One axis of `background-position`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BgAnchor {
    /// A length in px from the box's start edge (left / top).
    Start(f32),
    /// A length in px from the box's end edge - `right`, `bottom`, and the three-value forms
    /// `right 10px` / `bottom 1em`.
    End(f32),
    /// A percentage: that point of the image is aligned with the same point of the box, so `50%`
    /// centres it and `100%` puts its far edge on the box's far edge.
    Percent(f32),
}

impl BgAnchor {
    /// Where the tile's start edge lands, given the box's and the tile's extent on this axis.
    #[must_use]
    pub fn resolve(self, box_extent: f32, tile_extent: f32) -> f32 {
        match self {
            BgAnchor::Start(v) => v,
            BgAnchor::End(v) => box_extent - tile_extent - v,
            BgAnchor::Percent(p) => (box_extent - tile_extent) * p / 100.0,
        }
    }
}

/// `background-position` group → one [`BgAnchor`] per axis.
///
/// The two-keyword form may be written in either order - `center right` means the same as
/// `right center` - so the axis a keyword belongs to is decided by the keyword, not by where it
/// sits in the list. Reading the first value as the horizontal one put Wikipedia's external-link
/// icon, which is positioned `center right`, in the middle of every link instead of after it.
///
/// Also handles the three-value edge-offset form (`right 10px`) and the one-value form, whose
/// missing axis is `center` rather than the start edge.
fn resolve_bg_position(group: &[BgTok]) -> (BgAnchor, BgAnchor) {
    let mut x: Option<BgAnchor> = None;
    let mut y: Option<BgAnchor> = None;
    // How many position components were written, so the one-value form can default its other axis
    // to `center`. A length that an edge keyword swallows (`right 10px`) is part of that keyword's
    // component, not one of its own.
    let mut components = 0usize;
    // Whether the last keyword was an edge one, and which axis/edge it named, so a length after it
    // becomes an offset from that edge.
    let mut pending_edge: Option<(bool, bool)> = None;
    // A leading `center` claims a component without naming an axis, and the two-value form then
    // means it took the horizontal one: `center 4px` is x = center, y = 4px. Without this the
    // length filled the still-empty horizontal slot and the tile moved along the wrong axis.
    let mut center_took_x = false;

    for tok in group {
        match tok {
            BgTok::Kw(k) => {
                let edge = match k.as_str() {
                    "left" => Some((false, false)),
                    "right" => Some((false, true)),
                    "top" => Some((true, false)),
                    "bottom" => Some((true, true)),
                    _ => None,
                };
                match (edge, k.as_str()) {
                    (Some((vertical, from_end)), _) => {
                        let anchor = if from_end {
                            BgAnchor::End(0.0)
                        } else {
                            BgAnchor::Start(0.0)
                        };
                        if vertical {
                            y = Some(anchor);
                        } else {
                            x = Some(anchor);
                        }
                        components += 1;
                        pending_edge = Some((vertical, from_end));
                    }
                    (None, "center") => {
                        // Which axis it means depends on what follows, so it waits - but a length
                        // after it is the *other* axis.
                        if x.is_none() && y.is_none() {
                            center_took_x = true;
                        }
                        components += 1;
                        pending_edge = None;
                    }
                    // Not a position keyword at all (a `background` shorthand carries
                    // `no-repeat`, `cover`, … in the same list).
                    (None, _) => pending_edge = None,
                }
            }
            BgTok::Len(v) => match pending_edge.take() {
                Some((true, from_end)) => {
                    y = Some(if from_end {
                        BgAnchor::End(*v)
                    } else {
                        BgAnchor::Start(*v)
                    })
                }
                Some((false, from_end)) => {
                    x = Some(if from_end {
                        BgAnchor::End(*v)
                    } else {
                        BgAnchor::Start(*v)
                    })
                }
                None => {
                    if x.is_none() && !center_took_x {
                        x = Some(BgAnchor::Start(*v));
                    } else if y.is_none() {
                        y = Some(BgAnchor::Start(*v));
                    }
                    components += 1;
                }
            },
            BgTok::Pct(p) => {
                pending_edge = None;
                if x.is_none() && !center_took_x {
                    x = Some(BgAnchor::Percent(*p));
                } else if y.is_none() {
                    y = Some(BgAnchor::Percent(*p));
                }
                components += 1;
            }
        }
    }

    // Whatever no component claimed is `center`: that is what a lone `center` means on the axis it
    // did not name, and what the one-value form means for its missing axis.
    let centered = BgAnchor::Percent(50.0);
    match (x, y) {
        (Some(x), Some(y)) => (x, y),
        (Some(x), None) => (x, centered),
        (None, Some(y)) => (centered, y),
        // No component at all is the initial value, `0% 0%`.
        (None, None) if components == 0 => (BgAnchor::Start(0.0), BgAnchor::Start(0.0)),
        (None, None) => (centered, centered),
    }
}

/// Whether a keyword names a place in `background-position`, as opposed to the repeat and size
/// keywords that share the `background` shorthand's token list.
fn is_position_keyword(k: &str) -> bool {
    matches!(k, "left" | "right" | "top" | "bottom" | "center")
}

/// Whether a token could be part of a `<bg-position>`, used to tell a `background-position`
/// declaration that says something from one that only carries junk.
fn is_position_token(t: &BgTok) -> bool {
    match t {
        BgTok::Kw(k) => is_position_keyword(k),
        BgTok::Len(_) | BgTok::Pct(_) => true,
    }
}

/// `background-repeat` group -> (repeat_x, repeat_y). Defaults to repeating both axes.
fn resolve_bg_repeat(group: &[BgTok]) -> (bool, bool) {
    let kws: Vec<&str> = group
        .iter()
        .filter_map(|t| match t {
            BgTok::Kw(k) => Some(k.as_str()),
            _ => None,
        })
        .collect();
    let axis = |k: &str| k != "no-repeat"; // repeat / space / round all tile
    match kws.as_slice() {
        [] => (true, true),
        ["repeat-x"] => (true, false),
        ["repeat-y"] => (false, true),
        [a] => (axis(a), axis(a)),
        [a, b, ..] => (axis(a), axis(b)),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PipelineNodeKind {
    Text,
    Comment,
    Element,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BgSize {
    /// `auto` / absent - the image's intrinsic size.
    Auto,
    /// Explicit lengths in px.
    Length(f32, f32),
    /// `cover` - scale (preserving aspect) so the image fully covers the box, cropping overflow.
    Cover,
    /// `contain` - scale (preserving aspect) so the image fits inside the box, letterboxing.
    Contain,
}

/// Resolved `background-repeat`/`-size`/`-position` for an element's first background layer. Read
/// from the `background` shorthand as well as the longhands, since pages write `background: url(x)
/// repeat`. `cover`/`contain` need the box size, so final tile geometry is computed at paint time.
#[derive(Debug, Clone, Copy)]
pub struct BgImageLayout {
    /// Whether the tile repeats on the x / y axis (`background-repeat`; default repeat both).
    pub repeat: (bool, bool),
    /// Where the tile is anchored on each axis (`background-position`), resolved against the box
    /// at paint time since edges and percentages need to know how big it is.
    pub position: (BgAnchor, BgAnchor),
    /// Resolved `background-size`.
    pub size: BgSize,
}

impl Default for BgImageLayout {
    fn default() -> Self {
        BgImageLayout {
            repeat: (true, true),
            position: (BgAnchor::Start(0.0), BgAnchor::Start(0.0)),
            size: BgSize::Auto,
        }
    }
}

/// The open `<select>` dropdown as the pipeline sees it (mirrors the engine's state).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OpenPopup {
    pub select: NodeId,
    pub hover: Option<usize>,
    pub active: Option<usize>,
    pub first_row: usize,
    pub viewport_top: f64,
    pub viewport_height: f64,
}

// ── PipelineDocument trait ────────────────────────────────────────────────────

pub trait PipelineDocument: Send + Sync {
    fn root(&self) -> Option<NodeId>;
    fn children(&self, id: NodeId) -> Vec<NodeId>;
    fn node_kind(&self, id: NodeId) -> PipelineNodeKind;
    fn tag_name(&self, id: NodeId) -> Option<String>;
    fn is_display_none(&self, id: NodeId) -> bool;
    fn parent(&self, id: NodeId) -> Option<NodeId>;
    fn html_node_id(&self) -> Option<NodeId>;
    fn body_node_id(&self) -> Option<NodeId>;
    fn base_url(&self) -> String;
    fn inner_html(&self, id: NodeId) -> String;
    fn get_node_by_id(&self, _id: NodeId) -> Option<Node> {
        None
    }

    /// The node's computed style: one typed field per property, every one of them holding a
    /// value - the element's own, the one it inherited, or the property's initial.
    ///
    /// [`ComputedStyle::has`] answers the separate question of whether the element's own
    /// cascade said anything about a property, which a handful of readers need: an element with
    /// no `display` of its own falls back to what its tag name means, and an undeclared
    /// `z-index` stacks differently from `z-index: auto`.
    fn computed_style(&self, id: NodeId) -> Arc<ComputedStyle>;

    /// `background-image` gradient layers in source order (first listed paints on top), each
    /// carrying its resolved tiling (`None` tiling = fill the box). Empty for solid/image
    /// backgrounds.
    ///
    /// `box_size` is the painting area the layers are positioned against; an edge or percentage
    /// `background-position` cannot be resolved without it.
    fn background_layers(&self, _id: NodeId, _box_size: (f32, f32)) -> Vec<Gradient> {
        Vec::new()
    }

    /// Tiling for a raster/SVG `background-image`, read from both the `background` shorthand and
    /// the longhands. Defaults to "repeat both axes, intrinsic size, no offset".
    fn background_image_layout(&self, _id: NodeId) -> BgImageLayout {
        BgImageLayout::default()
    }

    fn is_focused(&self, _id: NodeId) -> bool {
        false
    }

    /// Typed value, caret and selection of a text control; `None` = untouched.
    fn control_edit_state(&self, _id: NodeId) -> Option<gosub_interface::document::ControlEditState> {
        None
    }

    fn is_checked(&self, _id: NodeId) -> bool {
        false
    }

    /// An element's attribute, for the few paint decisions that hang on markup rather than
    /// style: which format a date input shows its value in.
    fn attribute(&self, _id: NodeId, _name: &str) -> Option<String> {
        None
    }

    fn selected_option(&self, _select: NodeId) -> Option<NodeId> {
        None
    }

    /// The open `<select>` dropdown, if any.
    fn open_select(&self) -> Option<OpenPopup> {
        None
    }

    /// Border-box size the user resized a control to.
    fn control_size(&self, _id: NodeId) -> Option<(f64, f64)> {
        None
    }

    /// The translation part of CSS `transform` (`translate`/`translateX`/`translateY`), each axis
    /// in px or a percentage of the element's own box. Other transform functions are ignored.
    fn transform_translate(&self, _id: NodeId) -> Option<(LengthPercentage, LengthPercentage)> {
        None
    }

    /// Forces the next `get_own_style` to re-evaluate CSS selectors (including `:hover`) from
    /// scratch. No-op for backends that do not cache styles.
    fn clear_style_cache(&self) {}

    /// Cheaper than `clear_style_cache` for hover repaints where only a few elements changed.
    fn invalidate_style_for_nodes(&self, _ids: &[NodeId]) {}
}

// ── Pseudo-element (::before / ::after) synthetic nodes ───────────────────────
//
// Generated content has no DOM node, but the pipeline is keyed by `NodeId` - so mint synthetic
// ids the adapter resolves on the fly, letting the rest of the pipeline treat them as normal nodes.
//
// Encoding: top bit flags a synthetic id, next two bits are the role, the rest hold the owner
// element id. Real DOM ids are small, so the high bits are free.
const PSEUDO_FLAG: u64 = 1 << 62;
const ROLE_BEFORE_ELEM: u64 = 0; // the ::before pseudo-element box
const ROLE_AFTER_ELEM: u64 = 1; // the ::after pseudo-element box
const ROLE_BEFORE_TEXT: u64 = 2; // generated text child of ::before
const ROLE_AFTER_TEXT: u64 = 3; // generated text child of ::after

/// The element that generated a synthetic `::before`/`::after` node id; `None` for real nodes.
pub fn pseudo_owner(id: NodeId) -> Option<NodeId> {
    is_pseudo_id(u64::from(id)).then(|| decode_pseudo(id).0)
}

const fn is_pseudo_id(id_val: u64) -> bool {
    id_val & PSEUDO_FLAG != 0
}

fn encode_pseudo(owner: NodeId, role: u64) -> NodeId {
    NodeId::from(PSEUDO_FLAG | (u64::from(owner) << 2) | role)
}

fn decode_pseudo(id: NodeId) -> (NodeId, u64) {
    let v = u64::from(id) & !PSEUDO_FLAG;
    (NodeId::from(v >> 2), v & 0b11)
}

const fn role_is_after(role: u64) -> bool {
    matches!(role, ROLE_AFTER_ELEM | ROLE_AFTER_TEXT)
}

// ── Anonymous table boxes (CSS 2.1 §17.2.1, "generate missing parents") ───────
//
// A run of consecutive table-internal children (display: table-cell / table-row /
// row groups / ...) whose parent provides no table context must be wrapped in an
// anonymous table box. The wrapper is minted like a pseudo-element id: bit 61 flags
// the id and the payload is the run's FIRST member. Downstream (render tree, taffy,
// lattice, painter) then sees a regular `display: table` element; lattice's own
// fixup generates the missing rows/row-groups inside it.
const ANON_TABLE_FLAG: u64 = 1 << 61;
/// Anonymous table-ROW wrapper around a run of children that are not proper table/row-group
/// children. Needed not just for CSS structure: the taffy FIRST pass approximates a row as a
/// flex row, so without the wrapper bare cells stack vertically and a fit-content ancestor
/// (e.g. an abs-positioned overlay div) collapses to one cell's width.
const ANON_ROW_FLAG: u64 = 1 << 60;
/// Anonymous table-CELL wrapper around a run of non-cell children inside a row.
const ANON_CELL_FLAG: u64 = 1 << 59;

const ANON_FLAGS: u64 = ANON_TABLE_FLAG | ANON_ROW_FLAG | ANON_CELL_FLAG;

const fn is_anon_table_id(id_val: u64) -> bool {
    id_val & ANON_FLAGS == ANON_TABLE_FLAG && id_val & PSEUDO_FLAG == 0
}

const fn is_anon_row_id(id_val: u64) -> bool {
    id_val & ANON_FLAGS == ANON_ROW_FLAG && id_val & PSEUDO_FLAG == 0
}

const fn is_anon_cell_id(id_val: u64) -> bool {
    id_val & ANON_FLAGS == ANON_CELL_FLAG && id_val & PSEUDO_FLAG == 0
}

/// Any flavour of synthetic anonymous table box.
const fn is_anon_box_id(id_val: u64) -> bool {
    is_anon_table_id(id_val) || is_anon_row_id(id_val) || is_anon_cell_id(id_val)
}

/// The display a synthetic anonymous box carries.
fn anon_box_display(id_val: u64) -> Option<Display> {
    if is_anon_table_id(id_val) {
        Some(Display::Table)
    } else if is_anon_row_id(id_val) {
        Some(Display::TableRow)
    } else if is_anon_cell_id(id_val) {
        Some(Display::TableCell)
    } else {
        None
    }
}

fn encode_anon_table(first_member: NodeId) -> NodeId {
    NodeId::from(ANON_TABLE_FLAG | u64::from(first_member))
}

fn encode_anon_row(first_member: NodeId) -> NodeId {
    NodeId::from(ANON_ROW_FLAG | u64::from(first_member))
}

fn encode_anon_cell(first_member: NodeId) -> NodeId {
    NodeId::from(ANON_CELL_FLAG | u64::from(first_member))
}

fn decode_anon_box(id: NodeId) -> NodeId {
    NodeId::from(u64::from(id) & !ANON_FLAGS)
}

/// Is `child` a proper child of a table box (CSS 2.1 §17.2)? Everything else inside a table
/// gets wrapped in an anonymous row.
fn proper_table_child(d: Option<&Display>) -> bool {
    matches!(
        d,
        Some(
            Display::TableRow
                | Display::TableRowGroup
                | Display::TableHeaderGroup
                | Display::TableFooterGroup
                | Display::TableCaption
                | Display::TableColumn
                | Display::TableColumnGroup
        )
    )
}

/// Does a child with display `child` require a table ancestor that a parent with
/// display `parent` does not provide?
fn needs_table_parent(child: &Display, parent: Option<&Display>) -> bool {
    use Display::*;
    match child {
        TableCell => !matches!(
            parent,
            Some(Table | TableRow | TableRowGroup | TableHeaderGroup | TableFooterGroup)
        ),
        TableRow => !matches!(
            parent,
            Some(Table | TableRowGroup | TableHeaderGroup | TableFooterGroup)
        ),
        TableRowGroup | TableHeaderGroup | TableFooterGroup | TableCaption | TableColumnGroup => {
            !matches!(parent, Some(Table))
        }
        TableColumn => !matches!(parent, Some(Table | TableColumnGroup)),
        _ => false,
    }
}

const fn role_is_text(role: u64) -> bool {
    matches!(role, ROLE_BEFORE_TEXT | ROLE_AFTER_TEXT)
}

/// A materialized pseudo-element: its computed style map plus the generated text (if the
/// resolved `content` produced any). `text == None` means an empty box (e.g. `content: ""`).
struct PseudoBox<P> {
    styles: Arc<P>,
    text: Option<String>,
}

fn unquote(s: &str) -> String {
    let b = s.as_bytes();
    if b.len() >= 2 && ((b[0] == b'"' && b[b.len() - 1] == b'"') || (b[0] == b'\'' && b[b.len() - 1] == b'\'')) {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

/// `None` only for `none`/`normal`, which suppress the box entirely.
fn content_token_to_string(s: &str) -> Option<String> {
    match s {
        "none" | "normal" => None,
        // We have no quote-pair stack, so use the typographic defaults.
        "open-quote" => Some("\u{201C}".to_string()),
        "close-quote" => Some("\u{201D}".to_string()),
        "no-open-quote" | "no-close-quote" => Some(String::new()),
        _ => Some(unquote(s)),
    }
}

/// Counter state (counter-reset/-increment scoping) is not tracked yet, so counters resolve to
/// empty text - generated boxes still appear, just without the number.
fn resolve_content_function<S: CssSystem>(name: &str, _args: &[S::Value]) -> String {
    if matches!(name, "counter" | "counters") {
        log::debug!("content: {name}() is not yet supported; rendering empty");
    }
    String::new()
}

fn content_value_to_string<S: CssSystem>(v: &S::Value) -> Option<String> {
    if let Some(s) = v.as_string() {
        return content_token_to_string(s);
    }
    if let Some((name, args)) = v.as_function() {
        return Some(resolve_content_function::<S>(name, args));
    }
    if let Some(list) = v.as_list() {
        let mut out = String::new();
        for item in list {
            if let Some(part) = content_value_to_string::<S>(item) {
                out.push_str(&part);
            }
        }
        return Some(out);
    }
    None
}

/// `None` => generate no box (`content: none | normal`); `Some("")` => an empty box.
fn resolve_content<S: CssSystem>(p: &S::Property) -> Option<String> {
    // A single string/keyword token.
    if let Some(s) = p.as_string() {
        return content_token_to_string(s);
    }
    // A list of tokens (strings, attr()/var() already resolved upstream, counters, quotes).
    if let Some(list) = p.as_list() {
        let mut out = String::new();
        for v in list {
            if let Some(part) = content_value_to_string::<S>(v) {
                out.push_str(&part);
            }
        }
        return Some(out);
    }
    // A bare function value.
    if let Some((name, args)) = p.as_function() {
        return Some(resolve_content_function::<S>(name, args));
    }
    None
}

// ── GosubDocumentAdapter ──────────────────────────────────────────────────────

/// A node's computed property map as held by the style cache.
type CachedStyles<C> = Arc<<<C as gosub_interface::config::HasCssSystem>::CssSystem as CssSystem>::PropertyMap>;

/// Adapts any `gosub_interface::document::Document<C>` into a `PipelineDocument`.
/// Which slottables ended up in which `<slot>`, for every shadow tree in the document.
///
/// Computed once, when the adapter is built. Without scripting neither the light DOM nor the
/// shadow trees change after parsing, so an assignment can never go stale - there is no
/// invalidation to run and no `slotchange` to fire.
#[derive(Default)]
struct SlotAssignment {
    /// The slottables projected into each slot, in tree order. A slot that is absent here, or
    /// present with an empty list, renders its own children as fallback content instead.
    assigned: HashMap<NodeId, Vec<NodeId>>,
    /// The slot each projected node landed in: the inverse of `assigned`, and the flat-tree
    /// parent that style inheritance follows.
    slot_of: HashMap<NodeId, NodeId>,
}

/// Assigns each shadow host's light children to the slots of its shadow tree.
fn compute_slot_assignment<C: HasDocument>(doc: &C::Document) -> SlotAssignment {
    let mut out = SlotAssignment::default();

    let mut stack = vec![doc.root()];
    while let Some(id) = stack.pop() {
        stack.extend(doc.children(id).iter().copied());

        let Some(shadow_root) = doc.shadow_root(id) else {
            continue;
        };
        // A shadow tree can contain hosts of its own, so it joins the walk. It is not reached
        // through `children`, which is exactly what keeps shadow trees out of everything that
        // has not opted in.
        stack.push(shadow_root);
        assign_to_slots::<C>(doc, id, shadow_root, &mut out);
    }

    out
}

/// The slot-assignment algorithm for one host: find the shadow tree's slots, then hand each of
/// the host's light children to the slot that claims it.
fn assign_to_slots<C: HasDocument>(doc: &C::Document, host: NodeId, shadow_root: NodeId, out: &mut SlotAssignment) {
    // Collect slots in tree order, first of a given name winning. The walk deliberately runs
    // over the whole shadow tree: a `<slot>` sitting in a *nested* host's light DOM is still a
    // descendant of this tree, and so is still one of this tree's slots.
    let mut default_slot: Option<NodeId> = None;
    let mut named_slots: HashMap<&str, NodeId> = HashMap::new();

    let mut stack: Vec<NodeId> = doc.children(shadow_root).iter().rev().copied().collect();
    while let Some(node) = stack.pop() {
        stack.extend(doc.children(node).iter().rev().copied());

        if doc.tag_name(node) != Some("slot") {
            continue;
        }
        match doc.attribute(node, "name").unwrap_or("") {
            "" => {
                default_slot.get_or_insert(node);
            }
            name => {
                named_slots.entry(name).or_insert(node);
            }
        }
    }

    for &child in doc.children(host) {
        let slot = match doc.node_type(child) {
            // Only elements and text are slottables. An element goes to the slot named by its
            // `slot` attribute; text has no such attribute and always goes to the default
            // slot - whitespace-only runs included, which is why an unslotted-looking gap can
            // still push content around.
            GosubNodeType::ElementNode => match doc.attribute(child, "slot").unwrap_or("") {
                "" => default_slot,
                name => named_slots.get(name).copied(),
            },
            GosubNodeType::TextNode => default_slot,
            _ => None,
        };

        // No slot claimed it: the node stays in the light DOM and renders nowhere.
        let Some(slot) = slot else {
            continue;
        };
        out.assigned.entry(slot).or_default().push(child);
        out.slot_of.insert(child, slot);
    }
}

pub struct GosubDocumentAdapter<C>
where
    C: HasDocument,
    <C::CssSystem as CssSystem>::PropertyMap: Send + Sync,
{
    pub doc: Arc<C::Document>,
    /// Per-node computed-style cache (from CSS selector matching). Populated lazily.
    style_cache: Mutex<HashMap<NodeId, CachedStyles<C>>>,
    /// The typed style each node resolved to, built from `style_cache` and the parent's struct.
    /// This is what the pipeline reads; the map behind it stays for the cascade's own questions
    /// (custom-property scope, which origin won a declaration) and for `getComputedStyle`.
    computed_cache: Mutex<HashMap<NodeId, Arc<ComputedStyle>>>,
    /// Materialized `::before` / `::after` pseudo-boxes, keyed by `(owner, is_after)`.
    /// `None` means "no generated box". Populated lazily.
    #[allow(clippy::type_complexity)]
    pseudo_cache: Mutex<HashMap<(NodeId, bool), Option<Arc<PseudoBox<<C::CssSystem as CssSystem>::PropertyMap>>>>>,
    /// `parent()` resolves anonymous-wrapper parents by scanning the real parent's child
    /// list; `get_style` calls it per inherited property, which made style resolution
    /// quadratic in table size. Keyed on the (possibly synthetic) id.
    parent_cache: Mutex<HashMap<NodeId, Option<NodeId>>>,
    /// Flat-tree slot assignment, computed up front and then never touched again.
    slots: SlotAssignment,
}

impl<C> GosubDocumentAdapter<C>
where
    C: HasDocument + Send + Sync + 'static,
    C::Document: Send + Sync,
    <C::CssSystem as CssSystem>::PropertyMap: Send + Sync,
{
    fn parent_uncached(&self, id: NodeId) -> Option<NodeId> {
        // Synthetic anonymous boxes: parent is the members' real parent when it provides the
        // right context, else the next synthetic wrapper up - located by finding the enclosing
        // run's start among the real parent's children.
        if is_anon_cell_id(u64::from(id)) {
            let first = decode_anon_box(id);
            let real_parent = self.doc.parent(first)?;
            if matches!(self.display_of(real_parent), Some(Display::TableRow)) {
                return Some(real_parent);
            }
            // The enclosing anonymous row wraps the row-level run containing this cell run.
            let rmembers = self.row_run_members_for(first);
            return Some(encode_anon_row(rmembers[0]));
        }
        if is_anon_row_id(u64::from(id)) {
            let first = decode_anon_box(id);
            let real_parent = self.doc.parent(first)?;
            let parent_display = self.display_of(real_parent);
            if matches!(
                parent_display,
                Some(Display::Table | Display::TableRowGroup | Display::TableHeaderGroup | Display::TableFooterGroup)
            ) {
                return Some(real_parent);
            }
            // The enclosing anonymous table wraps the table-level run containing this row run.
            let start = self
                .run_start_containing(real_parent, first, |c| {
                    self.display_of(c)
                        .is_some_and(|d| needs_table_parent(&d, parent_display.as_ref()))
                })
                .unwrap_or(first);
            return Some(encode_anon_table(start));
        }
        if is_anon_table_id(u64::from(id)) {
            return self.doc.parent(decode_anon_box(id));
        }
        // Members of a synthesized run report the wrapper as their parent, so the parent
        // chain matches the child lists children() hands out.
        if let Some(wrapper) = self.synthetic_parent_of(id) {
            return Some(wrapper);
        }
        if is_pseudo_id(u64::from(id)) {
            let (owner, role) = decode_pseudo(id);
            // Text child's parent is its pseudo-element; the pseudo-element's parent is the owner.
            return Some(if role_is_text(role) {
                encode_pseudo(
                    owner,
                    if role_is_after(role) {
                        ROLE_AFTER_ELEM
                    } else {
                        ROLE_BEFORE_ELEM
                    },
                )
            } else {
                owner
            });
        }
        self.flat_parent(id)
    }

    /// The flat-tree parent of a real node: the slot it was projected into, or the host
    /// standing in for a shadow root, or plainly its DOM parent.
    ///
    /// This is what inherited properties resolve through (`get_style` walks `parent`, not
    /// `children`), and `compute_styles` asks for it directly rather than via `parent`: the
    /// anonymous-table lookup in `parent_uncached` needs the node's own `display`, which needs
    /// its styles, which need its parent's - a cycle. An anonymous wrapper carries no styles of
    /// its own, so inheriting straight from the real parent gives the same answer.
    fn flat_parent(&self, id: NodeId) -> Option<NodeId> {
        if let Some(&slot) = self.slots.slot_of.get(&id) {
            // A projected node inherits from the slot it landed in - so it picks up the shadow
            // tree's chain, not the light DOM's, even though the DOM parent is still the host.
            return Some(slot);
        }
        let parent = self.doc.parent(id)?;
        if self.doc.node_type(parent) == GosubNodeType::ShadowRootNode {
            // The shadow root generates no box and has no styles of its own; the host stands
            // in for it, which is also where inheritance into a shadow tree comes from.
            return self.doc.shadow_host(parent);
        }
        Some(parent)
    }

    pub fn new(doc: Arc<C::Document>) -> Self {
        let slots = compute_slot_assignment::<C>(&doc);
        Self {
            doc,
            style_cache: Mutex::new(HashMap::new()),
            computed_cache: Mutex::new(HashMap::new()),
            pseudo_cache: Mutex::new(HashMap::new()),
            parent_cache: Mutex::new(HashMap::new()),
            slots,
        }
    }

    /// Whether `id` is a `<slot>`. There is no `slot` element in any other namespace, so the
    /// tag name settles it - as it does everywhere else in this adapter.
    fn is_slot(&self, id: NodeId) -> bool {
        self.doc.tag_name(id) == Some("slot")
    }

    /// The children of `id` in the flat tree - what actually generates boxes beneath it.
    ///
    /// Three rewrites, each of them purely local:
    ///
    ///  - a **shadow host** renders its shadow tree, so it yields the shadow root's children.
    ///    The shadow root itself is spliced out: it generates no box and carries no styles.
    ///  - a **`<slot>`** is replaced by the nodes projected into it, or by its own children as
    ///    fallback when nothing was. Like `display: contents`, which this engine has no general
    ///    support for, the slot generates no box - but it stays the *style* parent of what it
    ///    projects, which [`parent`](Self::parent) is what makes true.
    ///  - a **light child no slot claimed** is dropped, which is what makes unassigned content
    ///    invisible rather than merely unstyled.
    fn flat_children(&self, id: NodeId) -> Vec<NodeId> {
        // A slot the author gave a box of its own is an ordinary parent: its children are the
        // nodes projected into it.
        if self.is_slot(id) && self.slot_generates_a_box(id) {
            let mut out = Vec::new();
            self.push_slot_content(id, &mut out);
            return out;
        }

        let source = self.doc.shadow_root(id).unwrap_or(id);

        let children = self.doc.children(source);
        // The overwhelmingly common case: no slot among them, so nothing to rewrite.
        if !children.iter().any(|&child| self.is_slot(child)) {
            return children.to_vec();
        }

        let mut out = Vec::with_capacity(children.len());
        for &child in children {
            self.push_flattened(child, &mut out);
        }
        out
    }

    /// Whether a `<slot>` keeps a box of its own instead of being spliced away.
    ///
    /// The user-agent sheet gives every slot `display: contents`, which this engine has no
    /// `Display` variant for - it falls through to `Block` - so the *computed* value cannot
    /// tell the UA default apart from an authored `display: block`. The raw declared keyword
    /// can, so read that: only `contents` (or nothing at all) makes the slot transparent.
    fn slot_generates_a_box(&self, slot: NodeId) -> bool {
        let arc = self.cached_styles(slot);

        match <_ as CssPropertyMap<C::CssSystem>>::get(arc.as_ref(), "display").and_then(|p| p.as_string()) {
            Some("contents") | None => false,
            Some(_) => true,
        }
    }

    /// Appends `node` to `out`, or - when it is a slot that generates no box - whatever stands
    /// in its place.
    ///
    /// The expansion recurses because what a slot projects can be another slot: a `<slot>` in
    /// the light DOM of a nested host is a slottable of the inner tree *and* a slot of the
    /// outer one, so content flows through both. It always terminates - projection steps move
    /// strictly outwards through the host nesting, fallback steps strictly down the tree.
    fn push_flattened(&self, node: NodeId, out: &mut Vec<NodeId>) {
        if !self.is_slot(node) || self.slot_generates_a_box(node) {
            out.push(node);
            return;
        }
        self.push_slot_content(node, out);
    }

    /// Appends what a slot projects: the nodes assigned to it, or - when nothing was assigned -
    /// its own children, which are the slot's fallback content.
    fn push_slot_content(&self, slot: NodeId, out: &mut Vec<NodeId>) {
        match self.slots.assigned.get(&slot) {
            Some(assigned) if !assigned.is_empty() => {
                for &n in assigned {
                    self.push_flattened(n, out);
                }
            }
            _ => {
                for &n in self.doc.children(slot) {
                    self.push_flattened(n, out);
                }
            }
        }
    }

    /// `None` if no rule generates one. Computed and cached on first access.
    fn pseudo_box(
        &self,
        owner: NodeId,
        is_after: bool,
    ) -> Option<Arc<PseudoBox<<C::CssSystem as CssSystem>::PropertyMap>>> {
        if let Some(cached) = self.pseudo_cache.lock().get(&(owner, is_after)) {
            return cached.clone();
        }

        let result = self.compute_pseudo_box(owner, is_after);
        self.pseudo_cache.lock().insert((owner, is_after), result.clone());
        result
    }

    fn compute_pseudo_box(
        &self,
        owner: NodeId,
        is_after: bool,
    ) -> Option<Arc<PseudoBox<<C::CssSystem as CssSystem>::PropertyMap>>> {
        // Pseudo-elements only hang off real elements.
        if self.doc.node_type(owner) != GosubNodeType::ElementNode {
            return None;
        }
        let name = if is_after { "after" } else { "before" };
        let sheets = self.doc.stylesheets();
        let owner_styles = self.cached_styles(owner);
        let mut prop_map =
            C::CssSystem::pseudo_properties_from_node::<C>(&*self.doc, owner, sheets, name, Some(&owner_styles))?;
        for (_, prop) in prop_map.iter_mut() {
            prop.compute_value();
        }

        // Resolve `content` into generated text. `none`/`normal` means no box at all.
        let content_prop = <_ as CssPropertyMap<C::CssSystem>>::get(&prop_map, "content")?;
        let text = resolve_content::<C::CssSystem>(content_prop)?;

        // `content: ""` (and any all-empty result) generates a box but no text child.
        let text = if text.is_empty() { None } else { Some(text) };

        Some(Arc::new(PseudoBox {
            styles: Arc::new(prop_map),
            text,
        }))
    }

    /// Drop every cached style below `id` (not `id` itself), pseudo-boxes included.
    fn invalidate_subtree(&self, id: NodeId) {
        let mut cache = self.style_cache.lock();
        let mut computed_cache = self.computed_cache.lock();
        let mut pseudo_cache = self.pseudo_cache.lock();
        let mut stack: Vec<NodeId> = self.doc.children(id).to_vec();
        while let Some(node) = stack.pop() {
            cache.remove(&node);
            computed_cache.remove(&node);
            computed_cache.remove(&encode_pseudo(node, ROLE_BEFORE_ELEM));
            computed_cache.remove(&encode_pseudo(node, ROLE_AFTER_ELEM));
            computed_cache.remove(&encode_pseudo(node, ROLE_BEFORE_TEXT));
            computed_cache.remove(&encode_pseudo(node, ROLE_AFTER_TEXT));
            pseudo_cache.remove(&(node, false));
            pseudo_cache.remove(&(node, true));
            stack.extend_from_slice(self.doc.children(node));
        }
    }

    fn cached_styles(&self, id: NodeId) -> Arc<<C::CssSystem as CssSystem>::PropertyMap> {
        {
            if let Some(arc) = self.style_cache.lock().get(&id) {
                return arc.clone();
            }
        }
        let arc = Arc::new(self.compute_styles(id));
        self.style_cache.lock().insert(id, arc.clone());
        arc
    }

    /// The typed style of `id`, computed on first use and kept.
    fn cached_computed_style(&self, id: NodeId) -> Arc<ComputedStyle> {
        {
            if let Some(style) = self.computed_cache.lock().get(&id) {
                return style.clone();
            }
        }
        let style = Arc::new(self.build_computed_style(id));
        self.computed_cache.lock().insert(id, style.clone());
        style
    }

    /// The typed style of the node the inherited values come from: the flat-tree parent when it
    /// is an element, and nothing above the root.
    fn inherited_from(&self, id: NodeId) -> Option<Arc<ComputedStyle>> {
        self.flat_parent(id)
            .filter(|&parent| self.doc.node_type(parent) == GosubNodeType::ElementNode)
            .map(|parent| self.cached_computed_style(parent))
    }

    fn build_computed_style(&self, id: NodeId) -> ComputedStyle {
        let raw = u64::from(id);

        // An anonymous table box IS its display and has nothing else of its own; everything
        // that inherits comes from the real parent it was generated inside.
        if let Some(display) = anon_box_display(raw) {
            let parent = self
                .doc
                .parent(decode_anon_box(id))
                .map(|parent| self.cached_computed_style(parent));
            let mut style = ComputedStyle::inherit_from(parent.as_deref());
            style.box_group.display = display;
            style.declared.set(Prop::Display);
            return style;
        }

        if is_pseudo_id(raw) {
            let (owner, role) = decode_pseudo(id);
            if role_is_text(role) {
                // Generated text has no style of its own; it inherits from the pseudo-element
                // that generated it, exactly as a real text node does from its element.
                let element = encode_pseudo(
                    owner,
                    if role_is_after(role) {
                        ROLE_AFTER_ELEM
                    } else {
                        ROLE_BEFORE_ELEM
                    },
                );
                let parent = self.cached_computed_style(element);
                return ComputedStyle::inherit_from(Some(&parent));
            }
            let owner_style = self.cached_computed_style(owner);
            return match self.pseudo_box(owner, role_is_after(role)) {
                Some(pseudo) => pseudo.styles.computed_style(Some(&owner_style)),
                None => ComputedStyle::inherit_from(Some(&owner_style)),
            };
        }

        let parent = self.inherited_from(id);
        let map = self.cached_styles(id);
        let mut style = map.computed_style(parent.as_deref());

        // The presentational attributes, at the two precedences they have today. Both are
        // pending step 3b, where they join the cascade as the origin the HTML spec gives them.
        if self.doc.node_type(id) == GosubNodeType::ElementNode {
            presentation_hints::apply_table_hints(&mut style, self.table_hints(id, map.as_ref()));
            if let Some(attrs) = self.doc.attributes(id) {
                presentation_hints::apply_presentation_attrs(&mut style, attrs);
            }
        }
        style
    }

    /// The `cellspacing`/`cellpadding` this element picks up, with the author declarations that
    /// outrank them already taken out.
    fn table_hints(
        &self,
        id: NodeId,
        map: &<C::CssSystem as CssSystem>::PropertyMap,
    ) -> presentation_hints::TableHints {
        let author_declared = |name: &str| {
            <_ as CssPropertyMap<C::CssSystem>>::get(map, name)
                .and_then(|property| property.winning_origin())
                .is_some_and(|origin| matches!(origin, CssOrigin::Author))
        };
        let attr_px = |node: NodeId, attr: &str| -> Option<f32> {
            presentation_hints::attr_px(self.doc.attributes(node)?.get(attr)?)
        };
        let tag_is = |node: NodeId, tag: &str| self.doc.tag_name(node).is_some_and(|t| t.eq_ignore_ascii_case(tag));

        let mut hints = presentation_hints::TableHints::default();

        if tag_is(id, "table") && !author_declared("border-spacing") {
            hints.border_spacing = attr_px(id, "cellspacing");
        }

        if tag_is(id, "td") || tag_is(id, "th") {
            // The hint applies to in-table cells only; a parentless cell keeps the UA default.
            let mut table = None;
            let mut current = self.doc.parent(id);
            while let Some(node) = current {
                if tag_is(node, "table") {
                    table = Some(node);
                    break;
                }
                current = self.doc.parent(node);
            }
            if let Some(table) = table {
                let padding = attr_px(table, "cellpadding").unwrap_or(presentation_hints::DEFAULT_CELL_PADDING);
                let sides = ["padding-top", "padding-right", "padding-bottom", "padding-left"];
                for (slot, side) in hints.cell_padding.iter_mut().zip(sides) {
                    if !author_declared(side) && !author_declared("padding") {
                        *slot = Some(padding);
                    }
                }
            }
        }

        hints
    }

    /// The cascaded property map of `id`.
    ///
    /// The `style` attribute is not read here: the cascade already ranks it at inline
    /// specificity, through the real CSS parser, so a second hand-written parser on top of it
    /// could only disagree with the first.
    fn compute_styles(&self, id: NodeId) -> <C::CssSystem as CssSystem>::PropertyMap {
        // CSS selectors cannot target text nodes - only elements.
        if self.doc.node_type(id) == GosubNodeType::TextNode {
            return Default::default();
        }
        let sheets = self.doc.stylesheets();
        // Styles resolve top-down: the parent's map carries the inherited custom properties.
        // The *flat*-tree parent, so a slotted node picks them up from the slot it was
        // projected into rather than from its light-DOM host. Not `parent`: that one also
        // resolves anonymous table wrappers, which needs this node's styles first.
        let parent_styles = self
            .flat_parent(id)
            .filter(|&p| self.doc.node_type(p) == GosubNodeType::ElementNode)
            .map(|p| self.cached_styles(p));
        let mut prop_map = C::CssSystem::properties_from_node::<C>(&*self.doc, id, sheets, parent_styles.as_deref())
            .unwrap_or_default();
        for (_, prop) in prop_map.iter_mut() {
            prop.compute_value();
        }
        prop_map
    }

    // ── Anonymous table synthesis ─────────────────────────────────────────────

    /// The node's computed `display`, if the cascade assigned one.
    fn display_of(&self, id: NodeId) -> Option<Display> {
        let style = self.cached_computed_style(id);
        if !style.has(Prop::Display) {
            return None;
        }
        // Inline-table differs from table only in OUTER display (how it participates in
        // its parent's formatting context); every display_of consumer asks about table
        // structure, so normalize here and keep the inline-ness at the Node level.
        Some(match style.box_group.display {
            Display::InlineTable => Display::Table,
            display => display,
        })
    }

    /// Children skipped silently when collecting anonymous runs: whitespace-only text,
    /// comments/doctypes, and `display: none` children (none of them generate a box).
    fn run_skippable(&self, id: NodeId) -> bool {
        let raw = u64::from(id);
        if is_pseudo_id(raw) || is_anon_box_id(raw) {
            return false;
        }
        match self.doc.node_type(id) {
            GosubNodeType::TextNode => {
                if self.doc.text_value(id).is_some_and(|t| !t.trim().is_empty()) {
                    return false;
                }
                // Whitespace-only text: skippable only when collapsing would remove it.
                // Under `white-space: pre`/`pre-wrap` the spaces are content and generate
                // anonymous boxes (CSS 2.1 §17.2.1 considers only whitespace "that would be
                // collapsed"). Resolved over the RAW DOM parent chain: `get_style` routes
                // through `parent()`, whose synthetic-wrapper resolution calls back into
                // `run_skippable` - a cycle.
                let mut cur = self.doc.parent(id);
                while let Some(p) = cur {
                    let style = self.cached_computed_style(p);
                    if style.has(Prop::WhiteSpace) {
                        return !style.inherited.white_space.preserves_spaces();
                    }
                    cur = self.doc.parent(p);
                }
                true
            }
            GosubNodeType::CommentNode | GosubNodeType::DocTypeNode => true,
            _ => matches!(self.display_of(id), Some(Display::None)),
        }
    }

    /// Is `id` an improper child of a row container with display `parent_display`
    /// (a table or row group), i.e. must it be wrapped in an anonymous row?
    fn needy_for_row(&self, id: NodeId, parent_display: &Display) -> bool {
        // Pseudo ids carry PSEUDO_FLAG; OR-ing an anon flag onto one would decode as a
        // pseudo of a nonexistent owner, so they never become run members.
        if is_pseudo_id(u64::from(id)) || self.run_skippable(id) {
            return false;
        }
        let d = self.display_of(id);
        match parent_display {
            Display::Table => !proper_table_child(d.as_ref()),
            // Row groups: only rows are proper.
            _ => !matches!(d, Some(Display::TableRow)),
        }
    }

    /// Is `id` an improper (non-cell) child of a row, i.e. must it be wrapped in an
    /// anonymous cell?
    fn needy_for_cell(&self, id: NodeId) -> bool {
        !is_pseudo_id(u64::from(id))
            && !self.run_skippable(id)
            && !matches!(self.display_of(id), Some(Display::TableCell))
    }

    /// Generic run-collapser: replace each run of consecutive `needy` children with one
    /// synthetic id (`encode` of the first member). Skippable children (whitespace text,
    /// comments, display:none) BETWEEN run members are absorbed into the run and dropped.
    fn collapse_runs(
        &self,
        kids: Vec<NodeId>,
        needy: impl Fn(NodeId) -> bool,
        encode: fn(NodeId) -> NodeId,
    ) -> Vec<NodeId> {
        if !kids.iter().any(|&k| needy(k)) {
            return kids;
        }
        let mut out = Vec::with_capacity(kids.len());
        let mut i = 0;
        while i < kids.len() {
            if !needy(kids[i]) {
                out.push(kids[i]);
                i += 1;
                continue;
            }
            out.push(encode(kids[i]));
            i += 1;
            loop {
                let mut j = i;
                while j < kids.len() && self.run_skippable(kids[j]) {
                    j += 1;
                }
                if j < kids.len() && needy(kids[j]) {
                    i = j + 1;
                } else {
                    break;
                }
            }
        }
        out
    }

    /// The real members of a synthetic run: the needy siblings starting at `first`
    /// (interior skippable children are dropped).
    fn run_members(&self, first: NodeId, needy: impl Fn(NodeId) -> bool) -> Vec<NodeId> {
        let mut members = vec![first];
        let Some(parent) = self.doc.parent(first) else {
            return members;
        };
        let kids = self.doc.children(parent);
        let Some(pos) = kids.iter().position(|&k| k == first) else {
            return members;
        };
        let mut i = pos + 1;
        loop {
            let mut j = i;
            while j < kids.len() && self.run_skippable(kids[j]) {
                j += 1;
            }
            if j < kids.len() && needy(kids[j]) {
                members.push(kids[j]);
                i = j + 1;
            } else {
                break;
            }
        }
        members
    }

    /// First member of the run (per `needy`) among `parent`'s children that contains
    /// `member`, mirroring `collapse_runs`' grouping.
    fn run_start_containing(&self, parent: NodeId, member: NodeId, needy: impl Fn(NodeId) -> bool) -> Option<NodeId> {
        let kids = self.doc.children(parent);
        let mut i = 0;
        while i < kids.len() {
            if !needy(kids[i]) {
                i += 1;
                continue;
            }
            let start = kids[i];
            let mut hit = kids[i] == member;
            i += 1;
            loop {
                let mut j = i;
                while j < kids.len() && self.run_skippable(kids[j]) {
                    j += 1;
                }
                if j < kids.len() && needy(kids[j]) {
                    hit |= kids[j] == member;
                    i = j + 1;
                } else {
                    break;
                }
            }
            if hit {
                return Some(start);
            }
        }
        None
    }

    /// Collapse each run of table-internal children lacking a table parent into one
    /// anonymous-table id (CSS 2.1 §17.2.1 "generate missing parents").
    fn wrap_anon_table_runs(&self, parent_display: Option<&Display>, kids: Vec<NodeId>) -> Vec<NodeId> {
        // A parent that itself provides table context never wraps a table: the anonymous
        // row/cell wrappers below own the interior of a table.
        if matches!(
            parent_display,
            Some(
                Display::Table
                    | Display::TableRow
                    | Display::TableRowGroup
                    | Display::TableHeaderGroup
                    | Display::TableFooterGroup
                    | Display::TableColumnGroup
            )
        ) {
            return kids;
        }
        self.collapse_runs(
            kids,
            |id| {
                !is_pseudo_id(u64::from(id))
                    && self
                        .display_of(id)
                        .is_some_and(|d| needs_table_parent(&d, parent_display))
            },
            encode_anon_table,
        )
    }

    /// Collapse each run of improper children of a table / row group into one
    /// anonymous-row id.
    fn wrap_anon_row_runs(&self, parent_display: Option<&Display>, kids: Vec<NodeId>) -> Vec<NodeId> {
        let Some(pd) = parent_display else { return kids };
        if !matches!(
            pd,
            Display::Table | Display::TableRowGroup | Display::TableHeaderGroup | Display::TableFooterGroup
        ) {
            return kids;
        }
        self.collapse_runs(kids, |id| self.needy_for_row(id, pd), encode_anon_row)
    }

    /// Collapse each run of non-cell children of a (real or anonymous) row into one
    /// anonymous-cell id.
    fn wrap_anon_cell_runs(&self, parent_display: Option<&Display>, kids: Vec<NodeId>) -> Vec<NodeId> {
        if !matches!(parent_display, Some(Display::TableRow)) {
            return kids;
        }
        self.collapse_runs(kids, |id| self.needy_for_cell(id), encode_anon_cell)
    }

    /// Start of the maximal sub-run of consecutive `pred` members containing `id`.
    fn sub_run_start(&self, members: &[NodeId], id: NodeId, pred: impl Fn(NodeId) -> bool) -> NodeId {
        let Some(mut i) = members.iter().position(|&m| m == id) else {
            return id;
        };
        while i > 0 && pred(members[i - 1]) {
            i -= 1;
        }
        members[i]
    }

    /// The synthetic wrapper `children()` places `id` under, if any. Run members' parent
    /// chain must route through the anonymous boxes, or sibling walks (whitespace
    /// collapsing, vertical-align resolution) diverge from the tree children() produces.
    fn synthetic_parent_of(&self, id: NodeId) -> Option<NodeId> {
        let raw = u64::from(id);
        if is_pseudo_id(raw) || is_anon_box_id(raw) {
            return None;
        }
        let parent = self.doc.parent(id)?;
        let parent_display = self.display_of(parent);
        let d = self.display_of(id);

        // Inside a real row: non-cell children live in an anonymous cell.
        if matches!(parent_display, Some(Display::TableRow)) {
            if matches!(d, Some(Display::TableCell)) || self.run_skippable(id) {
                return None;
            }
            let start = self.run_start_containing(parent, id, |c| self.needy_for_cell(c))?;
            return Some(encode_anon_cell(start));
        }

        // Inside a real table / row group: improper children live in an anonymous row,
        // and non-cells among them one level deeper in an anonymous cell.
        if matches!(
            parent_display,
            Some(Display::Table | Display::TableRowGroup | Display::TableHeaderGroup | Display::TableFooterGroup)
        ) {
            let pd = parent_display.unwrap_or(Display::Table);
            if !self.needy_for_row(id, &pd) {
                return None;
            }
            let rstart = self.run_start_containing(parent, id, |c| self.needy_for_row(c, &pd))?;
            if matches!(d, Some(Display::TableCell)) {
                return Some(encode_anon_row(rstart));
            }
            let rmembers = self.anon_box_members(encode_anon_row(rstart));
            let cstart = self.sub_run_start(&rmembers, id, |c| self.needy_for_cell(c));
            return Some(encode_anon_cell(cstart));
        }

        // No table context at all: table-internal children live inside an anonymous table.
        let table_needy = |c: NodeId| {
            self.display_of(c)
                .is_some_and(|dd| needs_table_parent(&dd, parent_display.as_ref()))
        };
        if !table_needy(id) {
            return None;
        }
        let tstart = self.run_start_containing(parent, id, table_needy)?;
        if proper_table_child(d.as_ref()) {
            return Some(encode_anon_table(tstart));
        }
        let tmembers = self.anon_box_members(encode_anon_table(tstart));
        let rstart = self.sub_run_start(&tmembers, id, |c| self.needy_for_row(c, &Display::Table));
        if matches!(d, Some(Display::TableCell)) {
            return Some(encode_anon_row(rstart));
        }
        let rmembers = self.anon_box_members(encode_anon_row(rstart));
        let cstart = self.sub_run_start(&rmembers, id, |c| self.needy_for_cell(c));
        Some(encode_anon_cell(cstart))
    }

    /// The prefix of `members` starting at `first` for which `pred` holds contiguously.
    fn members_sub_run(&self, members: &[NodeId], first: NodeId, pred: impl Fn(NodeId) -> bool) -> Vec<NodeId> {
        let Some(i) = members.iter().position(|&m| m == first) else {
            return vec![first];
        };
        let mut out = vec![first];
        for &m in &members[i + 1..] {
            if pred(m) {
                out.push(m);
            } else {
                break;
            }
        }
        out
    }

    /// Members of the row-level run containing `id`. In a real table/row-group the run is
    /// collected over the raw siblings; inside an anonymous table it is BOUNDED by the
    /// table's own member run - the broad "improper child" predicate must never leak past
    /// the anonymous table and absorb ordinary siblings (`a <cell/><cell/> d`).
    fn row_run_members_for(&self, id: NodeId) -> Vec<NodeId> {
        let Some(parent) = self.doc.parent(id) else {
            return vec![id];
        };
        let parent_display = self.display_of(parent);
        if matches!(
            parent_display,
            Some(Display::Table | Display::TableRowGroup | Display::TableHeaderGroup | Display::TableFooterGroup)
        ) {
            let pd = parent_display.unwrap_or(Display::Table);
            let rstart = self
                .run_start_containing(parent, id, |c| self.needy_for_row(c, &pd))
                .unwrap_or(id);
            return self.run_members(rstart, |c| self.needy_for_row(c, &pd));
        }
        let table_needy = |c: NodeId| {
            self.display_of(c)
                .is_some_and(|d| needs_table_parent(&d, parent_display.as_ref()))
        };
        let tstart = self.run_start_containing(parent, id, table_needy).unwrap_or(id);
        let tmembers = self.run_members(tstart, table_needy);
        let rstart = self.sub_run_start(&tmembers, id, |c| self.needy_for_row(c, &Display::Table));
        self.members_sub_run(&tmembers, rstart, |c| self.needy_for_row(c, &Display::Table))
    }

    /// The real members of a synthetic anonymous box's run (the wrapper's flavour decides
    /// the run predicate).
    fn anon_box_members(&self, anon: NodeId) -> Vec<NodeId> {
        let raw = u64::from(anon);
        let first = decode_anon_box(anon);
        let Some(parent) = self.doc.parent(first) else {
            return vec![first];
        };
        let parent_display = self.display_of(parent);

        if is_anon_table_id(raw) {
            return self.run_members(first, |id| {
                self.display_of(id)
                    .is_some_and(|d| needs_table_parent(&d, parent_display.as_ref()))
            });
        }

        if is_anon_row_id(raw) {
            return self.row_run_members_for(first);
        }

        // Anonymous cell: bounded by the enclosing row's members unless the parent is a
        // real row (then the raw sibling run is the row's interior).
        if matches!(parent_display, Some(Display::TableRow)) {
            return self.run_members(first, |id| self.needy_for_cell(id));
        }
        let rmembers = self.row_run_members_for(first);
        self.members_sub_run(&rmembers, first, |c| self.needy_for_cell(c))
    }

    fn find_child_by_tag(&self, parent: NodeId, tag: &str) -> Option<NodeId> {
        self.doc
            .children(parent)
            .iter()
            .find(|&&child| self.doc.tag_name(child).is_some_and(|t| t.eq_ignore_ascii_case(tag)))
            .copied()
    }
}

impl<C> PipelineDocument for GosubDocumentAdapter<C>
where
    C: HasDocument + Send + Sync + 'static,
    C::Document: Send + Sync,
    <C::CssSystem as CssSystem>::PropertyMap: Send + Sync,
{
    fn root(&self) -> Option<NodeId> {
        self.html_node_id().or_else(|| Some(self.doc.root()))
    }

    fn children(&self, id: NodeId) -> Vec<NodeId> {
        if is_anon_table_id(u64::from(id)) {
            // The anonymous table's members may need row wrappers of their own.
            let members = self.anon_box_members(id);
            return self.wrap_anon_row_runs(Some(&Display::Table), members);
        }
        if is_anon_row_id(u64::from(id)) {
            // ...and an anonymous row's members may need cell wrappers.
            let members = self.anon_box_members(id);
            return self.wrap_anon_cell_runs(Some(&Display::TableRow), members);
        }
        if is_anon_cell_id(u64::from(id)) {
            return self.anon_box_members(id);
        }
        if is_pseudo_id(u64::from(id)) {
            let (owner, role) = decode_pseudo(id);
            // A pseudo-element's only child is its generated text (if any); text nodes are leaves.
            if role_is_text(role) {
                return Vec::new();
            }
            return match self.pseudo_box(owner, role_is_after(role)) {
                Some(pb) if pb.text.is_some() => {
                    let text_role = if role_is_after(role) {
                        ROLE_AFTER_TEXT
                    } else {
                        ROLE_BEFORE_TEXT
                    };
                    vec![encode_pseudo(owner, text_role)]
                }
                _ => Vec::new(),
            };
        }

        let mut out = Vec::new();
        // `::before` is inserted as the first child, `::after` as the last.
        if self.pseudo_box(id, false).is_some() {
            out.push(encode_pseudo(id, ROLE_BEFORE_ELEM));
        }
        out.extend(self.flat_children(id));
        if self.pseudo_box(id, true).is_some() {
            out.push(encode_pseudo(id, ROLE_AFTER_ELEM));
        }
        let display = self.display_of(id);
        let out = self.wrap_anon_table_runs(display.as_ref(), out);
        let out = self.wrap_anon_row_runs(display.as_ref(), out);
        self.wrap_anon_cell_runs(display.as_ref(), out)
    }

    fn node_kind(&self, id: NodeId) -> PipelineNodeKind {
        if is_anon_box_id(u64::from(id)) {
            return PipelineNodeKind::Element;
        }
        if is_pseudo_id(u64::from(id)) {
            let (_, role) = decode_pseudo(id);
            return if role_is_text(role) {
                PipelineNodeKind::Text
            } else {
                PipelineNodeKind::Element
            };
        }
        match self.doc.node_type(id) {
            GosubNodeType::TextNode => PipelineNodeKind::Text,
            GosubNodeType::CommentNode | GosubNodeType::DocTypeNode => PipelineNodeKind::Comment,
            GosubNodeType::ElementNode => PipelineNodeKind::Element,
            GosubNodeType::DocumentNode => PipelineNodeKind::Element,
            // A shadow root generates no box of its own; the flattened traversal yields its
            // children in the host's place, so this is only a belt-and-braces answer.
            GosubNodeType::ShadowRootNode => PipelineNodeKind::Comment,
        }
    }

    fn tag_name(&self, id: NodeId) -> Option<String> {
        // Pseudo-elements and anonymous table boxes have no tag name.
        if is_pseudo_id(u64::from(id)) || is_anon_box_id(u64::from(id)) {
            return None;
        }
        self.doc.tag_name(id).map(|s| s.to_string())
    }

    fn is_focused(&self, id: NodeId) -> bool {
        self.doc.is_focused(id)
    }

    fn control_edit_state(&self, id: NodeId) -> Option<gosub_interface::document::ControlEditState> {
        self.doc.control_edit_state(id)
    }

    fn is_checked(&self, id: NodeId) -> bool {
        self.doc.is_checked(id)
    }

    fn attribute(&self, id: NodeId, name: &str) -> Option<String> {
        self.doc.attribute(id, name).map(str::to_string)
    }

    fn selected_option(&self, select: NodeId) -> Option<NodeId> {
        self.doc.selected_option(select)
    }

    fn open_select(&self) -> Option<OpenPopup> {
        self.doc.open_select().map(|o| OpenPopup {
            select: o.select,
            hover: o.hover,
            active: o.active,
            first_row: o.first_row,
            viewport_top: o.viewport.0,
            viewport_height: o.viewport.1,
        })
    }

    fn control_size(&self, id: NodeId) -> Option<(f64, f64)> {
        self.doc.control_size(id)
    }

    fn transform_translate(&self, id: NodeId) -> Option<(LengthPercentage, LengthPercentage)> {
        let arc = if is_pseudo_id(u64::from(id)) {
            let (owner, role) = decode_pseudo(id);
            if role_is_text(role) {
                return None;
            }
            self.pseudo_box(owner, role_is_after(role))?.styles.clone()
        } else {
            self.cached_styles(id)
        };
        let p = <_ as CssPropertyMap<C::CssSystem>>::get(arc.as_ref(), "transform")?;
        translate_of::<C::CssSystem>(p)
    }

    fn is_display_none(&self, id: NodeId) -> bool {
        let style = self.cached_computed_style(id);
        style.has(Prop::Display) && style.box_group.display == Display::None
    }

    fn parent(&self, id: NodeId) -> Option<NodeId> {
        if let Some(&cached) = self.parent_cache.lock().get(&id) {
            return cached;
        }
        let parent = self.parent_uncached(id);
        self.parent_cache.lock().insert(id, parent);
        parent
    }

    fn computed_style(&self, id: NodeId) -> Arc<ComputedStyle> {
        self.cached_computed_style(id)
    }

    fn background_layers(&self, id: NodeId, box_size: (f32, f32)) -> Vec<Gradient> {
        if is_anon_box_id(u64::from(id)) {
            return Vec::new();
        }
        // Read the layers from the pseudo-element's own map, never the owner's.
        let arc = if is_pseudo_id(u64::from(id)) {
            let (owner, role) = decode_pseudo(id);
            if role_is_text(role) {
                return Vec::new();
            }
            match self.pseudo_box(owner, role_is_after(role)) {
                Some(pb) => pb.styles.clone(),
                None => return Vec::new(),
            }
        } else {
            self.cached_styles(id)
        };
        let map = arc.as_ref();

        let mut layers = Vec::new();
        for key in ["background-image", "background"] {
            if let Some(p) = <_ as CssPropertyMap<C::CssSystem>>::get(map, key) {
                layers = property_gradient_layers::<C::CssSystem>(p);
                if !layers.is_empty() {
                    break;
                }
            }
        }
        if layers.is_empty() {
            return Vec::new();
        }

        // `background-size/-position/-repeat` are per-layer lists cycled to the layer count.
        let read_groups = |key: &str| {
            <_ as CssPropertyMap<C::CssSystem>>::get(map, key)
                .map(bg_token_groups::<C::CssSystem>)
                .unwrap_or_default()
        };
        let size_groups = read_groups("background-size");
        let pos_groups = read_groups("background-position");
        let rep_groups = read_groups("background-repeat");
        let pick =
            |groups: &[Vec<BgTok>], i: usize| -> Option<usize> { (!groups.is_empty()).then(|| i % groups.len()) };

        for (i, g) in layers.iter_mut().enumerate() {
            let Some((tw, th)) = pick(&size_groups, i).and_then(|j| resolve_bg_size(&size_groups[j])) else {
                continue; // no explicit size -> fill the box (no tiling)
            };
            if tw <= 0.0 || th <= 0.0 {
                continue;
            }
            // Resolved against the painting area, as `compute_bg_tiling` does for raster images:
            // `right`, `center` and a percentage all mean a distance that depends on how much
            // wider the box is than the tile.
            let position = pick(&pos_groups, i)
                .map(|j| resolve_bg_position(&pos_groups[j]))
                .map(|(x, y)| (x.resolve(box_size.0, tw), y.resolve(box_size.1, th)))
                .unwrap_or((0.0, 0.0));
            let repeat = pick(&rep_groups, i)
                .map(|j| resolve_bg_repeat(&rep_groups[j]))
                .unwrap_or((true, true));
            g.tiling = Some(Tiling {
                tile_size: (tw, th),
                position,
                repeat,
            });
        }

        layers.into_iter().map(Gradient::Linear).collect()
    }

    fn background_image_layout(&self, id: NodeId) -> BgImageLayout {
        if is_anon_box_id(u64::from(id)) {
            return BgImageLayout::default();
        }
        let arc = self.cached_styles(id);
        let map = arc.as_ref();

        // The shorthand carries repeat/size keywords inline (`background: url(x) no-repeat center
        // / contain`) and then the longhands are usually empty, so scan both.
        let mut keywords: Vec<String> = Vec::new();
        let mut explicit_size: Option<(f32, f32)> = None;
        let mut position: Option<(BgAnchor, BgAnchor)> = None;

        let mut scan = |key: &str, read_size: bool, read_pos: bool| {
            let Some(p) = <_ as CssPropertyMap<C::CssSystem>>::get(map, key) else {
                return;
            };
            let groups = bg_token_groups::<C::CssSystem>(p);
            let Some(group) = groups.first() else {
                return;
            };
            if read_size && explicit_size.is_none() {
                explicit_size = resolve_bg_size(group);
            }
            if read_pos && position.is_none() && group.iter().any(is_position_token) {
                position = Some(resolve_bg_position(group));
            }
            for t in group {
                if let BgTok::Kw(k) = t {
                    keywords.push(k.clone());
                }
            }
        };
        // The shorthand mixes position and size (split by `/`); reading its bare lengths as a
        // position is unreliable, so only take position/size from the dedicated longhands.
        scan("background", false, false);
        scan("background-repeat", false, false);
        scan("background-size", true, false);
        scan("background-position", false, true);

        let has = |k: &str| keywords.iter().any(|s| s == k);
        let repeat = if has("no-repeat") {
            (false, false)
        } else if has("repeat-x") {
            (true, false)
        } else if has("repeat-y") {
            (false, true)
        } else {
            (true, true)
        };
        let size = match explicit_size {
            Some((w, h)) => BgSize::Length(w, h),
            None if has("cover") => BgSize::Cover,
            None if has("contain") => BgSize::Contain,
            None => BgSize::Auto,
        };
        // `background-position` from the longhand wins; otherwise the shorthand's own keywords
        // (`background: url(x) no-repeat center`) are read as a position.
        let position = position.unwrap_or_else(|| {
            let group: Vec<BgTok> = keywords
                .iter()
                .filter(|k| is_position_keyword(k))
                .map(|k| BgTok::Kw(k.clone()))
                .collect();
            resolve_bg_position(&group)
        });

        BgImageLayout { repeat, position, size }
    }

    fn clear_style_cache(&self) {
        self.style_cache.lock().clear();
        self.computed_cache.lock().clear();
        self.pseudo_cache.lock().clear();
        self.parent_cache.lock().clear();
    }

    fn invalidate_style_for_nodes(&self, ids: &[NodeId]) {
        let previous: Vec<(NodeId, Option<CachedStyles<C>>)> = {
            let mut cache = self.style_cache.lock();
            let mut computed_cache = self.computed_cache.lock();
            let mut pseudo_cache = self.pseudo_cache.lock();
            ids.iter()
                .map(|id| {
                    computed_cache.remove(id);
                    // The pseudo-elements' own structs hang off the owner's id too.
                    computed_cache.remove(&encode_pseudo(*id, ROLE_BEFORE_ELEM));
                    computed_cache.remove(&encode_pseudo(*id, ROLE_AFTER_ELEM));
                    computed_cache.remove(&encode_pseudo(*id, ROLE_BEFORE_TEXT));
                    computed_cache.remove(&encode_pseudo(*id, ROLE_AFTER_TEXT));
                    // Drop both pseudo-boxes belonging to this owner.
                    pseudo_cache.remove(&(*id, false));
                    pseudo_cache.remove(&(*id, true));
                    (*id, cache.remove(id))
                })
                .collect()
        };

        // Descendants were computed against these nodes' maps. Recompute now and, where the
        // inherited scope actually changed (a custom property set from `:hover`, say), drop
        // the subtree too; the common case - only the node's own properties moved - keeps
        // every descendant entry.
        for (id, old) in previous {
            let Some(old) = old else { continue };
            let fresh = self.cached_styles(id);
            if !fresh.inherited_scope_eq(&old) {
                self.invalidate_subtree(id);
            }
        }
        // A display change on any node can reshape the anonymous-wrapper runs around its
        // siblings, so the parent memo is dropped wholesale (it is cheap to rebuild).
        self.parent_cache.lock().clear();
    }

    fn html_node_id(&self) -> Option<NodeId> {
        let root = self.doc.root();
        self.find_child_by_tag(root, "html")
    }

    fn body_node_id(&self) -> Option<NodeId> {
        let html = self.html_node_id().or_else(|| Some(self.doc.root()))?;
        self.find_child_by_tag(html, "body")
    }

    fn base_url(&self) -> String {
        self.doc.url().map(|u| u.to_string()).unwrap_or_default()
    }

    fn inner_html(&self, id: NodeId) -> String {
        if is_pseudo_id(u64::from(id)) || is_anon_box_id(u64::from(id)) {
            return String::new();
        }
        self.doc.write_from_node(id)
    }

    fn get_node_by_id(&self, id: NodeId) -> Option<Node> {
        // Synthetic anonymous-table wrapper: a tagless `display: table` / `table-row` element.
        if let Some(d) = anon_box_display(u64::from(id)) {
            // An anonymous table generated in INLINE context is an inline-table (CSS 2.1
            // §17.2.1). We have no inline-table display; marking the synthetic NODE
            // inline-block makes the layouter's line grouping keep it (and the whitespace
            // around it) in the line box, while the computed style still reports
            // `table` to the converter, lattice, and painter.
            let node_display = if matches!(d, Display::Table) {
                let parent_inline = self.parent(id).is_some_and(|p| {
                    matches!(
                        self.display_of(p),
                        None | Some(Display::Inline | Display::InlineBlock | Display::InlineFlex | Display::InlineGrid)
                    ) && !matches!(self.doc.node_type(p), GosubNodeType::DocumentNode)
                });
                if parent_inline {
                    Display::InlineBlock
                } else {
                    d
                }
            } else {
                d
            };
            return Some(Node {
                node_id: id,
                parent_id: self.parent(id),
                children: self.children(id),
                node_type: NodeType::Element(ElementData::new(
                    String::new(),
                    Some(AttrMap::new()),
                    Some(node_display),
                )),
            });
        }
        // Synthetic pseudo nodes: build a transient Element (the box) or Text (its content).
        if is_pseudo_id(u64::from(id)) {
            let (owner, role) = decode_pseudo(id);
            let node_type = if role_is_text(role) {
                let text = self
                    .pseudo_box(owner, role_is_after(role))
                    .and_then(|pb| pb.text.clone());
                NodeType::Text(text.unwrap_or_default())
            } else {
                // Carry the computed `display` on the synthetic element so the layouter's
                // inline-vs-block grouping (which is tag-name based and would see an empty tag)
                // treats the pseudo-element correctly. ::before/::after default to inline.
                let display = self.cached_computed_style(id).box_group.display;
                NodeType::Element(ElementData::new(String::new(), Some(AttrMap::new()), Some(display)))
            };
            return Some(Node {
                node_id: id,
                parent_id: self.parent(id),
                children: self.children(id),
                node_type,
            });
        }

        // The flat tree, like the pseudo-element branch above and like `parent`/`children`
        // themselves. It has to be: a shadow root has no `Node` of its own (the match below ends
        // in `return None`), so reporting one as a parent makes the layouter drop the child - a
        // text node directly inside a shadow tree never got laid out.
        let parent_id = PipelineDocument::parent(self, id);
        let children = self.children(id);

        let node_type = match self.doc.node_type(id) {
            GosubNodeType::TextNode => {
                let text = self.doc.text_value(id).unwrap_or("").to_string();
                // Text nodes carry no own style; inheritance handled by get_style() chain.
                NodeType::Text(text)
            }
            GosubNodeType::CommentNode => {
                let comment = self.doc.comment_value(id).unwrap_or("").to_string();
                NodeType::Comment(comment)
            }
            GosubNodeType::ElementNode => {
                let tag_name = self.doc.tag_name(id).unwrap_or("").to_string();
                let mut attr_map = AttrMap::new();
                if let Some(attrs) = self.doc.attributes(id) {
                    for (k, v) in attrs {
                        attr_map.set(k, v);
                    }
                }
                // Style is normally read through `computed_style`, but the layouter's
                // inline-vs-block grouping reads the node alone - so carry the cascaded
                // `display` onto it for rules like `figcaption b { display: block }`.
                // Only when the cascade assigned one: `None` preserves the intrinsic tag-name
                // fallback, since the incomplete user-agent stylesheet makes the `inline`
                // initial value the wrong answer here.
                let style = self.cached_computed_style(id);
                // Same trick as anonymous inline-context tables: the Node carries
                // inline-block so line grouping keeps the element in the line box,
                // while the computed style (display_of and the explicit matches) reports
                // table structure to the converter, lattice, and painter.
                let display = style.has(Prop::Display).then(|| match style.box_group.display {
                    Display::InlineTable => Display::InlineBlock,
                    display => display,
                });
                let element_data = ElementData::new(tag_name, Some(attr_map), display);
                NodeType::Element(element_data)
            }
            _ => return None,
        };

        Some(Node {
            node_id: id,
            parent_id,
            children,
            node_type,
        })
    }
}

// ── Helpers used by the bridge ────────────────────────────────────────────────

/// Sum the translate functions of a `transform` list; `None` when there is no translation.
fn translate_of<S: CssSystem>(p: &S::Property) -> Option<(LengthPercentage, LengthPercentage)> {
    fn length<S: CssSystem>(v: &S::Value) -> Option<LengthPercentage> {
        if let Some(pct) = v.as_percentage() {
            return Some(LengthPercentage::Percent(pct));
        }
        if v.as_unit().is_some() {
            return Some(LengthPercentage::Px(v.unit_to_px()));
        }
        v.as_number().map(LengthPercentage::Px)
    }
    fn add(a: LengthPercentage, b: LengthPercentage) -> LengthPercentage {
        match (a, b) {
            (LengthPercentage::Px(x), LengthPercentage::Px(y)) => LengthPercentage::Px(x + y),
            (LengthPercentage::Percent(x), LengthPercentage::Percent(y)) => LengthPercentage::Percent(x + y),
            // Mixed px/% can't be summed without the box; keep the later one.
            (_, b) => b,
        }
    }
    let funcs: Vec<(&str, &[S::Value])> = match p.as_function() {
        Some(f) => vec![f],
        None => p.as_list()?.iter().filter_map(|v| v.as_function()).collect(),
    };
    let mut out: Option<(LengthPercentage, LengthPercentage)> = None;
    for (name, args) in funcs {
        let args: Vec<&S::Value> = args.iter().filter(|a| !a.is_comma()).collect();
        let zero = LengthPercentage::ZERO;
        let (dx, dy) = match name.cow_to_ascii_lowercase().as_ref() {
            "translate" => (
                args.first().and_then(|a| length::<S>(a)).unwrap_or(zero),
                args.get(1).and_then(|a| length::<S>(a)).unwrap_or(zero),
            ),
            "translatex" => (args.first().and_then(|a| length::<S>(a)).unwrap_or(zero), zero),
            "translatey" => (zero, args.first().and_then(|a| length::<S>(a)).unwrap_or(zero)),
            _ => continue,
        };
        out = Some(match out {
            None => (dx, dy),
            Some((x, y)) => (add(x, dx), add(y, dy)),
        });
    }
    out
}

#[cfg(test)]
mod bg_position_tests {
    use super::{resolve_bg_position, BgAnchor, BgTok};

    fn kw(k: &str) -> BgTok {
        BgTok::Kw(k.to_string())
    }

    /// The two-keyword form may be written in either order, so `center right` has to mean the same
    /// as `right center`. Taking the first value as the horizontal one put Wikipedia's
    /// external-link icon in the middle of every link.
    #[test]
    fn keyword_pairs_are_read_in_either_order() {
        let right_middle = (BgAnchor::End(0.0), BgAnchor::Percent(50.0));
        assert_eq!(resolve_bg_position(&[kw("right"), kw("center")]), right_middle);
        assert_eq!(resolve_bg_position(&[kw("center"), kw("right")]), right_middle);

        let middle_top = (BgAnchor::Percent(50.0), BgAnchor::Start(0.0));
        assert_eq!(resolve_bg_position(&[kw("center"), kw("top")]), middle_top);
        assert_eq!(resolve_bg_position(&[kw("top"), kw("center")]), middle_top);
    }

    #[test]
    fn one_value_centres_the_other_axis() {
        assert_eq!(
            resolve_bg_position(&[kw("right")]),
            (BgAnchor::End(0.0), BgAnchor::Percent(50.0))
        );
        assert_eq!(
            resolve_bg_position(&[kw("center")]),
            (BgAnchor::Percent(50.0), BgAnchor::Percent(50.0))
        );
        assert_eq!(
            resolve_bg_position(&[BgTok::Len(20.0)]),
            (BgAnchor::Start(20.0), BgAnchor::Percent(50.0))
        );
    }

    /// `right 10px` is an offset *from the right edge*, not a position 10px from the left.
    /// `center 4px` is the two-value form: the `center` is the horizontal value and the length is
    /// the vertical one. Letting the length take the first empty slot moved the tile sideways.
    #[test]
    fn a_leading_center_takes_the_horizontal_axis() {
        assert_eq!(
            resolve_bg_position(&[kw("center"), BgTok::Len(4.0)]),
            (BgAnchor::Percent(50.0), BgAnchor::Start(4.0))
        );
        assert_eq!(
            resolve_bg_position(&[kw("center"), BgTok::Pct(25.0)]),
            (BgAnchor::Percent(50.0), BgAnchor::Percent(25.0))
        );
        // A trailing `center` still means the vertical axis.
        assert_eq!(
            resolve_bg_position(&[BgTok::Len(4.0), kw("center")]),
            (BgAnchor::Start(4.0), BgAnchor::Percent(50.0))
        );
    }

    #[test]
    fn an_edge_keyword_swallows_the_length_after_it() {
        assert_eq!(
            resolve_bg_position(&[kw("right"), BgTok::Len(10.0), kw("bottom"), BgTok::Len(4.0)]),
            (BgAnchor::End(10.0), BgAnchor::End(4.0))
        );
    }

    #[test]
    fn lengths_and_percentages_fill_the_axes_in_order() {
        assert_eq!(
            resolve_bg_position(&[BgTok::Len(5.0), BgTok::Len(9.0)]),
            (BgAnchor::Start(5.0), BgAnchor::Start(9.0))
        );
        assert_eq!(
            resolve_bg_position(&[BgTok::Pct(50.0), BgTok::Pct(100.0)]),
            (BgAnchor::Percent(50.0), BgAnchor::Percent(100.0))
        );
    }

    /// The keywords of a `background` shorthand arrive in the same list as the position ones.
    #[test]
    fn non_position_keywords_are_ignored() {
        assert_eq!(
            resolve_bg_position(&[kw("no-repeat"), kw("right"), kw("cover")]),
            (BgAnchor::End(0.0), BgAnchor::Percent(50.0))
        );
        assert_eq!(
            resolve_bg_position(&[kw("no-repeat")]),
            (BgAnchor::Start(0.0), BgAnchor::Start(0.0))
        );
    }

    /// An anchor only becomes a pixel offset once the box and the tile are known.
    #[test]
    fn anchors_resolve_against_the_box() {
        assert_eq!(BgAnchor::Start(12.0).resolve(300.0, 20.0), 12.0);
        assert_eq!(BgAnchor::End(0.0).resolve(300.0, 20.0), 280.0);
        assert_eq!(BgAnchor::End(10.0).resolve(300.0, 20.0), 270.0);
        assert_eq!(BgAnchor::Percent(50.0).resolve(300.0, 20.0), 140.0);
        assert_eq!(BgAnchor::Percent(100.0).resolve(300.0, 20.0), 280.0);
    }
}
