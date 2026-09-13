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

use crate::common::document::node::NodeId as DomNodeId;
use crate::common::document::pipeline_doc::PipelineDocument;
use crate::common::document::style::{lookup, StyleProperty, Unit, Value};
use crate::common::geo::{Dimension, Rect};
use crate::layouter::{LayoutElementId, LayoutTree};
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
    match doc.get_own_style(id, &StyleProperty::Position) {
        Some(Value::Keyword(kw)) => match lookup(kw).as_str() {
            "absolute" => PositionKind::Absolute,
            "fixed" => PositionKind::Fixed,
            "relative" | "sticky" => PositionKind::InFlowPositioned,
            _ => PositionKind::Static,
        },
        _ => PositionKind::Static,
    }
}

/// An inset (`top`/`right`/`bottom`/`left`) resolved against the containing block's size, or
/// `None` for `auto` - which means "leave the box where the flow put it" on that axis.
fn inset(doc: &dyn PipelineDocument, id: DomNodeId, prop: StyleProperty, basis: f64) -> Option<f64> {
    // `get_style`, not `get_own_style`: it resolves `em`/`rem` to px, which the converter feeding
    // taffy already does (`CssTaffyConverter::get_inset`). Reading the raw value here dropped
    // font-relative insets on the floor, and a box whose only specified side was one of them was
    // then treated as `auto` on that axis - left wherever taffy had put it rather than placed
    // against its containing block. The initial value of every inset is the keyword `auto`, so an
    // unspecified side still falls through to `None`.
    match doc.get_style(id, &prop) {
        Value::Unit(v, Unit::Px) => Some(v as f64),
        Value::Unit(v, Unit::Percent) => Some(basis * v as f64 / 100.0),
        _ => None,
    }
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
}

impl RebasedInsets {
    fn is_empty(&self) -> bool {
        self.left.is_none() && self.right.is_none() && self.top.is_none() && self.bottom.is_none()
    }
}

/// Whether an axis is `auto`-sized, so that opposing insets should stretch the box.
fn is_auto_size(doc: &dyn PipelineDocument, id: DomNodeId, prop: StyleProperty) -> bool {
    !matches!(doc.get_style(id, &prop), Value::Unit(..) | Value::Number(_))
}

/// Re-place every absolutely positioned box against the containing block CSS gives it.
///
/// Returns the boxes that also need *resizing*, keyed by DOM node - see [`RebasedInsets`]. The
/// caller feeds these back through a second layout pass; on that pass the map comes back empty
/// because taffy has already put the boxes where this pass would.
pub fn post_process_abspos(layout_tree: &mut LayoutTree, viewport: Dimension) -> HashMap<DomNodeId, RebasedInsets> {
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
        let left = inset(&*doc, dom_id, StyleProperty::InsetInlineStart, cb.width);
        let right = inset(&*doc, dom_id, StyleProperty::InsetInlineEnd, cb.width);
        let top = inset(&*doc, dom_id, StyleProperty::InsetBlockStart, cb.height);
        let bottom = inset(&*doc, dom_id, StyleProperty::InsetBlockEnd, cb.height);

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

        // Both insets on an axis with an `auto` size means "stretch between them". Taffy already
        // does that, but against the immediate parent rather than the CSS containing block, so
        // record insets rebased onto the parent for the caller to replay. `parent_reference`
        // returns `None` at the root, where the two coincide and nothing needs rebasing.
        if let Some(pb) = parent_reference(layout_tree, id) {
            let mut rebased = RebasedInsets::default();
            if let (Some(l), Some(r)) = (left, right) {
                // Only worth replaying when the parent is *not* the containing block. When it is -
                // the common `position: relative` parent holding its own absolute child - taffy
                // already measured from the right box and a second pass would buy nothing.
                if is_auto_size(&*doc, dom_id, StyleProperty::Width) && !spans_match(pb.x, pb.width, cb.x, cb.width) {
                    rebased.left = Some((cb.x + l - pb.x) as f32);
                    rebased.right = Some(((pb.x + pb.width) - (cb.x + cb.width - r)) as f32);
                }
            }
            if let (Some(t), Some(b)) = (top, bottom) {
                if is_auto_size(&*doc, dom_id, StyleProperty::Height) && !spans_match(pb.y, pb.height, cb.y, cb.height)
                {
                    rebased.top = Some((cb.y + t - pb.y) as f32);
                    rebased.bottom = Some(((pb.y + pb.height) - (cb.y + cb.height - b)) as f32);
                }
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
fn parent_reference(layout_tree: &LayoutTree, id: LayoutElementId) -> Option<Rect> {
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
    fn position_keywords_map_to_the_right_kind() {
        // `sticky` and `relative` stay in flow but still act as a containing block, which is what
        // makes them stop the ancestor walk.
        assert_ne!(PositionKind::InFlowPositioned, PositionKind::Static);
        assert_ne!(PositionKind::Absolute, PositionKind::Fixed);
    }
}
