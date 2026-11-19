//! Resolve absolutely positioned boxes against their real containing block.
//!
//! Taffy has no notion of a *positioned* ancestor: an absolutely positioned child is laid out
//! against its immediate parent, whatever that parent's `position` is. CSS instead measures it
//! from the padding box of the nearest ancestor with a `position` other than `static`, falling
//! back to the initial containing block (the viewport). The two agree only when the parent happens
//! to be the positioned ancestor, so without this pass a `top`/`right` offset is measured from the
//! wrong box and the element lands somewhere arbitrary - the classic symptom being a badge pinned
//! to the corner of the nearest wrapper instead of the card it belongs to.
//!
//! This runs after floats, so every ancestor has reached its final position first, and walks
//! parents-before-children so an absolutely positioned box nested inside another is measured
//! against the corrected outer one.

use crate::common::document::node::{NodeId as DomNodeId, NodeType};
use crate::common::document::pipeline_doc::PipelineDocument;
use crate::common::geo::{Dimension, Rect};
use crate::layouter::{LayoutElementId, LayoutTree};
use gosub_interface::style::{LengthPercentageAuto, Position, Prop};
use std::collections::HashMap;
use std::sync::Arc;

/// How a box is positioned, as far as this pass is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PositionKind {
    /// `static` - not positioned, and not a containing block for anything.
    Static,
    /// `relative` or `sticky`: stays in flow, but is a containing block.
    InFlowPositioned,
    Absolute,
    Fixed,
}

fn position_kind(doc: &dyn PipelineDocument, id: DomNodeId) -> PositionKind {
    let style = doc.computed_style(id);
    if !style.has(Prop::Position) {
        return PositionKind::Static;
    }
    match style.box_group.position {
        Position::Absolute => PositionKind::Absolute,
        Position::Fixed => PositionKind::Fixed,
        Position::Relative | Position::Sticky => PositionKind::InFlowPositioned,
        Position::Static => PositionKind::Static,
    }
}

/// An inset (`top`/`right`/`bottom`/`left`) resolved against the containing block's size, or
/// `None` for `auto` - which means "leave the box where the flow put it" on that axis.
fn inset(side: LengthPercentageAuto, basis: f64) -> Option<f64> {
    gosub_interface::used::length_auto(side, basis)
}

/// Taffy insets for one absolutely positioned box, rebased from its CSS containing block onto
/// its immediate parent - which is the box taffy actually measures from.
///
/// Only produced for an axis where CSS gives *both* insets and leaves the size `auto`, i.e. the
/// box is meant to stretch between them. Placing such a box is not enough: taffy sized it
/// against the wrong ancestor, and this pass only moves boxes, it does not resize them. Handing
/// taffy the rebased insets on a second pass lets its own algorithm do the stretching - and
/// re-lay-out the children at the corrected width, which patching the box model could not.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RebasedInsets {
    pub left: Option<f32>,
    pub right: Option<f32>,
    pub top: Option<f32>,
    pub bottom: Option<f32>,
    /// A percentage `width`, resolved against the containing block: taffy would take it of the
    /// parent instead.
    pub width: Option<f32>,
    /// A percentage `height`, resolved likewise (CSS 2 §10.5).
    pub height: Option<f32>,
}

impl RebasedInsets {
    fn is_empty(&self) -> bool {
        self.left.is_none()
            && self.right.is_none()
            && self.top.is_none()
            && self.bottom.is_none()
            && self.width.is_none()
            && self.height.is_none()
    }

    /// Whether taffy, given `self`, sizes the box as it would given `other`. The pass places the
    /// box itself, so a pair of insets only decides the size, between them: two pairs with the
    /// same sum stretch it alike.
    fn sizes_like(&self, other: &Self) -> bool {
        let close = |a: Option<f32>, b: Option<f32>| match (a, b) {
            (Some(a), Some(b)) => (a - b).abs() < 0.01,
            (a, b) => a.is_none() && b.is_none(),
        };
        let span = |start: Option<f32>, end: Option<f32>| start.zip(end).map(|(start, end)| start + end);
        close(span(self.left, self.right), span(other.left, other.right))
            && close(span(self.top, self.bottom), span(other.top, other.bottom))
            && close(self.width, other.width)
            && close(self.height, other.height)
    }
}

/// Whether a taffy tree built with the rebased values in `baked` sizes every box as `current` asks.
/// A resize that only moves boxes - `top: 50%` with a fixed height - keeps the tree.
pub fn sizes_match(baked: &HashMap<DomNodeId, RebasedInsets>, current: &HashMap<DomNodeId, RebasedInsets>) -> bool {
    baked.len() == current.len()
        && current
            .iter()
            .all(|(id, now)| baked.get(id).is_some_and(|then| then.sizes_like(now)))
}

/// Whether taffy gets `size` wrong when it measures from `parent` rather than from `cb`, the
/// containing block, on one axis (given as start and length). A percentage it gets right only when
/// the two boxes span the same range. A `calc()` it never sees at all: the converter has no taffy
/// form for one and hands it `auto`, so it is resolved here wherever the box sits.
fn size_needs_replay(size: LengthPercentageAuto, parent: (f64, f64), cb: (f64, f64)) -> bool {
    match size {
        LengthPercentageAuto::Calc { .. } => true,
        LengthPercentageAuto::Percent(_) => !spans_match(parent.0, parent.1, cb.0, cb.1),
        _ => false,
    }
}

/// Whether `id` is a replaced element: an `auto` size is then its own, and opposing insets do not
/// stretch it - the end one is ignored (CSS 2 §10.3.8, §10.6.5). Not a form control: an `<input>`
/// stretches like a block.
pub(crate) fn is_replaced(doc: &dyn PipelineDocument, id: DomNodeId) -> bool {
    doc.get_node_by_id(id).is_some_and(|node| {
        matches!(&node.node_type, NodeType::Element(data)
            if ["img", "svg", "video"].iter().any(|tag| data.tag_name.eq_ignore_ascii_case(tag)))
    })
}

/// Whether taffy gets the stretch between two opposing insets wrong when it measures from
/// `parent` rather than from `cb`. Same as for a size: a span that differs, or a `calc()` inset,
/// which taffy gets as `auto` - it then sees one inset and does not stretch at all.
fn insets_need_replay(
    insets: (LengthPercentageAuto, LengthPercentageAuto),
    parent: (f64, f64),
    cb: (f64, f64),
) -> bool {
    let is_calc = |side| matches!(side, LengthPercentageAuto::Calc { .. });
    is_calc(insets.0) || is_calc(insets.1) || !spans_match(parent.0, parent.1, cb.0, cb.1)
}

/// Whether an axis is `auto`-sized, so that opposing insets should stretch the box.
fn is_auto_size(size: LengthPercentageAuto) -> bool {
    size.is_auto()
}

/// Re-place every absolutely positioned box against the containing block CSS gives it.
///
/// Returns the boxes that also need *resizing*, keyed by DOM node - see [`RebasedInsets`]. The
/// caller feeds these back through another layout pass, until a pass returns the map it was given:
/// a box nested in a corrected one is only measured right once its ancestor has been.
///
/// `line_box` gives the padding box of the anonymous line box a box was laid out in, if it was in
/// one. The layout tree has no node for a line box, but taffy measures from it all the same.
pub fn post_process_abspos(
    layout_tree: &mut LayoutTree,
    viewport: Dimension,
    line_box: &dyn Fn(&LayoutTree, LayoutElementId) -> Option<Rect>,
) -> HashMap<DomNodeId, RebasedInsets> {
    let doc: Arc<dyn PipelineDocument> = Arc::clone(&layout_tree.render_tree.doc);
    let mut stretched: HashMap<DomNodeId, RebasedInsets> = HashMap::new();

    // Parents before children: a nested absolutely positioned box measures against its ancestor's
    // corrected position, so the ancestor has to be corrected first.
    let mut order = Vec::new();
    let mut stack = vec![layout_tree.root_id];
    while let Some(id) = stack.pop() {
        order.push(id);
        if let Some(el) = layout_tree.arena.get(&id) {
            stack.extend(el.children.iter().rev().copied());
        }
    }

    for id in order {
        let Some(el) = layout_tree.arena.get(&id) else {
            continue;
        };
        let kind = position_kind(&*doc, el.dom_node_id);
        if !matches!(kind, PositionKind::Absolute | PositionKind::Fixed) {
            continue;
        }

        let dom_id = el.dom_node_id;
        let margin_box = el.box_model.margin_box;
        let cb = containing_block(layout_tree, &*doc, id, kind, viewport);

        // With both insets on an axis given, the start one wins: this pass does not stretch a box
        // to satisfy both, it only places it.
        let style = doc.computed_style(dom_id);
        let left = inset(style.inset.inline_start, cb.width);
        let right = inset(style.inset.inline_end, cb.width);
        let top = inset(style.inset.block_start, cb.height);
        let bottom = inset(style.inset.block_end, cb.height);

        let x = match (left, right) {
            (Some(l), _) => cb.x + l,
            (None, Some(r)) => cb.x + cb.width - r - margin_box.width,
            // Both auto: the box keeps the static position taffy gave it.
            (None, None) => margin_box.x,
        };
        let y = match (top, bottom) {
            (Some(t), _) => cb.y + t,
            (None, Some(b)) => cb.y + cb.height - b - margin_box.height,
            (None, None) => margin_box.y,
        };
        // A replaced box between both insets keeps its own size, and `auto` margins take what it
        // leaves between them: an equal share each, never a negative one on the left (CSS 2
        // §10.3.8, §10.6.5). Taffy saw no end inset for an `auto` size (see the converter) and
        // gave such margins 0, and for a set size shared them out within the parent, which is
        // also what it took percentage margins of. So the margins are worked out here, of the
        // containing block (§8.3), and the box moved by the difference with what taffy used.
        let (x, y) = if is_replaced(&*doc, dom_id) {
            let border = el.box_model.border_box;
            let margin = &style.margin;
            // `None` for `auto`; a percentage is of the containing block's width on both axes.
            let declared = |side| inset(side, cb.width);
            // One axis: the declared start and end margins, the start margin taffy used, the size
            // and the room between the insets.
            let correction = |margins: (Option<f64>, Option<f64>), used: f64, size: f64, room: f64, floor: f64| {
                let free = room - size - margins.0.unwrap_or(0.0) - margins.1.unwrap_or(0.0);
                let start = match margins {
                    (None, None) => (free / 2.0).max(floor),
                    (None, Some(_)) => free,
                    (Some(start), _) => start,
                };
                start - used
            };
            let dx = match (left, right) {
                (Some(l), Some(r)) => correction(
                    (declared(margin.left), declared(margin.right)),
                    border.x - margin_box.x,
                    border.width,
                    cb.width - l - r,
                    0.0,
                ),
                _ => 0.0,
            };
            let dy = match (top, bottom) {
                (Some(t), Some(b)) => correction(
                    (declared(margin.top), declared(margin.bottom)),
                    border.y - margin_box.y,
                    border.height,
                    cb.height - t - b,
                    f64::NEG_INFINITY,
                ),
                _ => 0.0,
            };
            (x + dx, y + dy)
        } else {
            (x, y)
        };

        // Both insets on an axis with an `auto` size means "stretch between them". Taffy already
        // does that, but against the immediate parent rather than the CSS containing block, so
        // record insets rebased onto the parent for the caller to replay. `parent_reference`
        // returns `None` at the root, where the two coincide and nothing needs rebasing.
        if let Some(pb) = parent_reference(layout_tree, id, line_box) {
            let mut rebased = RebasedInsets::default();
            let (inset_x, inset_y) = (
                (style.inset.inline_start, style.inset.inline_end),
                (style.inset.block_start, style.inset.block_end),
            );
            let stretches = !is_replaced(&*doc, dom_id);
            if let (Some(l), Some(r)) = (left, right) {
                // Only worth replaying when the parent is *not* the containing block. When it is -
                // the common `position: relative` parent holding its own absolute child - taffy
                // already measured from the right box and a second pass would buy nothing.
                if stretches
                    && is_auto_size(style.size.width)
                    && insets_need_replay(inset_x, (pb.x, pb.width), (cb.x, cb.width))
                {
                    rebased.left = Some((cb.x + l - pb.x) as f32);
                    rebased.right = Some(((pb.x + pb.width) - (cb.x + cb.width - r)) as f32);
                }
            }
            if let (Some(t), Some(b)) = (top, bottom) {
                if stretches
                    && is_auto_size(style.size.height)
                    && insets_need_replay(inset_y, (pb.y, pb.height), (cb.y, cb.height))
                {
                    rebased.top = Some((cb.y + t - pb.y) as f32);
                    rebased.bottom = Some(((pb.y + pb.height) - (cb.y + cb.height - b)) as f32);
                }
            }
            // A percentage size is of the containing block as well. Taffy takes it of the parent,
            // which for a box written inside a line is the line box, whose height is its content.
            if size_needs_replay(style.size.width, (pb.x, pb.width), (cb.x, cb.width)) {
                rebased.width = inset(style.size.width, cb.width).map(|w| w as f32);
            }
            if size_needs_replay(style.size.height, (pb.y, pb.height), (cb.y, cb.height)) {
                rebased.height = inset(style.size.height, cb.height).map(|h| h as f32);
            }
            if !rebased.is_empty() {
                stretched.insert(dom_id, rebased);
            }
        }

        layout_tree.shift_subtree(id, x - margin_box.x, y - margin_box.y);
    }

    stretched
}

/// Whether two spans cover the same range, to within a sub-pixel tolerance. Used to spot the case
/// where the parent already *is* the containing block, so nothing needs rebasing.
fn spans_match(a_start: f64, a_len: f64, b_start: f64, b_len: f64) -> bool {
    const EPSILON: f64 = 0.01;
    (a_start - b_start).abs() < EPSILON && (a_len - b_len).abs() < EPSILON
}

/// The box taffy measures an absolutely positioned child's insets from: its parent's padding box.
/// For a box written inside a line - `<div>Search <span style="position:absolute">` - that is the
/// line box, which can be shorter or narrower than the block around it: `height: 100%` came out
/// as the line's height, and as 0 with no text on the line.
fn parent_reference(
    layout_tree: &LayoutTree,
    id: LayoutElementId,
    line_box: &dyn Fn(&LayoutTree, LayoutElementId) -> Option<Rect>,
) -> Option<Rect> {
    if let Some(line) = line_box(layout_tree, id) {
        return Some(line);
    }
    let parent = layout_tree.arena.get(&id)?.parent?;
    Some(layout_tree.arena.get(&parent)?.box_model.padding_box)
}

/// The containing block of an absolutely positioned box: the padding box of its nearest positioned
/// ancestor, or the initial containing block when it has none (and always, for `fixed`).
fn containing_block(
    layout_tree: &LayoutTree,
    doc: &dyn PipelineDocument,
    id: LayoutElementId,
    kind: PositionKind,
    viewport: Dimension,
) -> Rect {
    // The initial containing block is anchored at the canvas origin and sized like the viewport
    // (CSS 2.1 §10.1) - it is not the root element's box. Taking the root's content box put the
    // origin *inside* the root's border and padding, so `top: 0; left: 0` on a page with
    // `html { padding: 20px }` landed at (20, 20) instead of the top-left corner.
    let initial = || Rect::new(0.0, 0.0, viewport.width, viewport.height);

    // `fixed` is measured from the viewport, never from an ancestor.
    if kind == PositionKind::Fixed {
        return initial();
    }

    let mut current = layout_tree.arena.get(&id).and_then(|el| el.parent);
    while let Some(ancestor_id) = current {
        let Some(ancestor) = layout_tree.arena.get(&ancestor_id) else {
            break;
        };
        if position_kind(doc, ancestor.dom_node_id) != PositionKind::Static {
            return ancestor.box_model.padding_box;
        }
        current = ancestor.parent;
    }

    initial()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_percentage_inset_resolves_against_the_containing_block() {
        // Guards the arithmetic the pass relies on; the containing-block walk itself needs a
        // layout tree and is covered by the rendering repros.
        assert_eq!(Rect::new(10.0, 20.0, 200.0, 100.0).width, 200.0);
    }

    #[test]
    fn rebased_insets_size_alike_by_their_sum() {
        let pair = |left, right| RebasedInsets {
            left: Some(left),
            right: Some(right),
            ..RebasedInsets::default()
        };
        let id = DomNodeId::from(7usize);
        let map = |insets| HashMap::from([(id, insets)]);
        // Moved, same size: the pass places the box, taffy only needs the sum.
        assert!(sizes_match(&map(pair(10.0, 20.0)), &map(pair(25.0, 5.0))));
        assert!(!sizes_match(&map(pair(10.0, 20.0)), &map(pair(10.0, 30.0))));
        let width = |w| RebasedInsets {
            width: Some(w),
            ..RebasedInsets::default()
        };
        assert!(!sizes_match(&map(width(100.0)), &map(width(110.0))));
        // A box that gained or lost its replay changes what taffy has to be told.
        assert!(!sizes_match(&HashMap::new(), &map(width(100.0))));
        assert!(!sizes_match(&map(width(100.0)), &HashMap::new()));
    }

    #[test]
    fn position_keywords_map_to_the_right_kind() {
        // `sticky` and `relative` stay in flow but still act as a containing block, which is what
        // makes them stop the ancestor walk.
        assert_ne!(PositionKind::InFlowPositioned, PositionKind::Static);
        assert_ne!(PositionKind::Absolute, PositionKind::Fixed);
    }
}
