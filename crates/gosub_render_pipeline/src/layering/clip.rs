//! Where each element may paint: a box whose `overflow` is not `visible` clips its descendants to
//! its padding box (CSS Overflow 3, §3). Clips follow the containing block chain, not the DOM: an
//! absolutely positioned box escapes a clipping ancestor that sits below its containing block, and
//! a fixed box escapes them all.

use crate::common::geo::Rect;
use crate::layouter::scroll_extent::{clips, viewport_overflow_node};
use crate::layouter::{LayoutElementId, LayoutTree};
use gosub_interface::style::{Display, Float, Position, Prop};
use std::collections::HashMap;

/// Stands in for "no edge" on an axis that does not clip.
const FAR: f64 = 1e9;

/// The overlap of `a` and `b`, empty (zero-sized) when they do not overlap.
pub fn intersect(a: Rect, b: Rect) -> Rect {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    let right = (a.x + a.width).min(b.x + b.width);
    let bottom = (a.y + a.height).min(b.y + b.height);
    Rect::new(x, y, (right - x).max(0.0), (bottom - y).max(0.0))
}

/// The clip of every element that has one, in page coordinates.
pub fn overflow_clips(tree: &LayoutTree) -> HashMap<LayoutElementId, Rect> {
    let doc = tree.render_tree.doc.as_ref();
    let viewport = viewport_overflow_node(doc);
    let mut out = HashMap::new();

    // (element, clip on an in-flow box here, clip on an absolutely positioned box here)
    let mut stack: Vec<(LayoutElementId, Option<Rect>, Option<Rect>)> = vec![(tree.root_id, None, None)];
    while let Some((id, flow, abs)) = stack.pop() {
        let Some(el) = tree.get_node_by_id(id) else {
            continue;
        };
        let style = doc.computed_style(el.dom_node_id);
        let position = if style.has(Prop::Position) {
            style.box_group.position
        } else {
            Position::Static
        };
        let own = match position {
            Position::Fixed => None,
            Position::Absolute => abs,
            _ => flow,
        };
        if let Some(rect) = own {
            out.insert(id, rect);
        }

        // The box the window takes its overflow from clips nothing itself, and an inline box
        // has no overflow to clip (§3 "applies to block containers, flex and grid containers").
        // A positioned or floated one is blockified.
        let floated = style.has(Prop::Float) && style.box_group.float != Float::None;
        let inline = style.has(Prop::Display)
            && style.box_group.display == Display::Inline
            && !matches!(position, Position::Absolute | Position::Fixed)
            && !floated;
        let (clip_x, clip_y) = if Some(el.dom_node_id) == viewport || inline {
            (false, false)
        } else {
            clips(doc, el.dom_node_id)
        };
        let inner = if clip_x || clip_y {
            let p = el.box_model.padding_box;
            let (x, width) = if clip_x { (p.x, p.width) } else { (-FAR, 2.0 * FAR) };
            let (y, height) = if clip_y { (p.y, p.height) } else { (-FAR, 2.0 * FAR) };
            let rect = Rect::new(x, y, width, height);
            Some(own.map_or(rect, |own| intersect(own, rect)))
        } else {
            own
        };
        // A positioned box is the containing block of the absolutely positioned boxes under it.
        let inner_abs = if position == Position::Static { abs } else { inner };

        for &child_id in &el.children {
            // A box split into several layout elements (a table and its border overlay) is not
            // clipped by itself.
            let same_box = tree
                .get_node_by_id(child_id)
                .is_some_and(|child| child.dom_node_id == el.dom_node_id);
            if same_box {
                stack.push((child_id, own, abs));
            } else {
                stack.push((child_id, inner, inner_abs));
            }
        }
    }
    out
}
