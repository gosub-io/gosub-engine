//! How far the page scrolls: the viewport's scrollable overflow (CSS Overflow 3, §2.2).
//!
//! The page is not the root element's box. A root with `height: 100%` is exactly one window
//! tall, and a body taller than that overflows it: the overflow is still the page, and the
//! window scrolls over it. Taking the root's box as the page made that content unreachable -
//! nothing past the first window drew, and nothing scrolled to it.
//!
//! The extent is the root's margin box grown by the border box of every box under it, with
//! four exceptions:
//! - a box that clips (`overflow` other than `visible`) adds its own box, not what it clips -
//!   on the axis it clips: `overflow-x: clip` beside a visible `overflow-y` still lets a tall
//!   child make the page taller;
//! - a `position: fixed` box sits in the window, not on the page, and adds nothing;
//! - a box with no area (zero width or height) adds nothing of its own;
//! - the root's `overflow`, or the body's when the root's is `visible`, is the window's, and that
//!   box itself then clips nothing (§3.3). A window that is `hidden` or `clip` on an axis does
//!   not scroll past the root's box on it; `auto` and `scroll` still scroll over everything.
//!
//! Only the far edges count: the page starts at the origin, and overflow above or left of it is
//! out of reach in any browser.

use crate::common::document::node::NodeId as DomNodeId;
use crate::common::document::pipeline_doc::PipelineDocument;
use crate::common::geo::Dimension;
use crate::layouter::LayoutTree;
use gosub_interface::style::{Overflow, Position, Prop};

/// `id`'s `overflow` on each axis, as computed: `visible` beside a value that scrolls or hides
/// becomes `auto` (CSS Overflow 3, §3.1), so only `visible` beside `visible` or `clip` stays.
fn overflow(doc: &dyn PipelineDocument, id: DomNodeId) -> (Overflow, Overflow) {
    let style = doc.computed_style(id);
    let x = if style.has(Prop::OverflowX) {
        style.box_group.overflow_x
    } else {
        Overflow::Visible
    };
    let y = if style.has(Prop::OverflowY) {
        style.box_group.overflow_y
    } else {
        Overflow::Visible
    };
    let scrolls = |v| !matches!(v, Overflow::Visible | Overflow::Clip);
    match (x, y) {
        (Overflow::Visible, y) if scrolls(y) => (Overflow::Auto, y),
        (x, Overflow::Visible) if scrolls(x) => (x, Overflow::Auto),
        both => both,
    }
}

/// Whether `id`'s box clips what overflows it, horizontally and vertically.
fn clips(doc: &dyn PipelineDocument, id: DomNodeId) -> (bool, bool) {
    let (x, y) = overflow(doc, id);
    (x != Overflow::Visible, y != Overflow::Visible)
}

fn is_fixed(doc: &dyn PipelineDocument, id: DomNodeId) -> bool {
    let style = doc.computed_style(id);
    style.has(Prop::Position) && style.box_group.position == Position::Fixed
}

/// The size of the page the window scrolls over, measured from the origin.
pub fn scrollable_extent(layout_tree: &LayoutTree) -> Dimension {
    let Some(root) = layout_tree.get_node_by_id(layout_tree.root_id) else {
        return layout_tree.root_dimension;
    };
    let root_box = root.box_model.margin_box;
    let mut right = root_box.x + root_box.width;
    let mut bottom = root_box.y + root_box.height;

    // The window takes the root's `overflow`, or the body's when the root's is `visible`, and the
    // box it came from clips nothing itself. The window then stops at the root's box on an axis
    // it hides or clips; `auto` and `scroll` scroll over the overflow.
    let doc = layout_tree.render_tree.doc.as_ref();
    let visible = (Overflow::Visible, Overflow::Visible);
    let html = doc.html_node_id();
    let propagated = match html {
        Some(id) if overflow(doc, id) != visible => Some(id),
        _ => doc.body_node_id(),
    };
    let (window_x, window_y) = propagated.map_or(visible, |id| overflow(doc, id));
    let stops = |v| matches!(v, Overflow::Hidden | Overflow::Clip);
    let (window_x, window_y) = (stops(window_x), stops(window_y));

    // Each box with whether something above it already clips it on each axis: a clipped axis
    // takes nothing more from that subtree.
    let mut stack: Vec<_> = root.children.iter().map(|&id| (id, window_x, window_y)).collect();
    while let Some((id, clipped_x, clipped_y)) = stack.pop() {
        if clipped_x && clipped_y {
            continue;
        }
        let Some(el) = layout_tree.get_node_by_id(id) else {
            continue;
        };
        if is_fixed(doc, el.dom_node_id) {
            continue;
        }
        // A box with no area adds nothing of its own, though what overflows it still does.
        let border_box = el.box_model.border_box;
        if border_box.width > 0.0 && border_box.height > 0.0 {
            if !clipped_x {
                right = right.max(border_box.x + border_box.width);
            }
            if !clipped_y {
                bottom = bottom.max(border_box.y + border_box.height);
            }
        }
        let (clips_x, clips_y) = if Some(el.dom_node_id) == propagated {
            (false, false)
        } else {
            clips(doc, el.dom_node_id)
        };
        let (clipped_x, clipped_y) = (clipped_x || clips_x, clipped_y || clips_y);
        stack.extend(el.children.iter().map(|&child| (child, clipped_x, clipped_y)));
    }

    Dimension::new(right.max(0.0), bottom.max(0.0))
}
