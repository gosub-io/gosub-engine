//! A deliberately tiny selector matcher.
//!
//! `querySelector` shows up in ~370 of the WPT forms tests, almost always with a single
//! compound selector. Anything more than that throws rather than silently mismatching - a
//! test that fails loudly is worth more than one that passes for the wrong reason.

use cow_utils::CowUtils;
use gosub_interface::document::Document as _;
use gosub_shared::node::NodeId;

use crate::Doc;

pub struct Compound {
    tag: Option<String>,
    /// Every `#id` in the compound, not just the last. `#a#b` is legal CSS that simply never
    /// matches, since an element has one id - keeping only the last would make it match `#b`.
    ids: Vec<String>,
    classes: Vec<String>,
}

pub fn parse(selector: &str) -> Result<Compound, String> {
    let selector = selector.trim();
    if selector.is_empty() || selector.contains([' ', '>', '+', '~', ',', '[', ':', '*']) {
        return Err(format!("unsupported selector: {selector:?}"));
    }

    let mut compound = Compound {
        tag: None,
        ids: Vec::new(),
        classes: Vec::new(),
    };
    let mut rest = selector;

    let tag_end = rest.find(['#', '.']).unwrap_or(rest.len());
    if tag_end > 0 {
        compound.tag = Some(rest[..tag_end].cow_to_ascii_lowercase().into_owned());
    }
    rest = &rest[tag_end..];

    while let Some(kind) = rest.chars().next() {
        let body = &rest[1..];
        let end = body.find(['#', '.']).unwrap_or(body.len());
        let name = &body[..end];
        if name.is_empty() {
            return Err(format!("unsupported selector: {selector:?}"));
        }
        match kind {
            '#' => compound.ids.push(name.to_string()),
            _ => compound.classes.push(name.to_string()),
        }
        rest = &body[end..];
    }
    Ok(compound)
}

pub fn matches(doc: &Doc, id: NodeId, compound: &Compound) -> bool {
    let Some(tag) = doc.tag_name(id) else {
        return false;
    };
    if compound
        .tag
        .as_ref()
        .is_some_and(|want| !tag.eq_ignore_ascii_case(want))
    {
        return false;
    }
    if !compound.ids.is_empty() {
        let have = doc.attribute(id, "id");
        if !compound.ids.iter().all(|want| have == Some(want)) {
            return false;
        }
    }
    compound.classes.iter().all(|class| doc.has_class(id, class))
}

/// Depth-first walk over the element descendants of `root`, in tree order.
pub fn descendants(doc: &Doc, root: NodeId) -> Vec<NodeId> {
    let mut out = Vec::new();
    let mut stack: Vec<NodeId> = doc.children(root).iter().rev().copied().collect();
    while let Some(id) = stack.pop() {
        stack.extend(doc.children(id).iter().rev());
        if doc.tag_name(id).is_some() {
            out.push(id);
        }
    }
    out
}
