//! Which elements are focusable and where focus moves on a click or Tab. The state itself lives
//! on the DOM document, where the `:focus` selectors read it (like `:hover`).

use crate::html::{EngineDocument, RenderConfiguration};
use cow_utils::CowUtils;
use gosub_interface::document::Document as _;
use gosub_interface::node::NodeType;
use gosub_shared::node::NodeId;
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focusability {
    No,
    /// `tabindex="-1"`: click/script only, skipped by Tab.
    ClickOnly,
    /// In tab order; the `tabindex` value (0 = document order).
    Sequential(i32),
}

const FORM_CONTROLS: [&str; 4] = ["input", "textarea", "select", "button"];

/// `contenteditable` makes an element editable only when the attribute is empty or `true`;
/// `contenteditable="false"` explicitly opts out. Matching `is_text_input` in `context.rs`.
pub fn is_contenteditable<C: RenderConfiguration>(doc: &EngineDocument<C>, id: NodeId) -> bool {
    doc.attribute(id, "contenteditable")
        .is_some_and(|v| v.is_empty() || v.eq_ignore_ascii_case("true"))
}

pub fn focusability<C: RenderConfiguration>(doc: &EngineDocument<C>, id: NodeId) -> Focusability {
    if doc.node_type(id) != NodeType::ElementNode {
        return Focusability::No;
    }
    let Some(tag) = doc.tag_name(id) else {
        return Focusability::No;
    };
    let is_control = FORM_CONTROLS.contains(&tag);
    if is_control && doc.attribute(id, "disabled").is_some() {
        return Focusability::No;
    }
    if let Some(ti) = doc.attribute(id, "tabindex").and_then(|v| v.trim().parse::<i32>().ok()) {
        return if ti < 0 {
            Focusability::ClickOnly
        } else {
            Focusability::Sequential(ti)
        };
    }
    let natural = match tag {
        "input" => !doc
            .attribute(id, "type")
            .is_some_and(|t| t.eq_ignore_ascii_case("hidden")),
        "textarea" | "select" | "button" | "summary" | "iframe" => true,
        "a" | "area" => doc.attribute(id, "href").is_some(),
        _ => is_contenteditable(doc, id),
    };
    if natural {
        Focusability::Sequential(0)
    } else {
        Focusability::No
    }
}

/// Whether a *click* shows the focus ring: browsers do for text-entry controls, not for buttons,
/// checkboxes or links. Keyboard focus always shows it.
pub fn click_shows_ring<C: RenderConfiguration>(doc: &EngineDocument<C>, id: NodeId) -> bool {
    match doc.tag_name(id) {
        Some("textarea") | Some("select") => true,
        Some("input") => !doc.attribute(id, "type").is_some_and(|t| {
            matches!(
                t.cow_to_ascii_lowercase().as_ref(),
                "button" | "submit" | "reset" | "checkbox" | "radio" | "range" | "color" | "file" | "image"
            )
        }),
        _ => is_contenteditable(doc, id),
    }
}

/// The nearest focusable ancestor-or-self of `leaf`, or the control a `<label>` on that path is
/// bound to.
pub fn click_target<C: RenderConfiguration>(doc: &EngineDocument<C>, leaf: NodeId) -> Option<NodeId> {
    let mut id = leaf;
    loop {
        if focusability(doc, id) != Focusability::No {
            return Some(id);
        }
        if doc.tag_name(id) == Some("label") {
            if let Some(control) = label_control(doc, id) {
                return Some(control);
            }
        }
        id = doc.parent(id)?;
    }
}

/// The control a `<label>` activates: its `for` target, else the first descendant control.
fn label_control<C: RenderConfiguration>(doc: &EngineDocument<C>, label: NodeId) -> Option<NodeId> {
    if let Some(target) = doc.attribute(label, "for").and_then(|f| doc.node_by_named_id(f)) {
        return (focusability(doc, target) != Focusability::No).then_some(target);
    }
    let mut stack: Vec<NodeId> = doc.children(label).iter().rev().copied().collect();
    while let Some(id) = stack.pop() {
        if doc.tag_name(id).is_some_and(|t| FORM_CONTROLS.contains(&t)) && focusability(doc, id) != Focusability::No {
            return Some(id);
        }
        stack.extend(doc.children(id).iter().rev());
    }
    None
}

/// Positive `tabindex` first (ascending, then document order), then the rest in document order.
/// `rendered` = elements that have a box (the others are unreachable by keyboard); `None` = no
/// render yet, keep every focusable element.
pub fn tab_order<C: RenderConfiguration>(doc: &EngineDocument<C>, rendered: Option<&HashSet<NodeId>>) -> Vec<NodeId> {
    let mut positive: Vec<(i32, usize, NodeId)> = Vec::new();
    let mut natural: Vec<NodeId> = Vec::new();
    let mut stack = vec![doc.root()];
    let mut seq = 0usize;
    while let Some(id) = stack.pop() {
        if rendered.is_none_or(|r| r.contains(&id)) {
            match focusability(doc, id) {
                Focusability::Sequential(0) => natural.push(id),
                Focusability::Sequential(n) => {
                    positive.push((n, seq, id));
                    seq += 1;
                }
                _ => {}
            }
        }
        stack.extend(doc.children(id).iter().rev());
    }
    positive.sort_by_key(|&(n, seq, _)| (n, seq));
    positive.into_iter().map(|(_, _, id)| id).chain(natural).collect()
}
