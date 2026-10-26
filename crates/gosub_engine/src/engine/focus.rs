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

/// A top-level document's autofocus state (HTML, "autofocus processed flag" and "autofocus
/// candidates"), run by [`Autofocus::flush`] at every update of the rendering.
///
/// The candidates are the elements with `autofocus` that the insertion steps would have
/// appended while the flag was clear. No script inserts or moves elements here, so every
/// element is the parser's and was inserted before the first flush: they are taken in tree
/// order then, once. Nested documents are not modelled, so the document is always fully
/// active, its own top-level document and of its own origin.
#[derive(Debug, Default)]
pub struct Autofocus {
    processed: bool,
    /// `None` until the first flush gathers them.
    candidates: Option<std::collections::VecDeque<NodeId>>,
}

impl Autofocus {
    /// Whether a flush could still focus anything: the flag is clear and the candidates are
    /// not known to be empty. A cheap check before gathering what a flush needs.
    pub fn pending(&self) -> bool {
        !self.processed && self.candidates.as_ref().is_none_or(|c| !c.is_empty())
    }

    /// HTML "flush autofocus candidates": the element to run the focusing steps for, if
    /// any. `rendered` = elements that have a box (`None` = no render yet, see
    /// [`tab_order`]); `fragment` = the document URL's fragment, for its target element.
    pub fn flush<C: RenderConfiguration>(
        &mut self,
        doc: &EngineDocument<C>,
        rendered: Option<&HashSet<NodeId>>,
        fragment: Option<&str>,
    ) -> Option<NodeId> {
        // 1.
        if self.processed {
            return None;
        }
        // 2. and 3.
        let candidates = self.candidates.get_or_insert_with(|| autofocus_elements(doc).into());
        if candidates.is_empty() {
            return None;
        }
        // 4. The focused area is the document itself when nothing is focused.
        if doc.focused_node().is_some() || fragment.is_some_and(|f| indicated_element(doc, f).is_some()) {
            candidates.clear();
            self.processed = true;
            return None;
        }
        // 5. Steps 3 and 4 skip nothing in a lone top-level document, and step 5 never
        // returns: the engine has every stylesheet in before the first layout, so the
        // script-blocking style sheet set is empty by the time anything flushes. Step 8 finds
        // no target element, which step 4 already ruled out.
        while let Some(element) = candidates.pop_front() {
            // 9. and 10. An element that is not a focusable area has none to give: only a
            // navigable container or a shadow host would, and neither is modelled.
            let focusable =
                focusability(doc, element) != Focusability::No && rendered.is_none_or(|r| r.contains(&element));
            if focusable {
                // 11.
                candidates.clear();
                self.processed = true;
                // 12.
                return Some(element);
            }
        }
        None
    }
}

/// Every element with an `autofocus` attribute, in tree order.
fn autofocus_elements<C: RenderConfiguration>(doc: &EngineDocument<C>) -> Vec<NodeId> {
    let mut found = Vec::new();
    let mut stack = vec![doc.root()];
    while let Some(id) = stack.pop() {
        if doc.node_type(id) == NodeType::ElementNode && doc.attribute(id, "autofocus").is_some() {
            found.push(id);
        }
        stack.extend(doc.children(id).iter().rev());
    }
    found
}

/// The element a URL fragment indicates (HTML, "the indicated part of the document"), when
/// it is an element and not the top of the document or nothing: what becomes the
/// document's target element.
pub fn indicated_element<C: RenderConfiguration>(doc: &EngineDocument<C>, fragment: &str) -> Option<NodeId> {
    // 1. An empty fragment is the top of the document.
    if fragment.is_empty() {
        return None;
    }
    // 2. to 4. The fragment as it is, then percent-decoded. Step 5's `top` is the top of the
    // document, which no element is.
    potential_indicated_element(doc, fragment).or_else(|| {
        let decoded = percent_encoding::percent_decode_str(fragment).decode_utf8_lossy();
        potential_indicated_element(doc, &decoded)
    })
}

/// HTML "find a potential indicated element": the element whose ID is `fragment`, else the
/// first `a` in tree order whose `name` is.
fn potential_indicated_element<C: RenderConfiguration>(doc: &EngineDocument<C>, fragment: &str) -> Option<NodeId> {
    let mut by_id = None;
    let mut by_name = None;
    let mut stack = vec![doc.root()];
    while let Some(id) = stack.pop() {
        if doc.node_type(id) == NodeType::ElementNode {
            if doc.attribute(id, "id") == Some(fragment) {
                by_id = Some(id);
                break;
            }
            if by_name.is_none() && doc.tag_name(id) == Some("a") && doc.attribute(id, "name") == Some(fragment) {
                by_name = Some(id);
            }
        }
        stack.extend(doc.children(id).iter().rev());
    }
    by_id.or(by_name)
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
