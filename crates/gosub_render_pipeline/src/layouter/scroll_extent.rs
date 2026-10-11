//! How far the page scrolls: the viewport's scrollable overflow (CSS Overflow 3, §2.2).
//!
//! The page is not the root element's box. A root with `height: 100%` is exactly one window
//! tall, and a body taller than that overflows it: the overflow is still the page, and the
//! window scrolls over it. Taking the root's box as the page made that content unreachable -
//! nothing past the first window drew, and nothing scrolled to it.
//!
//! The extent is the root's margin box grown by the border box of every box under it, with
//! three exceptions:
//! - a box that clips (`overflow` other than `visible`) adds its own box, not what it clips;
//! - a `position: fixed` box sits in the window, not on the page, and adds nothing;
//! - a root (or, failing that, body) whose `overflow` is not `visible` passes it on to the
//!   window, which then does not scroll past the root's box.
//!
//! Only the far edges count: the page starts at the origin, and overflow above or left of it is
//! out of reach in any browser.

use crate::common::document::node::NodeId as DomNodeId;
use crate::common::document::pipeline_doc::PipelineDocument;
use crate::common::geo::Dimension;
use crate::layouter::LayoutTree;
use gosub_interface::style::{Overflow, Position, Prop};

/// Whether `id`'s box clips what overflows it.
fn clips(doc: &dyn PipelineDocument, id: DomNodeId) -> bool {
    let style = doc.computed_style(id);
    (style.has(Prop::OverflowX) && style.box_group.overflow_x != Overflow::Visible)
        || (style.has(Prop::OverflowY) && style.box_group.overflow_y != Overflow::Visible)
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

    // The window takes the root's `overflow`, or the body's when the root's is `visible`. Either
    // way, a clipping value there means the window clips at the root's box.
    let doc = layout_tree.render_tree.doc.as_ref();
    let window_clips = [doc.html_node_id(), doc.body_node_id()]
        .into_iter()
        .flatten()
        .any(|id| clips(doc, id));

    if !window_clips {
        let mut stack = root.children.clone();
        while let Some(id) = stack.pop() {
            let Some(el) = layout_tree.get_node_by_id(id) else {
                continue;
            };
            if is_fixed(doc, el.dom_node_id) {
                continue;
            }
            let border_box = el.box_model.border_box;
            if border_box.width > 0.0 || border_box.height > 0.0 {
                right = right.max(border_box.x + border_box.width);
                bottom = bottom.max(border_box.y + border_box.height);
            }
            if !clips(doc, el.dom_node_id) {
                stack.extend(el.children.iter().copied());
            }
        }
    }

    Dimension::new(right.max(0.0), bottom.max(0.0))
}
