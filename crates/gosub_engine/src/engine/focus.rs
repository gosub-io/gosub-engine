//! Keyboard focus: which element receives key input and shows the focus ring.
//!
//! Focus lives on the DOM document (so the `:focus` selector family can read it during style
//! resolution, like `:hover`); this module decides *what* is focusable and *where* focus moves on
//! a click or a Tab press. A focus change is rare, so it triggers a full re-render rather than the
//! paint-only hover fast path - that also keeps `:focus` rules that touch layout correct.

use crate::html::{EngineDocument, RenderConfiguration};
use cow_utils::CowUtils;
use gosub_interface::document::Document as _;
use gosub_interface::node::NodeType;
use gosub_shared::node::NodeId;
use std::collections::HashSet;

/// Whether, and how, an element can take focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focusability {
    No,
    /// Focusable by click/script only (`tabindex="-1"`), skipped by Tab.
    ClickOnly,
    /// In the sequential focus order; the value is `tabindex` (0 = document order).
    Sequential(i32),
}

const FORM_CONTROLS: [&str; 4] = ["input", "textarea", "select", "button"];

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
        _ => doc.attribute(id, "contenteditable").is_some(),
    };
    if natural {
        Focusability::Sequential(0)
    } else {
        Focusability::No
    }
}

/// Whether a *pointer* click on this element should show the focus ring (`:focus-visible`).
/// Browsers show it for text-entry controls, where the caret needs a visible home, and hide it
/// for buttons/checkboxes/links clicked with the mouse. Keyboard focus always shows it.
pub fn click_shows_ring<C: RenderConfiguration>(doc: &EngineDocument<C>, id: NodeId) -> bool {
    match doc.tag_name(id) {
        Some("textarea") | Some("select") => true,
        Some("input") => !doc.attribute(id, "type").is_some_and(|t| {
            matches!(
                t.cow_to_ascii_lowercase().as_ref(),
                "button" | "submit" | "reset" | "checkbox" | "radio" | "range" | "color" | "file" | "image"
            )
        }),
        _ => doc.attribute(id, "contenteditable").is_some(),
    }
}

/// The element a click on `leaf` gives focus to: the nearest focusable ancestor-or-self, or the
/// control a `<label>` on that path is bound to. `None` blurs.
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

/// The sequential focus order: positive `tabindex` values first (ascending, document order
/// within a value), then everything else in document order. `rendered` filters out elements
/// that have no box (display:none and friends), which are unreachable by keyboard; `None` (no
/// render exists yet, e.g. a Tab before the first frame) keeps every focusable element.
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
