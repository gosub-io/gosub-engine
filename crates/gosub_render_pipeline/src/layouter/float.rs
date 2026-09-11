//! CSS float placement.
//!
//! Taffy has no concept of floats. The converter therefore hands Taffy every floated box as an
//! absolutely positioned one, so the *rest* of the flow is laid out as if the float were not
//! there - which is exactly what CSS 2.1 §9.5 asks for, since a float is out of normal flow.
//! What Taffy cannot do is put the float in the right place, so this pass runs after layout and
//! computes the real position from the float rules.
//!
//! Placement follows CSS 2.1 §9.5.1: a left float goes as far left (a right float as far right)
//! as it fits at its current vertical offset, never above the top of an earlier float in the same
//! block, and drops below the floats already there when it does not fit beside them.
//!
//! Text flows around a float rather than under it: [`resolve_bands_in_document_order`] turns the placed floats
//! into per-block insets that the layouter's second pass applies to its line boxes. A float's
//! position is only known after layout, so the insets are derived from the first pass and fed
//! back into a second one. The block itself keeps its full width - it is the line boxes that
//! narrow, so backgrounds and borders still span the float, as CSS requires.
//!
//! The inset is per block, not per line: every line box in a block clears the floats beside that
//! block, so lines that hang below a float's bottom edge stay narrower than CSS would have them.
//! Making those lines widen again means splitting a text node at the float's bottom, and the
//! pipeline keeps one layout element per DOM node, so a text box cannot yet be broken in two.

use crate::common::document::node::NodeId as DomNodeId;
use crate::common::document::pipeline_doc::PipelineDocument;
use crate::common::document::style::{lookup, StyleProperty, Value};
use crate::common::geo::Rect;
use crate::layouter::{ElementContext, LayoutElementId, LayoutTree};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Which edge a float is pinned to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FloatSide {
    Left,
    Right,
}

/// The float side declared on a node, or `None` for `float: none` and unset.
///
/// Per CSS 2.1 §9.7 `float` computes to `none` on an absolutely positioned box, so the caller is
/// responsible for checking `position` first where that matters.
pub fn float_side(doc: &dyn PipelineDocument, id: DomNodeId) -> Option<FloatSide> {
    match doc.get_own_style(id, &StyleProperty::Float) {
        Some(Value::Keyword(kw)) => match lookup(kw).as_str() {
            "left" => Some(FloatSide::Left),
            "right" => Some(FloatSide::Right),
            _ => None,
        },
        _ => None,
    }
}

/// True when `position` takes the box out of flow itself, in which case `float` does not apply.
pub fn position_is_out_of_flow(doc: &dyn PipelineDocument, id: DomNodeId) -> bool {
    match doc.get_own_style(id, &StyleProperty::Position) {
        Some(Value::Keyword(kw)) => matches!(lookup(kw).as_str(), "absolute" | "fixed"),
        _ => false,
    }
}

/// The sides a node clears, as `(left, right)`.
fn clear_sides(doc: &dyn PipelineDocument, id: DomNodeId) -> (bool, bool) {
    match doc.get_own_style(id, &StyleProperty::Clear) {
        Some(Value::Keyword(kw)) => match lookup(kw).as_str() {
            "left" => (true, false),
            "right" => (false, true),
            "both" => (true, true),
            _ => (false, false),
        },
        _ => (false, false),
    }
}

/// Whether a box establishes a block formatting context, and so grows to contain its floats.
///
/// Only the `overflow` trigger is recognised; `display: flow-root`, table cells and the other
/// BFC roots are not modelled by this pipeline yet. The document root always contains its
/// floats so the page scroll height includes them.
fn establishes_bfc(doc: &dyn PipelineDocument, id: DomNodeId) -> bool {
    [StyleProperty::OverflowX, StyleProperty::OverflowY]
        .iter()
        .any(|prop| match doc.get_own_style(id, prop) {
            Some(Value::Keyword(kw)) => !matches!(lookup(kw).as_str(), "visible" | "clip"),
            _ => false,
        })
}

/// A placed float, kept as the band of vertical space it occupies and the inner edge it pushes
/// later content to.
#[derive(Debug, Clone, Copy)]
struct Band {
    top: f64,
    bottom: f64,
    /// For a left float the x its right edge reaches; for a right float the x of its left edge.
    inner_edge: f64,
}

/// The float bands active inside one block container.
#[derive(Default)]
struct FloatContext {
    left: Vec<Band>,
    right: Vec<Band>,
}

impl FloatContext {
    /// The left content edge at vertical offset `y`, given the container's own left edge.
    fn left_edge_at(&self, y: f64, container_left: f64) -> f64 {
        self.left
            .iter()
            .filter(|b| y >= b.top && y < b.bottom)
            .map(|b| b.inner_edge)
            .fold(container_left, f64::max)
    }

    /// The right content edge at vertical offset `y`, given the container's own right edge.
    fn right_edge_at(&self, y: f64, container_right: f64) -> f64 {
        self.right
            .iter()
            .filter(|b| y >= b.top && y < b.bottom)
            .map(|b| b.inner_edge)
            .fold(container_right, f64::min)
    }

    /// The lowest band bottom strictly below `y`, i.e. the next offset where the available
    /// width can change. `None` when no float extends past `y`.
    fn next_edge_below(&self, y: f64) -> Option<f64> {
        self.left
            .iter()
            .chain(self.right.iter())
            .map(|b| b.bottom)
            .filter(|&b| b > y)
            .fold(None, |acc: Option<f64>, b| Some(acc.map_or(b, |a: f64| a.min(b))))
    }

    /// The bottom of the lowest float placed so far, which is where a float that fits nowhere
    /// else ends up.
    fn lowest_bottom(&self) -> f64 {
        self.left
            .iter()
            .chain(self.right.iter())
            .map(|b| b.bottom)
            .fold(f64::NEG_INFINITY, f64::max)
    }
}

/// A float after placement: what it is, which edge it took, and the area in-flow content has to
/// keep clear of. Used by the second layout pass to shorten line boxes beside it.
#[derive(Debug, Clone, Copy)]
pub struct PlacedFloat {
    pub layout_id: LayoutElementId,
    pub dom_id: DomNodeId,
    pub side: FloatSide,
    /// The exclusion area, in absolute page coordinates.
    pub rect: Rect,
}

/// Place every float in the tree and write the result back into the arena, returning what was
/// placed so a later pass can flow text around it.
pub fn post_process_floats(layout_tree: &mut LayoutTree) -> Vec<PlacedFloat> {
    let doc: Arc<dyn PipelineDocument> = Arc::clone(&layout_tree.render_tree.doc);
    let mut placed: Vec<PlacedFloat> = Vec::new();

    // Innermost containers first. A container that contains its floats grows to fit them, and an
    // outer container's clearfix has to see that final height - so every descendant must settle
    // before its ancestor. Placement itself is expressed relative to the container's content box,
    // and moving a box later translates its whole subtree rigidly, so positions computed early
    // stay correct when an ancestor moves.
    let mut order = Vec::new();
    let mut stack = vec![layout_tree.root_id];
    while let Some(id) = stack.pop() {
        order.push(id);
        if let Some(el) = layout_tree.arena.get(&id) {
            stack.extend(el.children.iter().copied());
        }
    }

    let mut deepest_float_bottom = f64::NEG_INFINITY;
    for id in order.into_iter().rev() {
        if let Some(bottom) = place_floats_in(&*doc, layout_tree, id, &mut placed) {
            deepest_float_bottom = deepest_float_bottom.max(bottom);
        }
    }

    // A float that no ancestor contains still counts towards the document's scrollable overflow,
    // so the page can be scrolled to the bottom of it. Grow the root to cover the lowest one
    // rather than clipping the page short.
    if !deepest_float_bottom.is_finite() {
        return placed;
    }
    let root_id = layout_tree.root_id;
    if let Some(root) = layout_tree.arena.get(&root_id) {
        let bottom = root.box_model.content_box.y + root.box_model.content_box.height;
        let growth = deepest_float_bottom - bottom;
        if growth > 0.0 {
            grow_and_propagate(&*doc, layout_tree, root_id, growth);
        }
    }

    placed
}

/// Place the direct floated children of one block container.
/// Returns the bottom of the lowest float placed here, if any.
fn place_floats_in(
    doc: &dyn PipelineDocument,
    layout_tree: &mut LayoutTree,
    container_id: LayoutElementId,
    placed: &mut Vec<PlacedFloat>,
) -> Option<f64> {
    let container = layout_tree.arena.get(&container_id)?;

    let children = container.children.clone();
    let content = container.box_model.content_box;
    let container_left = content.x;
    let container_right = content.x + content.width;

    // Classify once: a container with no floats and no `clear` costs only these lookups.
    enum Role {
        /// Side, plus the `clear` flags - `clear` applies to a float as much as to an in-flow
        /// box (CSS 2.1 §9.5.2), and it is what stacks a column of floated figures down one
        /// margin instead of letting them sit side by side.
        Float(FloatSide, bool, bool),
        InFlow(bool, bool),
        OutOfFlow,
    }
    let roles: Vec<(LayoutElementId, Role)> = children
        .iter()
        .filter_map(|&child_id| {
            let child = layout_tree.arena.get(&child_id)?;
            let dom_id = child.dom_node_id;
            if position_is_out_of_flow(doc, dom_id) {
                return Some((child_id, Role::OutOfFlow));
            }
            let (clear_left, clear_right) = clear_sides(doc, dom_id);
            if let Some(side) = float_side(doc, dom_id) {
                return Some((child_id, Role::Float(side, clear_left, clear_right)));
            }
            Some((child_id, Role::InFlow(clear_left, clear_right)))
        })
        .collect();

    if !roles.iter().any(|(_, r)| matches!(r, Role::Float(..))) {
        return None;
    }
    let mut ctx = FloatContext::default();

    // Where the next float's outer top goes. A float aligns with the current position in the
    // *normal flow*, so only in-flow siblings advance this - earlier floats are out of flow and
    // must not push a later float down the page. That distinction is what lets the negative-margin
    // column idiom (a 100%-wide float followed by rails with negative margins) sit on one line
    // instead of stacking.
    let mut flow_y = content.y;

    // Document order matters: a float is placed against the floats before it, and a cleared box
    // drops below exactly the floats that precede it.
    for (child_id, role) in roles {
        let (child_id, side, clear_left, clear_right) = match role {
            Role::Float(side, clear_left, clear_right) => (child_id, side, clear_left, clear_right),
            Role::OutOfFlow => continue,
            Role::InFlow(clear_left, clear_right) => {
                if clear_left || clear_right {
                    apply_clear(doc, layout_tree, &ctx, child_id, clear_left, clear_right);
                }
                if let Some(child) = layout_tree.arena.get(&child_id) {
                    let mb = child.box_model.margin_box;
                    flow_y = flow_y.max(mb.y + mb.height);
                }
                continue;
            }
        };

        let Some(child) = layout_tree.arena.get(&child_id) else {
            continue;
        };
        let margin_box = child.box_model.margin_box;
        let width = margin_box.width;

        // A float never rises above the top of its containing block, nor above the flow position
        // it was reached at.
        let mut y = flow_y.max(content.y);

        // `clear` on the float itself drops it below every earlier float on the cleared side,
        // before the fitting walk gets a chance to tuck it into a gap beside one. Wikipedia's
        // thumbnails are `float: right; clear: right`, which is what puts them in a single
        // column down the right margin rather than three abreast across the text.
        if clear_left || clear_right {
            let mut barrier = f64::NEG_INFINITY;
            if clear_left {
                barrier = ctx.left.iter().map(|b| b.bottom).fold(barrier, f64::max);
            }
            if clear_right {
                barrier = ctx.right.iter().map(|b| b.bottom).fold(barrier, f64::max);
            }
            if barrier.is_finite() {
                y = y.max(barrier);
            }
        }

        // Walk down until the float fits between the bands already at that offset, or until no
        // float extends further and it has to go below all of them.
        let placed_x = loop {
            let left = ctx.left_edge_at(y, container_left);
            let right = ctx.right_edge_at(y, container_right);

            if right - left >= width {
                break match side {
                    FloatSide::Left => left,
                    FloatSide::Right => right - width,
                };
            }

            match ctx.next_edge_below(y) {
                Some(next) if next > y => y = next,
                // Wider than the container itself: put it below everything and overflow.
                _ => {
                    let bottom = ctx.lowest_bottom();
                    if bottom.is_finite() && bottom > y {
                        y = bottom;
                    }
                    break match side {
                        FloatSide::Left => container_left,
                        FloatSide::Right => (container_right - width).max(container_left),
                    };
                }
            }
        };

        let band = Band {
            top: y,
            bottom: y + margin_box.height,
            inner_edge: match side {
                FloatSide::Left => placed_x + width,
                FloatSide::Right => placed_x,
            },
        };
        match side {
            FloatSide::Left => ctx.left.push(band),
            FloatSide::Right => ctx.right.push(band),
        }

        layout_tree.shift_subtree(child_id, placed_x - margin_box.x, y - margin_box.y);

        // Record the area in-flow content must avoid. Backgrounds paint the border box and a
        // negative margin can shrink the margin box below it, so exclude the union of the two.
        if let Some(el) = layout_tree.arena.get(&child_id) {
            let bm = &el.box_model;
            let m = bm.margin_box;
            let b = bm.border_box;
            let x = m.x.min(b.x);
            let top = m.y.min(b.y);
            let right = (m.x + m.width).max(b.x + b.width);
            let bottom = (m.y + m.height).max(b.y + b.height);
            placed.push(PlacedFloat {
                layout_id: child_id,
                dom_id: el.dom_node_id,
                side,
                rect: Rect::new(x, top, right - x, bottom - top),
            });
        }
    }

    let lowest = ctx.lowest_bottom();
    if !lowest.is_finite() {
        return None;
    }

    // A float is out of flow, so it does not raise its parent's height - unless the parent
    // establishes a block formatting context, which is what makes the clearfix and
    // `overflow: hidden` idioms work.
    let contains = layout_tree
        .arena
        .get(&container_id)
        .is_some_and(|c| establishes_bfc(doc, c.dom_node_id));
    if contains {
        if let Some(container) = layout_tree.arena.get(&container_id) {
            let content_bottom = container.box_model.content_box.y + container.box_model.content_box.height;
            let growth = lowest - content_bottom;
            if growth > 0.0 {
                grow_and_propagate(doc, layout_tree, container_id, growth);
            }
        }
    }

    Some(lowest)
}

/// Drop a cleared box below the floats it clears, taking its following siblings with it.
fn apply_clear(
    doc: &dyn PipelineDocument,
    layout_tree: &mut LayoutTree,
    ctx: &FloatContext,
    child_id: LayoutElementId,
    clear_left: bool,
    clear_right: bool,
) {
    let mut barrier = f64::NEG_INFINITY;
    if clear_left {
        barrier = ctx.left.iter().map(|b| b.bottom).fold(barrier, f64::max);
    }
    if clear_right {
        barrier = ctx.right.iter().map(|b| b.bottom).fold(barrier, f64::max);
    }
    if !barrier.is_finite() {
        return;
    }

    let Some(child) = layout_tree.arena.get(&child_id) else {
        return;
    };
    let delta = barrier - child.box_model.margin_box.y;
    if delta <= 0.0 {
        return;
    }

    layout_tree.shift_subtree(child_id, 0.0, delta);
    shift_following_siblings(layout_tree, child_id, delta);
    if let Some(parent) = layout_tree.arena.get(&child_id).and_then(|el| el.parent) {
        grow_and_propagate(doc, layout_tree, parent, delta);
    }
}

/// True when a box is outside normal flow, so its own size cannot affect its siblings or its
/// parent's height.
fn is_out_of_flow(doc: &dyn PipelineDocument, id: DomNodeId) -> bool {
    position_is_out_of_flow(doc, id) || float_side(doc, id).is_some()
}

/// Grow an element's boxes by `delta` vertically and move everything that sat below it, walking
/// up to the root so an inner growth reaches the page height.
///
/// The walk stops at the first out-of-flow box. A float's height does not contribute to its
/// parent and does not push its siblings down - without that stop, a tall float growing to contain
/// its own floats would drag every later sibling (such as the next column of a multi-column float
/// layout) down the page with it.
fn grow_and_propagate(doc: &dyn PipelineDocument, layout_tree: &mut LayoutTree, id: LayoutElementId, delta: f64) {
    if delta <= 0.0 {
        return;
    }

    let mut current = Some(id);
    while let Some(node_id) = current {
        let mut out_of_flow = false;
        if let Some(el) = layout_tree.arena.get_mut(&node_id) {
            let bm = &mut el.box_model;
            bm.content_box.height += delta;
            bm.padding_box.height += delta;
            bm.border_box.height += delta;
            bm.margin_box.height += delta;
        }
        if let Some(el) = layout_tree.arena.get(&node_id) {
            out_of_flow = is_out_of_flow(doc, el.dom_node_id);
        }
        if out_of_flow {
            return;
        }
        shift_following_siblings(layout_tree, node_id, delta);
        current = layout_tree.arena.get(&node_id).and_then(|el| el.parent);
    }
}

/// Move every later sibling of `id` (and their subtrees) down by `delta`.
fn shift_following_siblings(layout_tree: &mut LayoutTree, id: LayoutElementId, delta: f64) {
    let Some(parent_id) = layout_tree.arena.get(&id).and_then(|el| el.parent) else {
        return;
    };
    let Some(parent) = layout_tree.arena.get(&parent_id) else {
        return;
    };
    let siblings = parent.children.clone();
    let Some(pos) = siblings.iter().position(|&s| s == id) else {
        return;
    };
    for &sibling in &siblings[pos + 1..] {
        layout_tree.shift_subtree(sibling, 0.0, delta);
    }
}

/// The line-box geometry a block needs in order to clear the floats beside it, as
/// `(left inset, line width)` in CSS pixels, keyed by the block's DOM node.
///
/// A float's exclusion area is known only after layout, so this reads the geometry of a completed
/// pass and the caller applies it while building the next one. The inset covers the block as a
/// whole: every line box in it clears the float, not only the lines actually beside it. Lines that
/// hang below the float's bottom therefore stay narrower than CSS requires - correcting that needs
/// the text split at the float's bottom edge, which the atomic text boxes do not currently allow.
/// One horizontal band of a block: the line-box geometry that holds over a range of its height.
///
/// A block crossed by a float is not one shape - it is narrow beside the float and full width
/// below it - so a single inset per block cannot describe it. Bands give the inline layout the
/// geometry line by line: it fills a band, then moves to the next.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FloatBand {
    /// Left edge of the line boxes, relative to the block's content box.
    pub left_inset: f32,
    /// Width available to the line boxes in this band.
    pub line_width: f32,
    /// How tall the band is. `None` on the last band, which runs to the bottom of the block and
    /// so holds however many lines are left.
    pub height: Option<f32>,
}

/// Split a block's height into bands at the edges of the floats that reach it.
///
/// `None` when every band would be the full width, which means there is nothing for the inline
/// layout to do differently. The returned list always ends with an open-ended band: past the
/// lowest float the block is full width again and may run on for any number of lines.
fn bands_for_block(content: Rect, floats: &[(Rect, FloatSide)]) -> Option<Vec<FloatBand>> {
    let block_top = content.y;
    let block_bottom = content.y + content.height;
    let block_left = content.x;
    let block_right = content.x + content.width;

    // Deliberately *not* clipped to the block's current height: that height came from a pass laid
    // out with no insets at all, so it is the very thing the bands are about to change. Clipping
    // to it collapsed the common case - a float taller than the block it starts in - into one
    // narrow band that then applied to every line, however far the text ran on.
    let lowest = floats
        .iter()
        .map(|(r, _)| r.y + r.height)
        .fold(block_top, f64::max)
        .max(block_bottom);

    let mut edges: Vec<f64> = vec![block_top, lowest];
    for (r, _) in floats {
        for edge in [r.y, r.y + r.height] {
            if edge > block_top && edge < lowest {
                edges.push(edge);
            }
        }
    }
    edges.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    edges.dedup();

    let mut bands: Vec<FloatBand> = Vec::new();
    for pair in edges.windows(2) {
        let (top, bottom) = (pair[0], pair[1]);
        if bottom - top <= 0.0 {
            continue;
        }
        // A float applies to a band when it covers the band, which the midpoint decides: the
        // boundaries are exactly the float edges, so none starts or stops part-way through one.
        let mid = (top + bottom) / 2.0;
        let mut left_inset: f64 = 0.0;
        let mut right_inset: f64 = 0.0;
        for (r, side) in floats {
            if r.y > mid || r.y + r.height <= mid {
                continue;
            }
            match side {
                FloatSide::Left => left_inset = left_inset.max(r.x + r.width - block_left),
                FloatSide::Right => right_inset = right_inset.max(block_right - r.x),
            }
        }
        let left_inset = left_inset.clamp(0.0, content.width);
        let right_inset = right_inset.clamp(0.0, content.width);
        let band = FloatBand {
            left_inset: left_inset as f32,
            line_width: (content.width - left_inset - right_inset).max(0.0) as f32,
            height: Some((bottom - top) as f32),
        };
        // Merge with the previous band when the geometry is unchanged, so a paragraph beside two
        // stacked floats of the same width is one band, not two.
        match bands.last_mut() {
            Some(prev) if prev.left_inset == band.left_inset && prev.line_width == band.line_width => {
                if let (Some(h), Some(add)) = (prev.height, band.height) {
                    prev.height = Some(h + add);
                }
            }
            _ => bands.push(band),
        }
    }

    // Nothing to say when every band is the full width.
    if !bands
        .iter()
        .any(|b| b.left_inset > 0.0 || b.line_width < content.width as f32)
    {
        return None;
    }

    let tail = FloatBand {
        left_inset: 0.0,
        line_width: content.width as f32,
        height: None,
    };
    match bands.last_mut() {
        Some(last) if last.left_inset == tail.left_inset && last.line_width == tail.line_width => {
            last.height = None;
        }
        _ => bands.push(tail),
    }
    Some(bands)
}

/// How tall `block` becomes once its line boxes follow `bands`.
///
/// This is what lets the float resolution run as a single forward sweep instead of a fixed-point
/// iteration. Narrowing a block's lines makes it taller, which moves every float below it, which
/// changes *their* bands - so a sweep has to know the new height at the moment it decides the
/// bands, before it has laid anything out again. The widths it needs are already known: the
/// baseline pass measured every word box, and packing those widths into the bands is the same
/// greedy fill the inline layout will perform.
///
/// `None` when the block holds nothing measurable, in which case the caller keeps its height.
pub fn predict_banded_height(layout_tree: &LayoutTree, block: LayoutElementId, bands: &[FloatBand]) -> Option<f64> {
    let el = layout_tree.arena.get(&block)?;

    // Word widths and the line height, in document order, from the baseline layout.
    let mut widths: Vec<f64> = Vec::new();
    let mut line_height = 0.0_f64;
    for child in &el.children {
        let Some(child) = layout_tree.arena.get(child) else {
            continue;
        };
        match &child.context {
            ElementContext::Text(text) => {
                if line_height <= 0.0 {
                    line_height = text.font_info.line_height;
                }
                widths.push(child.box_model.border_box.width);
            }
            // A replaced element on the line takes its own width; anything else contributes
            // whatever box the baseline gave it.
            _ => widths.push(child.box_model.border_box.width),
        }
    }
    if widths.is_empty() || line_height <= 0.0 {
        return None;
    }

    Some(stacked_height(&widths, line_height, bands))
}

/// Pack `widths` into `bands` as whole lines and report the height that takes.
///
/// The greedy fill the inline layout performs, over the word widths a previous layout measured:
/// words go onto a line until the next will not fit, and when a band has no room for another line
/// the tail it leaves is skipped, because a line that would still touch the float starts below it.
fn stacked_height(widths: &[f64], line_height: f64, bands: &[FloatBand]) -> f64 {
    let mut height = 0.0_f64;
    let mut index = 0usize;
    let mut used_in_band = 0.0_f64;
    let mut line_used = 0.0_f64;

    for &width in widths {
        let band = bands[index.min(bands.len().saturating_sub(1))];
        if line_used > 0.0 && line_used + width > f64::from(band.line_width) {
            // Close the line and charge it to the band.
            height += line_height;
            used_in_band += line_height;
            line_used = 0.0;

            // Out of room for another whole line? Step to the next band, skipping the tail.
            if let Some(band_height) = band.height {
                if used_in_band + line_height > f64::from(band_height) {
                    height += (f64::from(band_height) - used_in_band).max(0.0);
                    used_in_band = 0.0;
                    if index + 1 < bands.len() {
                        index += 1;
                    }
                }
            }
        }
        line_used += width;
    }
    // The line left open at the end still occupies one line box.
    if line_used > 0.0 {
        height += line_height;
    }
    height
}

/// Resolve every block's float bands in one forward sweep, in document order.
///
/// The iterative version of this measured bands from a finished layout and fed them into the next
/// one, which cannot settle: narrowing a block makes it taller, a float of fixed height then meets
/// fewer blocks, those blocks lose their bands and are short again. On the Wikipedia article the
/// banded-block count cycled 133 -> 100 -> 92 -> 133 forever, so *where you stopped* decided the
/// page.
///
/// A browser has no such problem because it lays out in document order and places each float as it
/// meets it: a line box only ever sees floats above it, and information flows one way. This does
/// the same. Walking top to bottom, each block's bands come from the floats already placed, and
/// the height it will have once banded is predicted (see [`predict_banded_height`]) so that
/// everything below it - floats included - is offset by the growth before its own turn comes.
/// Nothing later can change an earlier answer, so one sweep is the answer.
pub fn resolve_bands_in_document_order(
    layout_tree: &LayoutTree,
    placed: &[PlacedFloat],
) -> HashMap<DomNodeId, Vec<FloatBand>> {
    let mut bands: HashMap<DomNodeId, Vec<FloatBand>> = HashMap::new();
    if placed.is_empty() {
        return bands;
    }

    // Floats by the box they were placed against, so the sweep can pick them up when it reaches
    // them rather than looking at all of them for every block.
    let by_id: HashMap<LayoutElementId, &PlacedFloat> = placed.iter().map(|f| (f.layout_id, f)).collect();

    // Floats seen so far, with their positions corrected for the growth of everything above them.
    // The id is kept so a float can be excluded from the blocks it contains.
    let mut seen: Vec<(LayoutElementId, Rect, FloatSide)> = Vec::new();
    // Growth accumulated from blocks that got taller once banded. Everything below moves by it.
    let mut shift = 0.0_f64;

    let mut stack = vec![layout_tree.root_id];
    let mut order: Vec<LayoutElementId> = Vec::new();
    while let Some(id) = stack.pop() {
        order.push(id);
        if let Some(el) = layout_tree.arena.get(&id) {
            stack.extend(el.children.iter().rev().copied());
        }
    }

    let mut added: HashSet<LayoutElementId> = HashSet::new();
    for id in &order {
        let id = *id;
        let Some(el) = layout_tree.arena.get(&id) else {
            continue;
        };

        if let Some(float) = by_id.get(&id) {
            // A float takes the shift of the content above it, then joins the context so every
            // block after it is measured against where it actually ends up.
            if added.insert(id) {
                let mut rect = float.rect;
                rect.y += shift;
                seen.push((id, rect, float.side));
            }
            continue;
        }

        if !has_inline_content(layout_tree, id) {
            continue;
        }

        // A float *inside* this block shortens this block's own line boxes - it sits at the top of
        // its container's content, so document order reaching the container first must not hide
        // it. Written as `<p><span class="thumb">…</span>text…</p>`, which is how a figure beside
        // a paragraph is usually marked up, the float is a descendant of the very block it
        // displaces.
        for (float_id, float) in &by_id {
            if added.contains(float_id) || !is_ancestor(layout_tree, id, *float_id) {
                continue;
            }
            added.insert(*float_id);
            let mut rect = float.rect;
            rect.y += shift;
            seen.push((*float_id, rect, float.side));
        }
        let mut content = el.box_model.content_box;
        if content.width <= 0.0 || content.height <= 0.0 {
            continue;
        }
        content.y += shift;

        // Only floats that are already placed can affect this block - that is the whole point.
        let relevant: Vec<(Rect, FloatSide)> = seen
            .iter()
            .filter(|(float_id, rect, _)| {
                // A float never displaces its own contents. Without this the infobox - itself a
                // float - banded every paragraph inside it against its own edges, and its caption
                // came out one character per line.
                if *float_id == id || is_ancestor(layout_tree, *float_id, id) {
                    return false;
                }
                rect.y < content.y + content.height
                    && rect.y + rect.height > content.y
                    && rect.x < content.x + content.width
                    && rect.x + rect.width > content.x
            })
            .map(|(_, rect, side)| (*rect, *side))
            .collect();
        if relevant.is_empty() {
            continue;
        }
        let Some(block_bands) = bands_for_block(content, &relevant) else {
            continue;
        };

        if let Some(height) = predict_banded_height(layout_tree, id, &block_bands) {
            shift += (height - content.height).max(0.0);
        }
        bands.insert(el.dom_node_id, block_bands);
    }

    if std::env::var("GOSUB_DEBUG_FLOAT_BANDS").is_ok() {
        eprintln!(
            "float bands: {} placed floats, {} blocks banded in one document-order sweep",
            placed.len(),
            bands.len()
        );
    }
    bands
}

/// Whether `node` holds inline content directly (text, or an inline box), meaning it is the block
/// whose line boxes a neighbouring float shortens.
fn has_inline_content(layout_tree: &LayoutTree, node: LayoutElementId) -> bool {
    let Some(el) = layout_tree.arena.get(&node) else {
        return false;
    };
    el.children.iter().any(|child| {
        layout_tree
            .arena
            .get(child)
            .is_some_and(|c| matches!(c.context, ElementContext::Text(_)))
    })
}

/// Whether `ancestor` is `node` or one of its ancestors.
fn is_ancestor(layout_tree: &LayoutTree, ancestor: LayoutElementId, node: LayoutElementId) -> bool {
    let mut current = Some(node);
    while let Some(id) = current {
        if id == ancestor {
            return true;
        }
        current = layout_tree.arena.get(&id).and_then(|el| el.parent);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn band(top: f64, bottom: f64, inner_edge: f64) -> Band {
        Band {
            top,
            bottom,
            inner_edge,
        }
    }

    fn rect(x: f64, y: f64, width: f64, height: f64) -> Rect {
        Rect::new(x, y, width, height)
    }

    /// A 600x100 block at the origin, the shape most of the band tests use.
    fn block() -> Rect {
        rect(0.0, 0.0, 600.0, 100.0)
    }

    fn open(line_width: f32) -> FloatBand {
        FloatBand {
            left_inset: 0.0,
            line_width,
            height: None,
        }
    }

    fn narrow(line_width: f32, height: f32) -> FloatBand {
        FloatBand {
            left_inset: 0.0,
            line_width,
            height: Some(height),
        }
    }

    #[test]
    fn height_without_a_float_is_just_the_lines() {
        // Six 50px words at a 100px width: two per line, three lines.
        let widths = vec![50.0; 6];
        assert_eq!(stacked_height(&widths, 20.0, &[open(100.0)]), 60.0);
    }

    #[test]
    fn a_narrower_band_makes_the_block_taller() {
        // The same words at half the width take twice as many lines. This is the number the
        // document-order sweep needs *before* it lays anything out: it is what moves every float
        // below this block, and getting it after the fact is what made the old iteration cycle.
        let widths = vec![50.0; 6];
        let tall = stacked_height(&widths, 20.0, &[open(50.0)]);
        let short = stacked_height(&widths, 20.0, &[open(100.0)]);
        assert!(tall > short, "{tall} vs {short}");
        assert_eq!(tall, 120.0);
    }

    #[test]
    fn text_widens_again_below_the_float() {
        // 40px of band holds two 20px lines at the narrow width; the rest runs at full width.
        // Four words fit two-to-a-line while narrow, then three-to-a-line after.
        let widths = vec![50.0; 10];
        let banded = stacked_height(&widths, 20.0, &[narrow(100.0, 40.0), open(200.0)]);
        let narrow_throughout = stacked_height(&widths, 20.0, &[open(100.0)]);
        assert!(banded < narrow_throughout, "{banded} vs {narrow_throughout}");
    }

    #[test]
    fn a_bands_unused_tail_is_skipped() {
        // One 50px word in a band 100px tall: the line takes 20px and the remaining 80px are
        // skipped, because the next line would still be beside the float.
        let widths = vec![50.0, 50.0, 50.0];
        // Two words fill the 50px-wide band's single line, the third starts below the band.
        let height = stacked_height(&widths, 20.0, &[narrow(60.0, 25.0), open(200.0)]);
        assert!(height >= 45.0, "the tail of the first band must be skipped: {height}");
    }

    #[test]
    fn an_empty_run_has_no_height() {
        assert_eq!(stacked_height(&[], 20.0, &[open(100.0)]), 0.0);
    }

    #[test]
    fn a_float_shorter_than_the_block_gives_two_bands() {
        let bands = bands_for_block(block(), &[(rect(400.0, 0.0, 200.0, 40.0), FloatSide::Right)])
            .expect("the float narrows the block");
        assert_eq!(
            bands,
            vec![
                FloatBand {
                    left_inset: 0.0,
                    line_width: 400.0,
                    height: Some(40.0)
                },
                FloatBand {
                    left_inset: 0.0,
                    line_width: 600.0,
                    height: None
                },
            ]
        );
    }

    #[test]
    fn a_float_taller_than_the_block_still_ends() {
        // The block's height came from a pass with no insets, so it is *shorter* than the text
        // will be once the bands apply. Clipping the bands to it made the narrow band open-ended
        // and every line of the paragraph narrow, however far it ran past the float.
        let bands = bands_for_block(block(), &[(rect(400.0, 0.0, 200.0, 260.0), FloatSide::Right)])
            .expect("the float narrows the block");
        assert_eq!(bands.len(), 2, "{bands:?}");
        assert_eq!(bands[0].height, Some(260.0));
        assert_eq!(bands[0].line_width, 400.0);
        assert_eq!(bands[1].height, None);
        assert_eq!(bands[1].line_width, 600.0);
    }

    #[test]
    fn a_left_float_indents_rather_than_narrowing_only() {
        let bands = bands_for_block(block(), &[(rect(0.0, 0.0, 150.0, 40.0), FloatSide::Left)])
            .expect("the float indents the block");
        assert_eq!(bands[0].left_inset, 150.0);
        assert_eq!(bands[0].line_width, 450.0);
        assert_eq!(bands[1].left_inset, 0.0);
    }

    #[test]
    fn floats_on_both_sides_narrow_from_both() {
        let bands = bands_for_block(
            block(),
            &[
                (rect(0.0, 0.0, 100.0, 40.0), FloatSide::Left),
                (rect(500.0, 0.0, 100.0, 40.0), FloatSide::Right),
            ],
        )
        .expect("both floats apply");
        assert_eq!(bands[0].left_inset, 100.0);
        assert_eq!(bands[0].line_width, 400.0);
    }

    #[test]
    fn stacked_floats_of_one_width_are_a_single_band() {
        // Two thumbnails down the same margin should read as one narrow band, not two identical
        // ones - the cursor charges lines against band heights, so splitting them would place a
        // line break at the seam between the floats.
        let bands = bands_for_block(
            block(),
            &[
                (rect(400.0, 0.0, 200.0, 40.0), FloatSide::Right),
                (rect(400.0, 40.0, 200.0, 40.0), FloatSide::Right),
            ],
        )
        .expect("the floats narrow the block");
        assert_eq!(bands.len(), 2, "{bands:?}");
        assert_eq!(bands[0].height, Some(80.0));
        assert_eq!(bands[0].line_width, 400.0);
    }

    #[test]
    fn a_float_starting_below_the_top_leaves_a_full_width_band_above_it() {
        let bands = bands_for_block(block(), &[(rect(400.0, 30.0, 200.0, 40.0), FloatSide::Right)])
            .expect("the float narrows the block");
        assert_eq!(bands.len(), 3, "{bands:?}");
        assert_eq!((bands[0].line_width, bands[0].height), (600.0, Some(30.0)));
        assert_eq!((bands[1].line_width, bands[1].height), (400.0, Some(40.0)));
        assert_eq!((bands[2].line_width, bands[2].height), (600.0, None));
    }

    #[test]
    fn a_float_that_misses_the_block_horizontally_gives_no_bands() {
        // Nothing overlaps, so every band would be full width and there is nothing to say.
        assert!(bands_for_block(block(), &[]).is_none());
    }

    #[test]
    fn edges_narrow_only_within_a_band() {
        let mut ctx = FloatContext::default();
        ctx.left.push(band(0.0, 100.0, 120.0));
        ctx.right.push(band(0.0, 50.0, 700.0));

        // Inside both bands the usable strip lies between the two inner edges.
        assert_eq!(ctx.left_edge_at(10.0, 0.0), 120.0);
        assert_eq!(ctx.right_edge_at(10.0, 800.0), 700.0);

        // Below the right float's bottom the right edge opens back up.
        assert_eq!(ctx.right_edge_at(60.0, 800.0), 800.0);

        // Below both, the container's own edges apply again - `bottom` is exclusive.
        assert_eq!(ctx.left_edge_at(100.0, 0.0), 0.0);
        assert_eq!(ctx.right_edge_at(100.0, 800.0), 800.0);
    }

    #[test]
    fn stacked_left_floats_take_the_furthest_edge() {
        let mut ctx = FloatContext::default();
        ctx.left.push(band(0.0, 100.0, 120.0));
        ctx.left.push(band(0.0, 100.0, 240.0));
        assert_eq!(ctx.left_edge_at(50.0, 0.0), 240.0);
    }

    #[test]
    fn next_edge_below_finds_the_nearest_band_bottom() {
        let mut ctx = FloatContext::default();
        ctx.left.push(band(0.0, 100.0, 120.0));
        ctx.right.push(band(0.0, 50.0, 700.0));

        assert_eq!(ctx.next_edge_below(0.0), Some(50.0));
        assert_eq!(ctx.next_edge_below(50.0), Some(100.0));
        assert_eq!(ctx.next_edge_below(100.0), None);
        assert_eq!(ctx.lowest_bottom(), 100.0);
    }

    #[test]
    fn an_empty_context_reports_the_container_edges() {
        let ctx = FloatContext::default();
        assert_eq!(ctx.left_edge_at(0.0, 16.0), 16.0);
        assert_eq!(ctx.right_edge_at(0.0, 784.0), 784.0);
        assert_eq!(ctx.next_edge_below(0.0), None);
        assert!(!ctx.lowest_bottom().is_finite());
    }
}
