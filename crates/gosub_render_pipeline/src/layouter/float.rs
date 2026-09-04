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
//! Text flows around a float rather than under it: [`line_box_insets`] turns the placed floats
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
use std::collections::HashMap;
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
        Float(FloatSide),
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
            if let Some(side) = float_side(doc, dom_id) {
                return Some((child_id, Role::Float(side)));
            }
            let (l, r) = clear_sides(doc, dom_id);
            Some((child_id, Role::InFlow(l, r)))
        })
        .collect();

    if !roles.iter().any(|(_, r)| matches!(r, Role::Float(_))) {
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
        let (child_id, side) = match role {
            Role::Float(side) => (child_id, side),
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
pub fn line_box_insets(layout_tree: &LayoutTree, placed: &[PlacedFloat]) -> HashMap<DomNodeId, (f32, f32)> {
    let mut insets: HashMap<DomNodeId, (f32, f32)> = HashMap::new();
    if placed.is_empty() {
        return insets;
    }

    for (&block_id, block) in layout_tree.arena.iter() {
        // Only blocks that actually hold text need an inset; a wrapper contributes nothing and
        // would double-count against its children.
        if !has_inline_content(layout_tree, block_id) {
            continue;
        }

        let content = block.box_model.content_box;
        if content.width <= 0.0 || content.height <= 0.0 {
            continue;
        }
        let block_top = content.y;
        let block_bottom = content.y + content.height;
        let block_left = content.x;
        let block_right = content.x + content.width;

        let mut left_inset: f64 = 0.0;
        let mut right_inset: f64 = 0.0;
        let mut float_cover: f64 = 0.0;

        for float in placed {
            // A float never displaces its own contents, nor the contents of anything inside it.
            if float.layout_id == block_id || is_ancestor(layout_tree, float.layout_id, block_id) {
                continue;
            }
            let r = float.rect;
            let overlaps_vertically = r.y < block_bottom && r.y + r.height > block_top;
            let overlaps_horizontally = r.x < block_right && r.x + r.width > block_left;
            if !overlaps_vertically || !overlaps_horizontally {
                continue;
            }
            float_cover = float_cover.max((r.y + r.height).min(block_bottom) - r.y.max(block_top));
            match float.side {
                FloatSide::Left => left_inset = left_inset.max(r.x + r.width - block_left),
                FloatSide::Right => right_inset = right_inset.max(block_right - r.x),
            }
        }

        if left_inset <= 0.0 && right_inset <= 0.0 {
            continue;
        }

        // One inset has to stand in for every line in the block, so only take it where it is
        // close to what per-line shortening would produce. Two guards bound the error:
        //
        // The float must cover most of the block. The inset is exact when the float spans the
        // whole block and drifts as more lines hang below it, so a float clipping the corner of a
        // long block is left alone rather than narrowing all of it.
        if float_cover < content.height * MIN_FLOAT_COVERAGE {
            continue;
        }

        // Enough width has to survive to be worth using. CSS puts a line that cannot fit beside a
        // float *below* it, which this model cannot express - it can only narrow. Squeezing text
        // into the sliver left by a near-full-width float would wrap it a word at a time and grow
        // the page enormously, so leave those lines full width instead.
        let line_width = content.width - left_inset - right_inset;
        if line_width < content.width * MIN_LINE_BOX_FRACTION {
            continue;
        }

        // Hand the caller the resolved line-box width rather than just the insets: an auto width
        // would have to be resolved against the block again, and the block's own width is already
        // known from this pass.
        insets.insert(block.dom_node_id, (left_inset as f32, line_width as f32));
    }

    insets
}

/// How much of a block's height a float must cover before the block's line boxes are inset.
const MIN_FLOAT_COVERAGE: f64 = 0.6;

/// How much of a block's width must survive the inset for it to be applied at all.
const MIN_LINE_BOX_FRACTION: f64 = 0.4;

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
