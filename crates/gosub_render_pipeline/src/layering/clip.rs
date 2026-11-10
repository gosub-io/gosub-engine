//! Where each element may paint.
//!
//! A box whose `overflow` is not `visible` clips its descendants to its padding box (CSS Overflow
//! 3, §3). Those clips follow the containing block chain, not the DOM: an absolutely positioned box
//! escapes a clipping ancestor that sits below its containing block, and a fixed box escapes them
//! all. `clip` and `clip-path: inset()` clip the box itself and everything under it, fixed boxes
//! included.

use crate::common::geo::Rect;
use crate::layouter::scroll_extent::{clips, viewport_overflow_node};
use crate::layouter::{LayoutElementId, LayoutTree};
use gosub_interface::style::{ComputedStyle, Display, Float, Position, Prop};
use gosub_interface::used::length;
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

/// Both clips; `None` is no clip.
fn and(a: Option<Rect>, b: Option<Rect>) -> Option<Rect> {
    match (a, b) {
        (Some(a), Some(b)) => Some(intersect(a, b)),
        (a, b) => a.or(b),
    }
}

/// What clips a box placed under some element, by how the box is positioned.
#[derive(Clone, Copy, Default)]
struct Clips {
    /// An in-flow, relative or sticky box.
    flow: Option<Rect>,
    /// An absolutely positioned box: only what clips at or above its containing block.
    abs: Option<Rect>,
    /// A fixed box: only `clip` and `clip-path`.
    fixed: Option<Rect>,
}

/// The clip of every element that has one, in page coordinates.
pub fn paint_clips(tree: &LayoutTree) -> HashMap<LayoutElementId, Rect> {
    let doc = tree.render_tree.doc.as_ref();
    let viewport = viewport_overflow_node(doc);
    let mut out = HashMap::new();

    let mut stack = vec![(tree.root_id, Clips::default())];
    while let Some((id, clips_here)) = stack.pop() {
        let Some(el) = tree.get_node_by_id(id) else {
            continue;
        };
        let style = doc.computed_style(el.dom_node_id);
        let position = if style.has(Prop::Position) {
            style.box_group.position
        } else {
            Position::Static
        };
        let shape = shape_clip(el.box_model.border_box, &style, position);
        let own = and(
            match position {
                Position::Fixed => clips_here.fixed,
                Position::Absolute => clips_here.abs,
                _ => clips_here.flow,
            },
            shape,
        );
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
        let overflow = (clip_x || clip_y).then(|| {
            let p = el.box_model.padding_box;
            let (x, width) = if clip_x { (p.x, p.width) } else { (-FAR, 2.0 * FAR) };
            let (y, height) = if clip_y { (p.y, p.height) } else { (-FAR, 2.0 * FAR) };
            Rect::new(x, y, width, height)
        });
        let inner = and(own, overflow);
        let under = Clips {
            flow: inner,
            // A positioned box is the containing block of the absolutely positioned boxes under it.
            abs: if position == Position::Static {
                and(clips_here.abs, shape)
            } else {
                inner
            },
            fixed: and(clips_here.fixed, shape),
        };

        for &child_id in &el.children {
            // A box split into several layout elements (a table and its border overlay) is not
            // clipped by itself.
            let same_box = tree
                .get_node_by_id(child_id)
                .is_some_and(|child| child.dom_node_id == el.dom_node_id);
            stack.push((child_id, if same_box { clips_here } else { under }));
        }
    }
    out
}

/// What the box's own `clip` and `clip-path` leave of it, from its border box.
fn shape_clip(border_box: Rect, style: &ComputedStyle, position: Position) -> Option<Rect> {
    let b = border_box;
    let rect = style
        .box_group
        .clip
        .filter(|_| style.has(Prop::Clip) && matches!(position, Position::Absolute | Position::Fixed))
        .map(|[top, right, bottom, left]| {
            let x = b.x + left.map_or(0.0, f64::from);
            let y = b.y + top.map_or(0.0, f64::from);
            let right = b.x + right.map_or(b.width, f64::from);
            let bottom = b.y + bottom.map_or(b.height, f64::from);
            Rect::new(x, y, (right - x).max(0.0), (bottom - y).max(0.0))
        });
    let path = style
        .box_group
        .clip_path
        .filter(|_| style.has(Prop::ClipPath))
        .map(|[top, right, bottom, left]| {
            let (top, bottom) = (length(top, b.height), length(bottom, b.height));
            let (left, right) = (length(left, b.width), length(right, b.width));
            Rect::new(
                b.x + left,
                b.y + top,
                (b.width - left - right).max(0.0),
                (b.height - top - bottom).max(0.0),
            )
        });
    and(rect, path)
}
