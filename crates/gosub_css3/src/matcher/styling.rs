use core::fmt::Debug;
use cow_utils::CowUtils;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt::Display;
use std::sync::{Arc, OnceLock};

use gosub_interface::config::HasDocument;
use gosub_interface::css3;
use gosub_interface::css3::{CssOrigin, CssPropertyMap};
use gosub_interface::document::Document;
use gosub_interface::node::NodeType;
use gosub_interface::style::ComputedStyle;
use gosub_shared::node::NodeId;

use crate::colors::{CssColor, RgbColor};
use crate::functions::calc;
use crate::matcher::bloom::AncestorFilter;
use crate::matcher::property_definitions::get_css_definitions;
use crate::matcher::property_ids::{LonghandId, PropertyId, ShorthandId, PROPERTY_COUNT};
use crate::property_registry::PropertyRegistry;
use crate::stylesheet::{Combinator, CssSelector, CssSelectorPart, CssValue, MatcherType, NthSelector, Specificity};
use crate::system::Css3System;
use crate::tokenizer::NumberKind;

// Matches a complete selector (all parts) against the given node(id).
//
// `pseudo` selects what we are matching against:
//   * `None`           - match the element itself. Any selector containing a `::pseudo-element`
//                        part never matches (pseudo-elements are not the element).
//   * `Some("before")` - match the `::before` pseudo-element of `node_id`. Only selectors that
//                        explicitly carry the matching `::before` part match; the rest of the
//                        compound is matched against the originating element as usual.
//
// `ancestors` summarises what the elements above `node_id` carry, so that a complex selector
// asking for an ancestor nothing above this element has can be dropped without the walk that
// would discover the same thing. It decides nothing: a filter that says "maybe" leads to
// exactly the match that would have run anyway. `None` matches without it.
pub(crate) fn match_selector<C: HasDocument>(
    document: &C::Document,
    node_id: NodeId,
    selector: &CssSelector,
    pseudo: Option<&str>,
    scope: ScopeContext,
    ancestors: Option<&AncestorFilter>,
) -> (bool, Specificity) {
    // A selector list (`a, b`) matches with the highest specificity of its matching parts, and
    // the filter answers each of them separately - a list is commonly one selector that reaches
    // deep and several that do not.
    let mut best: Option<Specificity> = None;
    for (index, (part, specificity)) in selector.complex().enumerate() {
        // No ancestor carries something this selector requires of one, so the walk can only
        // fail. Cheapest test there is, so it goes first.
        if let Some(filter) = ancestors {
            if !filter.may_match(selector.ancestor_keys_at(index)) {
                continue;
            }
        }

        // When matching a pseudo-element, the selector must explicitly target it.
        if let Some(target) = pseudo {
            if !part
                .iter()
                .any(|p| matches!(p, CssSelectorPart::PseudoElement(n) if pseudo_eq(n, target)))
            {
                continue;
            }
        }

        // Which way this selector reaches has to agree with where the sheet sits relative to
        // the element. A shadow tree's plain rules must not touch the host or the light DOM
        // projected into it, and its `:host` / `::slotted()` rules must not touch anything else.
        if subject_reach(part) != scope.mode {
            continue;
        }

        if match_compound::<C>(document, node_id, part, pseudo, scope) {
            best = Some(best.map_or(specificity, |b| b.max(specificity)));
        }
    }

    best.map_or((false, Specificity::new(0, 0, 0)), |s| (true, s))
}

/// Where a stylesheet sits relative to the element being matched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScopeMatch {
    /// Same tree scope - or a user-agent sheet, which is not scoped at all. Ordinary matching.
    Same,
    /// The sheet belongs to the shadow tree that this element *hosts*, so only its `:host`
    /// rules reach here.
    Host,
    /// The sheet belongs to a shadow tree that this element is *projected into*, so only its
    /// `::slotted()` rules reach here.
    Slotted,
}

/// The stylesheet's position relative to the element, plus the shadow tree it belongs to.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ScopeContext {
    pub(crate) mode: ScopeMatch,
    /// The shadow root whose tree the sheet was parsed into, if any. `:host` needs it to find
    /// the host, `::slotted()` to find the slot.
    pub(crate) tree: Option<NodeId>,
}

impl ScopeContext {
    /// An unscoped context, for matching the inner selector of `:not()`, `:host()` or
    /// `::slotted()` - those arguments are plain compounds and cannot nest scope-crossing parts.
    pub(crate) fn plain() -> Self {
        Self {
            mode: ScopeMatch::Same,
            tree: None,
        }
    }
}

/// Which way a selector reaches, read from its *subject* - the rightmost compound, i.e. the
/// thing the rule actually styles. `:host p` styles `p` inside the tree; `:host(.a)` styles the
/// host; `::slotted(p)` styles a projected node.
fn subject_reach(parts: &[CssSelectorPart]) -> ScopeMatch {
    let subject = match parts.iter().rposition(|p| matches!(p, CssSelectorPart::Combinator(_))) {
        Some(i) => &parts[i + 1..],
        None => parts,
    };

    if subject.iter().any(|p| matches!(p, CssSelectorPart::Slotted(_))) {
        ScopeMatch::Slotted
    } else if subject.iter().any(|p| matches!(p, CssSelectorPart::Host(_))) {
        ScopeMatch::Host
    } else {
        ScopeMatch::Same
    }
}

/// Matches one compound-selector sequence, handling a leading `:host` before the ordinary
/// right-to-left walk takes over.
fn match_compound<C: HasDocument>(
    doc: &C::Document,
    node_id: NodeId,
    parts: &[CssSelectorPart],
    pseudo: Option<&str>,
    scope: ScopeContext,
) -> bool {
    // A leading `:host` is a condition on the tree's host, which lives *outside* the tree and so
    // is unreachable by the ancestor walk - a shadow root has no parent, which is exactly what
    // stops ordinary selectors crossing the boundary. Check it here, then match the remainder
    // inside the tree as usual.
    let Some(CssSelectorPart::Host(inner)) = parts.first() else {
        return match_selector_parts::<C>(doc, node_id, parts, pseudo, scope);
    };

    let Some(tree) = scope.tree else {
        return false;
    };
    let Some(host) = doc.shadow_host(tree) else {
        return false;
    };
    if let Some(inner) = inner {
        if !inner
            .iter()
            .any(|compound| match_selector_parts::<C>(doc, host, compound, None, ScopeContext::plain()))
        {
            return false;
        }
    }

    let mut rest = &parts[1..];
    if rest.is_empty() {
        // `:host` on its own: the host is the subject.
        return node_id == host;
    }

    // `:host > x` means x is a top-level node of the shadow tree, since the host is x's parent
    // in the flattened tree. `:host x` puts no such constraint on it.
    if let Some(CssSelectorPart::Combinator(combinator)) = rest.first() {
        let direct_child = matches!(combinator, Combinator::Child);
        rest = &rest[1..];
        if direct_child && doc.parent(node_id) != Some(tree) {
            return false;
        }
    }

    match_selector_parts::<C>(doc, node_id, rest, pseudo, scope)
}

/// The slot in `tree` that `node` is projected into, mirroring the flat tree's assignment: an
/// element goes to the slot named by its `slot` attribute, anything else to the first unnamed
/// slot, first in tree order winning.
///
/// Only needed so a `slot[name=x]::slotted(y)` prefix has something to match against; the
/// render pipeline computes the authoritative assignment for layout.
fn assigned_slot<C: HasDocument>(doc: &C::Document, node: NodeId, tree: NodeId) -> Option<NodeId> {
    let wanted = match doc.node_type(node) {
        NodeType::ElementNode => doc.attribute(node, "slot").unwrap_or(""),
        NodeType::TextNode => "",
        _ => return None,
    };

    let mut stack: Vec<NodeId> = doc.children(tree).iter().rev().copied().collect();
    while let Some(current) = stack.pop() {
        stack.extend(doc.children(current).iter().rev().copied());

        if doc.tag_name(current) != Some("slot") {
            continue;
        }
        if doc.attribute(current, "name").unwrap_or("") == wanted {
            return Some(current);
        }
    }
    None
}

/// Case-insensitive compare of a pseudo-element name against a target (`before`/`after`).
fn pseudo_eq(name: &str, target: &str) -> bool {
    name.eq_ignore_ascii_case(target)
}

fn consume<'a, T>(this: &mut &'a [T]) -> Option<&'a T> {
    let last = this.last()?;

    if let Some(parts) = this.get(..this.len() - 1) {
        *this = parts;
    }

    Some(last)
}

/// Whether `node` sits at an `An+B` position among the siblings an `:nth-*()` counts: element
/// siblings only, of the node's own type for the `-of-type` pair, and matching `S` for `of S` -
/// which the node itself must match too (Selectors L4 §14.4).
fn match_nth<C: HasDocument>(doc: &C::Document, node: NodeId, nth: &NthSelector) -> bool {
    let Some(parent) = doc.parent(node) else {
        return false;
    };
    // An element's type is its local name and its namespace (Selectors L4 §5.1).
    let element_type = |id: NodeId| (doc.tag_name(id), doc.namespace(id));
    let own_type = element_type(node);
    let counts = |id: NodeId| {
        doc.node_type(id) == NodeType::ElementNode
            && (!nth.kind.of_type() || element_type(id) == own_type)
            && nth.of.as_ref().is_none_or(|of| {
                of.iter()
                    .any(|complex| match_selector_parts::<C>(doc, id, complex, None, ScopeContext::plain()))
            })
    };
    if !counts(node) {
        return false;
    }

    let siblings = doc.children(parent);
    let Some(position) = siblings.iter().position(|&id| id == node) else {
        return false;
    };
    let before = if nth.kind.from_end() {
        siblings[position + 1..].iter().filter(|&&id| counts(id)).count()
    } else {
        siblings[..position].iter().filter(|&&id| counts(id)).count()
    };
    i32::try_from(before + 1).is_ok_and(|index| nth.matches_index(index))
}

/// Whether a relative selector (`[combinator, compound, combinator, compound, ...]`) is satisfied
/// starting from `anchor`. Walked left to right, the way it reads: the combinator names the
/// candidates, each candidate matching the compound continues with the rest from there.
fn match_relative<C: HasDocument>(doc: &C::Document, anchor: NodeId, relative: &[CssSelectorPart]) -> bool {
    let Some((CssSelectorPart::Combinator(combinator), rest)) = relative.split_first() else {
        return false;
    };
    let end = rest
        .iter()
        .position(|p| matches!(p, CssSelectorPart::Combinator(_)))
        .unwrap_or(rest.len());
    let (compound, tail) = rest.split_at(end);
    if compound.is_empty() {
        return false;
    }

    let try_candidate = |candidate: NodeId| {
        doc.node_type(candidate) == NodeType::ElementNode
            && match_selector_parts::<C>(doc, candidate, compound, None, ScopeContext::plain())
            && (tail.is_empty() || match_relative::<C>(doc, candidate, tail))
    };

    match combinator {
        Combinator::Child => doc.children(anchor).iter().any(|&c| try_candidate(c)),
        Combinator::NextSibling | Combinator::SubsequentSibling => {
            let Some(parent) = doc.parent(anchor) else {
                return false;
            };
            let siblings = doc.children(parent);
            let Some(position) = siblings.iter().position(|&s| s == anchor) else {
                return false;
            };
            let mut later = siblings[position + 1..]
                .iter()
                .copied()
                .filter(|&s| doc.node_type(s) == NodeType::ElementNode);
            if matches!(combinator, Combinator::NextSibling) {
                later.next().is_some_and(try_candidate)
            } else {
                later.any(try_candidate)
            }
        }
        // Descendant, and the namespace and column combinators this engine has no tree for:
        // search the subtree, depth first.
        _ => {
            let mut stack: Vec<NodeId> = doc.children(anchor).iter().rev().copied().collect();
            while let Some(node) = stack.pop() {
                if try_candidate(node) {
                    return true;
                }
                stack.extend(doc.children(node).iter().rev().copied());
            }
            false
        }
    }
}

thread_local! {
    /// Within one element's cascade: the node last asked about and its content language (`None`
    /// when it has none). A page sheet can hold a `.x:lang(..)` rule per language, and bare
    /// `:lang(..)` rules are tried on every element; without this each walks the ancestor chain
    /// and looks the attribute up again.
    static LANG_MEMO: std::cell::RefCell<Option<(usize, NodeId, Option<String>)>> =
        const { std::cell::RefCell::new(None) };
    static LANG_MEMO_ON: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Turns the `:lang()` memo on for one element's cascade, starting empty; dropping it turns the
/// memo off and forgets it, so nothing remembered outlives a change to the document.
pub(crate) struct LangMemoScope(());

impl LangMemoScope {
    pub(crate) fn enter() -> Self {
        LANG_MEMO.with_borrow_mut(|memo| *memo = None);
        LANG_MEMO_ON.set(true);
        Self(())
    }
}

impl Drop for LangMemoScope {
    fn drop(&mut self) {
        LANG_MEMO_ON.set(false);
        LANG_MEMO.with_borrow_mut(|memo| *memo = None);
    }
}

/// Whether the element's content language matches any of `ranges`. The content language is
/// the `lang` attribute of the element or its nearest ancestor that has one (HTML §3.2.6.2);
/// with none it is unknown, which only the empty range `:lang("")` matches (Selectors L4 §7.2) -
/// the document-level fallbacks (`Content-Language`, `<meta http-equiv>`) are not consulted.
fn matches_lang<C: HasDocument>(doc: &C::Document, node: NodeId, ranges: &[String]) -> bool {
    let test = |lang: Option<&str>| {
        let lang = lang.unwrap_or("");
        ranges.iter().any(|r| language_range_matches(r, lang))
    };
    if !LANG_MEMO_ON.get() {
        return test(content_language::<C>(doc, node));
    }

    let doc_key = std::ptr::from_ref(doc).cast::<()>() as usize;
    LANG_MEMO.with_borrow_mut(|memo| {
        if !matches!(memo, Some((key, id, _)) if *key == doc_key && *id == node) {
            *memo = Some((doc_key, node, content_language::<C>(doc, node).map(str::to_owned)));
        }
        test(memo.as_ref().and_then(|(_, _, lang)| lang.as_deref()))
    })
}

fn content_language<C: HasDocument>(doc: &C::Document, node: NodeId) -> Option<&str> {
    lang_holder::<C>(doc, node)
        .and_then(|id| doc.attribute(id, "lang"))
        .map(str::trim)
}

/// The element itself or its nearest ancestor that carries a `lang` attribute. A shadow tree
/// takes its language from its host, so the walk goes on from a shadow root to the host.
fn lang_holder<C: HasDocument>(doc: &C::Document, node: NodeId) -> Option<NodeId> {
    let mut current = Some(node);
    while let Some(id) = current {
        if doc.node_type(id) == NodeType::ElementNode && doc.attribute(id, "lang").is_some() {
            return Some(id);
        }
        current = doc.parent(id).or_else(|| doc.shadow_host(id));
    }
    None
}

/// RFC 4647 §3.3.2 extended filtering, the matching `:lang()` uses (Selectors L4 §7.2): subtags
/// compare case-insensitively, `*` matches any one, a range subtag may skip over tag subtags
/// but never past a singleton (`x`, `u`, ...), and `en` therefore matches `en-US`. An empty
/// range matches only an element whose language is the empty string.
fn language_range_matches(range: &str, tag: &str) -> bool {
    if range.is_empty() || tag.is_empty() {
        return range.is_empty() && tag.is_empty();
    }

    let mut range = range.split('-');
    let mut tag = tag.split('-');
    let (Some(first_range), Some(first_tag)) = (range.next(), tag.next()) else {
        return false;
    };
    if first_range != "*" && !first_range.eq_ignore_ascii_case(first_tag) {
        return false;
    }

    for subtag in range {
        if subtag == "*" {
            continue;
        }
        loop {
            let Some(candidate) = tag.next() else {
                return false;
            };
            if candidate.eq_ignore_ascii_case(subtag) {
                break;
            }
            if candidate.len() == 1 {
                return false;
            }
        }
    }
    true
}

/// Returns true when the given node matches the part(s)
fn match_selector_parts<C: HasDocument>(
    doc: &C::Document,
    node_id: NodeId,
    mut parts: &[CssSelectorPart],
    pseudo: Option<&str>,
    scope: ScopeContext,
) -> bool {
    let mut next_current_id: Option<NodeId> = Some(node_id);

    while let Some(part) = consume(&mut parts) {
        let Some(current_id) = next_current_id else {
            return false;
        };

        if doc.parent(current_id).is_none() {
            return false;
        }

        if !match_selector_part::<C>(part, current_id, doc, &mut next_current_id, &mut parts, pseudo, scope) {
            return false;
        }
    }

    true
}

#[allow(clippy::too_many_arguments)] // one more than clippy's default, and every one is load-bearing
fn match_selector_part<C: HasDocument>(
    part: &CssSelectorPart,
    current_id: NodeId,
    doc: &C::Document,
    next_id: &mut Option<NodeId>,
    parts: &mut &[CssSelectorPart],
    pseudo: Option<&str>,
    scope: ScopeContext,
) -> bool {
    match part {
        CssSelectorPart::Universal => true,
        // `:not()` matches when none of its arguments do. Each argument is matched against this
        // same element, so the negation is evaluated where it is written rather than walking the
        // tree - `:not()` takes a compound, and a compound never crosses a combinator.
        CssSelectorPart::Not(inner) => !inner
            .iter()
            .any(|compound| match_selector_parts::<C>(doc, current_id, compound, pseudo, ScopeContext::plain())),
        CssSelectorPart::Nth(nth) => match_nth::<C>(doc, current_id, nth),
        // `:is()` and `:where()` match when any argument does. Unlike `:not()`'s compounds, an
        // argument may be a complex selector (`:is(nav a)`); its rightmost compound is matched
        // against this element and its combinators walk the document from here, as the
        // argument would on its own (Selectors L4 §4.2).
        CssSelectorPart::Is(inner) | CssSelectorPart::Where(inner) => inner
            .iter()
            .any(|complex| match_selector_parts::<C>(doc, current_id, complex, pseudo, ScopeContext::plain())),
        // `:has()` matches when some element, related to this one as its relative selector says,
        // matches it - a descendant, a child, the next sibling or a later one.
        //
        // The search is the expensive part of any selector, and a compound is matched right to
        // left, so in `.zone:has(.x)` it would run before `.zone` on every element tried. The
        // rest of this compound goes first instead: it is a conjunction, so asking it early
        // changes nothing but the cost.
        CssSelectorPart::Has(inner) => {
            let start = parts
                .iter()
                .rposition(|p| matches!(p, CssSelectorPart::Combinator(_)))
                .map_or(0, |i| i + 1);
            let others = &parts[start..];
            if !others.is_empty()
                && !others.iter().any(|p| matches!(p, CssSelectorPart::Has(_)))
                && !match_selector_parts::<C>(doc, current_id, others, pseudo, scope)
            {
                return false;
            }
            inner
                .iter()
                .any(|relative| match_relative::<C>(doc, current_id, relative))
        }
        CssSelectorPart::Lang(ranges) => matches_lang::<C>(doc, current_id, ranges),
        // `:host` is only meaningful as the leftmost part, where `match_compound` handles it.
        CssSelectorPart::Host(_) => false,
        CssSelectorPart::Slotted(inner) => {
            if scope.mode != ScopeMatch::Slotted {
                return false;
            }
            let Some(tree) = scope.tree else {
                return false;
            };
            // `::slotted()` matches the assigned node itself, never its descendants, so the
            // argument is matched against this element and nothing is walked.
            if !inner
                .iter()
                .any(|compound| match_selector_parts::<C>(doc, current_id, compound, None, ScopeContext::plain()))
            {
                return false;
            }
            // A `slot[name=x]` prefix selects the slot the node landed in, so the walk
            // continues there rather than up the DOM.
            *next_id = assigned_slot::<C>(doc, current_id, tree);
            true
        }
        CssSelectorPart::Type(name) => {
            doc.node_type(current_id) == NodeType::ElementNode && doc.tag_name(current_id).is_some_and(|t| t == name)
        }
        CssSelectorPart::Class(name) => doc.has_class(current_id, name),
        CssSelectorPart::Id(name) => {
            doc.node_type(current_id) == NodeType::ElementNode
                && doc.attribute(current_id, "id").is_some_and(|v| v == name)
        }
        CssSelectorPart::Attribute(attr) => {
            if doc.node_type(current_id) != NodeType::ElementNode {
                return false;
            }

            let Some(got_raw) = doc.attribute(current_id, &attr.name) else {
                return false;
            };

            // Two buffers so we don't allocate when matching case-sensitive
            let mut _wanted_buf = String::new();
            let mut _got_buf = String::new();

            let (wanted_attr_value, got_attr_value): (&str, &str) = if attr.case_insensitive {
                _wanted_buf = attr.value.cow_to_lowercase().to_string();
                _got_buf = got_raw.cow_to_lowercase().to_string();
                (&_wanted_buf, &_got_buf)
            } else {
                (&attr.value, got_raw)
            };

            match attr.matcher {
                MatcherType::None => true,
                MatcherType::Equals => wanted_attr_value == got_attr_value,
                // One of the attribute's words is exactly the value; a value with whitespace in
                // it can never be one word, so it matches nothing (selectors-4 §6.1).
                MatcherType::Includes => {
                    !wanted_attr_value.contains(|c: char| c.is_ascii_whitespace())
                        && got_attr_value.split_ascii_whitespace().any(|s| s == wanted_attr_value)
                }
                MatcherType::DashMatch => {
                    got_attr_value == wanted_attr_value || got_attr_value.starts_with(&format!("{wanted_attr_value}-"))
                }
                // An empty value matches nothing for the substring matchers (selectors-4 §6.2).
                MatcherType::PrefixMatch => {
                    !wanted_attr_value.is_empty() && got_attr_value.starts_with(wanted_attr_value)
                }
                MatcherType::SuffixMatch => {
                    !wanted_attr_value.is_empty() && got_attr_value.ends_with(wanted_attr_value)
                }
                MatcherType::SubstringMatch => {
                    !wanted_attr_value.is_empty() && got_attr_value.contains(wanted_attr_value)
                }
            }
        }
        CssSelectorPart::PseudoClass(name) => match name.as_ref() {
            "hover" => doc.is_hovered(current_id),
            // Link pseudo-classes: match any element with an href attribute.
            // We have no browsing history, so treat everything as unvisited
            // (`:link` matches, `:visited` does not).
            "link" | "any-link" | "-webkit-any-link" => {
                doc.node_type(current_id) == NodeType::ElementNode
                    && doc
                        .tag_name(current_id)
                        .is_some_and(|t| matches!(t, "a" | "area" | "link"))
                    && doc.attribute(current_id, "href").is_some()
            }
            "visited" => false,
            // Structural pseudo-classes count element siblings only (selectors-4 §14): the
            // whitespace text before the first tag of a pretty-printed list is not a child that
            // `:first-child` should lose to.
            "first-child" => {
                if let Some(parent_id) = doc.parent(current_id) {
                    doc.children(parent_id)
                        .iter()
                        .find(|&&id| doc.node_type(id) == NodeType::ElementNode)
                        .is_some_and(|&id| id == current_id)
                } else {
                    false
                }
            }
            "last-child" => {
                if let Some(parent_id) = doc.parent(current_id) {
                    doc.children(parent_id)
                        .iter()
                        .rfind(|&&id| doc.node_type(id) == NodeType::ElementNode)
                        .is_some_and(|&id| id == current_id)
                } else {
                    false
                }
            }
            "first-of-type" => {
                let tag = doc.tag_name(current_id);
                if let (Some(parent_id), Some(tag)) = (doc.parent(current_id), tag) {
                    doc.children(parent_id)
                        .iter()
                        .find(|&&id| doc.tag_name(id) == Some(tag))
                        .is_some_and(|&id| id == current_id)
                } else {
                    false
                }
            }
            "last-of-type" => {
                let tag = doc.tag_name(current_id);
                if let (Some(parent_id), Some(tag)) = (doc.parent(current_id), tag) {
                    doc.children(parent_id)
                        .iter()
                        .filter(|&&id| doc.tag_name(id) == Some(tag))
                        .last()
                        .is_some_and(|&id| id == current_id)
                } else {
                    false
                }
            }
            "only-child" => {
                if let Some(parent_id) = doc.parent(current_id) {
                    let elem_siblings: Vec<_> = doc
                        .children(parent_id)
                        .iter()
                        .filter(|&&id| doc.node_type(id) == NodeType::ElementNode)
                        .copied()
                        .collect();
                    elem_siblings.len() == 1 && elem_siblings[0] == current_id
                } else {
                    false
                }
            }
            "only-of-type" => {
                let tag = doc.tag_name(current_id);
                if let (Some(parent_id), Some(tag)) = (doc.parent(current_id), tag) {
                    doc.children(parent_id)
                        .iter()
                        .filter(|&&id| doc.tag_name(id) == Some(tag))
                        .count()
                        == 1
                } else {
                    false
                }
            }
            // The document's root element (`<html>`): an element whose parent is absent or is
            // the Document node. Checking `parent().is_none()` alone fails because `<html>`'s
            // parent is the Document node, so `:root` would match nothing and
            // `:root { --custom: … }` custom properties would never be collected.
            //
            // The parent must be the Document *specifically*, not merely a non-element: a
            // shadow tree's top-level elements hang off a shadow root, which is not an element
            // either, and a shadow tree has no root element at all for `:root` to select.
            "root" => {
                doc.node_type(current_id) == NodeType::ElementNode
                    && doc
                        .parent(current_id)
                        .is_none_or(|p| doc.node_type(p) == NodeType::DocumentNode)
            }
            "checked" => doc.is_checked(current_id),
            "disabled" => doc.attribute(current_id, "disabled").is_some(),
            "enabled" => {
                doc.attribute(current_id, "disabled").is_none() && doc.node_type(current_id) == NodeType::ElementNode
            }
            "read-only" => doc.attribute(current_id, "readonly").is_some(),
            "read-write" => {
                doc.attribute(current_id, "readonly").is_none()
                    && doc.attribute(current_id, "disabled").is_none()
                    && doc.node_type(current_id) == NodeType::ElementNode
            }
            "focus" => doc.is_focused(current_id),
            "focus-visible" => doc.is_focus_visible(current_id),
            "focus-within" => doc.is_focus_within(current_id),
            "active" => false,
            // Unknown / unimplemented pseudo-classes never match.
            _ => false,
        },
        // A pseudo-element part matches only when we are explicitly computing the styles for that
        // pseudo-element (`pseudo == Some(name)`). It does not advance `next_id`: the remaining
        // compound continues to match against the originating element.
        CssSelectorPart::PseudoElement(name) => pseudo.is_some_and(|target| pseudo_eq(name, target)),
        CssSelectorPart::Combinator(combinator) => match combinator {
            Combinator::Descendant => {
                // Every ancestor is a candidate, and the *whole* rest of the selector has to
                // match from it - not merely the one part to the left of this combinator.
                //
                // Testing a single part and committing to the first ancestor that matched it
                // made `.a > .b .c` miss
                // `<div class=a><div class=b><div class=b><p class=c>`: from the `.c` it found
                // the inner `.b`, committed, then required *that* one's parent to be `.a`.
                // It is not, so matching gave up rather than climbing to the outer `.b` - which
                // does satisfy the selector. Recursing on the remainder is what lets it climb.
                let rest = *parts;
                if rest.is_empty() {
                    // A selector that begins with a combinator. Nothing to match to the left of
                    // it, and an empty remainder trivially "matches", so refuse it explicitly.
                    return false;
                }
                // This arm consumes the remainder itself, so the caller's loop has nothing left
                // to walk and stops on whatever we answer.
                *parts = &[];
                *next_id = None;

                let mut ancestor = doc.parent(current_id);
                while let Some(id) = ancestor {
                    // Only an element can match a selector; the document node at the top of
                    // the chain must not satisfy `*`.
                    if doc.node_type(id) == NodeType::ElementNode
                        && match_selector_parts::<C>(doc, id, rest, pseudo, scope)
                    {
                        return true;
                    }
                    ancestor = doc.parent(id);
                }
                false
            }
            Combinator::Child => {
                let Some(parent_id) = doc.parent(current_id) else {
                    return false;
                };

                let Some(last) = consume(parts) else {
                    return false;
                };

                *next_id = Some(parent_id);

                match_selector_part::<C>(last, parent_id, doc, next_id, parts, pseudo, scope)
            }
            Combinator::NextSibling => {
                let Some(parent_id) = doc.parent(current_id) else {
                    return false;
                };

                let children = doc.children(parent_id);

                let Some(my_index) = children.iter().position(|&c| c == current_id) else {
                    return false;
                };

                // Previous *element* sibling: whitespace text between tags doesn't count.
                let Some(&prev_id) = children[..my_index]
                    .iter()
                    .rev()
                    .find(|&&c| doc.node_type(c) == NodeType::ElementNode)
                else {
                    return false;
                };

                let Some(last) = consume(parts) else {
                    return false;
                };

                *next_id = Some(prev_id);

                match_selector_part::<C>(last, prev_id, doc, next_id, parts, pseudo, scope)
            }
            Combinator::SubsequentSibling => {
                // Same as the descendant case, over preceding siblings rather than ancestors:
                // several of them may match the part on the left, and only some of those may
                // satisfy what lies further left still.
                let Some(parent_id) = doc.parent(current_id) else {
                    return false;
                };

                let children: Vec<NodeId> = doc.children(parent_id).to_vec();

                let rest = *parts;
                if rest.is_empty() {
                    return false;
                }
                *parts = &[];
                *next_id = None;

                for child_id in children {
                    if child_id == current_id {
                        break;
                    }
                    // Text and comment siblings are not candidates: `*` matches an element, and
                    // `* ~ .target` must not be satisfied by the whitespace before `.target`.
                    if doc.node_type(child_id) != NodeType::ElementNode {
                        continue;
                    }

                    if match_selector_parts::<C>(doc, child_id, rest, pseudo, scope) {
                        return true;
                    }
                }

                false
            }
            Combinator::Namespace => {
                let Some(namespace) = consume(parts) else {
                    return false;
                };

                if *namespace == CssSelectorPart::Universal {
                    return true;
                }

                let CssSelectorPart::Type(namespace) = namespace else {
                    return false;
                };

                doc.namespace(current_id).is_some_and(|ns| ns == namespace)
            }
            Combinator::Column => false,
        },
    }
}

/// A declarationProperty defines a single value for a property (color: red;). It consists of the value,
/// origin, importance, location and specificity of the declaration.
#[derive(Debug, Clone)]
pub struct DeclarationProperty {
    /// The declared value, shared with the stylesheet rule it came from rather than copied out
    /// of it - see [`crate::stylesheet::CssDeclaration::value`].
    pub value: Arc<CssValue>,
    /// Origin of the declaration (user stylesheet, author stylesheet etc.)
    pub origin: CssOrigin,
    /// Whether the declaration is !important
    pub important: bool,
    // @TODO: location should be a Location
    /// The location of the declaration in the stylesheet (name.css:123) or empty.
    ///
    /// Shared with the stylesheet rather than copied: every declaration of every element used to
    /// clone the sheet's URL, which on a page of a few thousand elements is a few hundred
    /// thousand string allocations that nothing ever reads apart from a debugger.
    pub location: Arc<str>,
    /// The specificity of the selector that declared this property
    pub specificity: Specificity,
    /// How many shadow boundaries deep the declaring stylesheet sits: 0 for the document,
    /// 1 for a sheet in a shadow tree hosted by a document element, and so on. Feeds the
    /// cross-tree half of the cascade; see [`DeclarationProperty::tree_rank`].
    pub shadow_depth: u16,
    /// The cascade layer this declaration came from, as its rank within its origin: higher
    /// means declared later. `None` for a declaration outside every layer.
    ///
    /// css-cascade-5 §6.4.1 sorts layers after the tree and before specificity, so a layer
    /// settles the winner while the selectors are still unread. A normal declaration is
    /// strongest when it sits in no layer at all and, failing that, in the latest one. For an
    /// important declaration the whole order turns round: the earliest layer wins and unlayered
    /// is weakest, which is what lets a reset layer keep an `!important` the page cannot undo.
    pub layer: Option<u32>,
    /// Whether the declaration came from the element's own `style` attribute.
    ///
    /// Element-attached styles are their own step of the cascade, above layers and specificity
    /// both (css-cascade-5 §6.3). Ranking them by specificity alone was enough until layers
    /// existed, because nothing else could reach that high; a rule in a late layer can.
    pub attached: bool,
    /// Position of the declaration in document order, counted across every matched rule.
    /// The last step of the cascade: when origin, tree and specificity all tie, the
    /// declaration that comes later in the stylesheets wins.
    ///
    /// A longhand produced by expanding a shorthand inherits the *shorthand's* position, so
    /// `margin: 4px` followed by `margin-left: 80px` resolves the way it reads. Without this,
    /// expansion order decided instead - and every expanded longhand is added after all the
    /// directly declared ones, so a shorthand silently beat every longhand it overlapped.
    pub order: u32,
}

/// Cascade rank of a declaration from its origin and importance, as defined in
/// <https://developer.mozilla.org/en-US/docs/Web/CSS/Cascade>: higher wins.
#[must_use]
pub fn cascade_rank(origin: CssOrigin, important: bool) -> u8 {
    match (origin, important) {
        (CssOrigin::UserAgent, true) => 7,
        (CssOrigin::User, true) => 6,
        (CssOrigin::Author, true) => 5,
        (CssOrigin::Author, false) => 3,
        (CssOrigin::User, false) => 2,
        (CssOrigin::UserAgent, false) => 1,
    }
}

impl DeclarationProperty {
    fn priority(&self) -> u8 {
        cascade_rank(self.origin, self.important)
    }

    /// The cross-tree tiebreak, applied after origin and importance but before specificity.
    ///
    /// CSS Scoping §3.3: when two declarations of the same origin and importance come from
    /// different trees, the *outer* one wins if they are normal and the *inner* one wins if
    /// they are important. So a normal declaration ranks higher the shallower it is, and an
    /// important one ranks higher the deeper it is. Declarations from the same tree tie here
    /// and fall through to specificity, as they always did.
    fn tree_rank(&self) -> u16 {
        if self.important {
            self.shadow_depth
        } else {
            u16::MAX - self.shadow_depth
        }
    }

    /// The cascade-layer step, as a number where higher wins (css-cascade-5 §6.4.1).
    ///
    /// Unlayered is the top of the order for a normal declaration and the bottom for an
    /// important one, and the layers themselves run in opposite directions for the two.
    fn layer_rank(&self) -> u32 {
        match (self.layer, self.important) {
            (None, false) => u32::MAX,
            (None, true) => 0,
            (Some(layer), false) => layer.saturating_add(1),
            (Some(layer), true) => u32::MAX.saturating_sub(layer).saturating_sub(1),
        }
    }
}

impl PartialEq<Self> for DeclarationProperty {
    fn eq(&self, other: &Self) -> bool {
        self.priority() == other.priority()
    }
}

impl PartialOrd<Self> for DeclarationProperty {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Eq for DeclarationProperty {}

impl Ord for DeclarationProperty {
    fn cmp(&self, other: &Self) -> Ordering {
        self.priority()
            .cmp(&other.priority())
            .then_with(|| self.tree_rank().cmp(&other.tree_rank()))
            .then_with(|| self.attached.cmp(&other.attached))
            .then_with(|| self.layer_rank().cmp(&other.layer_rank()))
            .then_with(|| self.specificity.cmp(&other.specificity))
            .then_with(|| self.order.cmp(&other.order))
    }
}

/// The CSS-wide keywords, which are valid for every property and mean something about the
/// cascade rather than about the property (css-cascade-4 §7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CssWide {
    Inherit,
    Initial,
    Unset,
    Revert,
    RevertLayer,
}

/// Which CSS-wide keyword `value` is, if any. The parser lowers all of them to a string, so the
/// dedicated variants are only what a caller that builds values directly produces.
#[must_use]
pub fn css_wide_keyword(value: &CssValue) -> Option<CssWide> {
    match value {
        CssValue::Inherit => Some(CssWide::Inherit),
        CssValue::Initial => Some(CssWide::Initial),
        CssValue::String(keyword) => {
            for (name, kind) in [
                ("inherit", CssWide::Inherit),
                ("initial", CssWide::Initial),
                ("unset", CssWide::Unset),
                ("revert", CssWide::Revert),
                ("revert-layer", CssWide::RevertLayer),
            ] {
                if keyword.eq_ignore_ascii_case(name) {
                    return Some(kind);
                }
            }
            None
        }
        _ => None,
    }
}

/// A value entry contains all values for a single property for a single node. It contains the declared values, and
/// all the computed values.
#[derive(Debug, Clone)]
pub struct CssProperty {
    /// Which property this is. `None` only for a property built straight from a value, which
    /// names nothing and so has no definition to consult - the same answer the name `unknown`
    /// used to get.
    pub id: Option<PropertyId>,
    /// True when this property needs to be recalculated
    pub dirty: bool,
    /// List of all declared values for this property
    pub declared: Vec<DeclarationProperty>,
    /// Computed value from the specified value: the last stage the style system settles.
    ///
    /// The used and actual values used to sit here too, as two more fields the chain copied
    /// into, and neither did anything. A used value needs a containing block and an actual
    /// value needs the device pixel grid; both belong to layout and paint, which have them.
    /// Nothing is rounded here in particular - snapping to the pixel grid is the renderer's
    /// job, and `0.14em` against a 20px font-size is exactly 2.8px, which is what a computed
    /// value has to report.
    pub computed: CssValue,
    pub inherited: CssValue,
    /// The px value an `em` in this property resolves against.
    ///
    /// For every property but `font-size` that is the element's *own* computed font-size; for
    /// `font-size` itself it is the parent's, since `font-size: 2em` doubles what it inherits
    /// rather than itself. Set by the cascade, which is the only place that knows either.
    pub font_size_basis: f32,
    /// The px value a `rem` in this property resolves against: the root element's computed
    /// `font-size`, the same for every property on every element in the document.
    ///
    /// The root's own `font-size` is the exception - it is what defines a `rem`, so `rem` inside
    /// it refers to the initial font-size instead of to the value being declared.
    pub root_font_size_basis: f32,
    /// The px value an `lh` in this property resolves against: the element's own computed
    /// `line-height`, or the parent's in `line-height` and `font-size` themselves, which is what
    /// stops `line-height: 2lh` referring to itself. `None` until the cascade knows it, when an
    /// `lh` falls back to a `normal` line box.
    pub line_height_basis: Option<f32>,
    /// The px value an `rlh` resolves against: the root element's computed `line-height`, with
    /// the same exception for the root's own `line-height` and `font-size` as a `rem` has.
    pub root_line_height_basis: Option<f32>,
}

/// The initial `font-size`, and so the `rem` basis until the root element declares otherwise.
pub const DEFAULT_FONT_SIZE_PX: f32 = 16.0;

/// How a `line-height` passes to a child that does not declare its own.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum InheritedLineHeight {
    /// A length, which the child inherits as it is.
    Px(f32),
    /// A unitless number or `normal`, which the child applies to its own font-size.
    FontSizeMultiple(f32),
}

/// The initial `line-height`, `normal`, at the initial font-size: the `lh` and `rlh` basis until
/// the root element declares otherwise.
#[expect(clippy::cast_possible_truncation, reason = "a line-height fits an f32")]
pub const DEFAULT_LINE_HEIGHT_PX: f32 = DEFAULT_FONT_SIZE_PX * calc::NORMAL_LINE_HEIGHT as f32;

/// Turn every `<line-width>` keyword in a value into the length it computes to. Recurses, because
/// `stroke-width` takes a list of them and `column-rule-width` puts them inside `repeat()`.
///
/// css-backgrounds-3 §4.1 leaves the sizes to the UA; these are what every browser uses.
/// `hairline` is one device pixel (css-borders-4), so on a 2x display it is 0.5px and no longer
/// the same as `thin`; but never more than 1px, which the spec puts first, so below 1x it stays
/// 1px rather than outgrowing `thin`. The ratio is the one media queries see, which style caches
/// already key on.
fn resolve_line_widths(value: CssValue) -> CssValue {
    match value {
        CssValue::String(keyword) => {
            let dpr = f64::from(crate::media_query::media_environment().device_pixel_ratio);
            let hairline = if dpr > 0.0 { (1.0 / dpr).min(1.0) } else { 1.0 };
            let px = [("hairline", hairline), ("thin", 1.0), ("medium", 3.0), ("thick", 5.0)]
                .into_iter()
                .find(|(name, _)| keyword.eq_ignore_ascii_case(name))
                .map(|(_, px)| px);
            match px {
                Some(px) => CssValue::Unit(px, "px".to_string()),
                None => CssValue::String(keyword),
            }
        }
        CssValue::List(items) => CssValue::List(items.into_iter().map(resolve_line_widths).collect()),
        CssValue::Function(name, args) => CssValue::Function(name, args.into_iter().map(resolve_line_widths).collect()),
        other => other,
    }
}

/// The px an absolute `font-size` keyword computes to.
///
/// The scale is the one every browser ships, `medium` at 16px. css-fonts-4 §2.5 offers its own
/// ratios only as guidelines, so this is not a deviation to correct.
fn absolute_font_size(keyword: &str) -> Option<f64> {
    let px = match keyword.cow_to_ascii_lowercase().as_ref() {
        "xx-small" => 9.0,
        "x-small" => 10.0,
        "small" => 13.0,
        "medium" => 16.0,
        "large" => 18.0,
        "x-large" => 24.0,
        "xx-large" => 32.0,
        "xxx-large" => 48.0,
        _ => return None,
    };
    Some(px)
}

/// The px a `font-size` keyword computes to, given the parent's size.
///
/// The absolute keywords are [`absolute_font_size`]. The relative ones step by the 1.2 the spec
/// suggests. `math` scales by `math-depth`, which this engine does not have, so it is the
/// parent's size.
fn font_size_keyword(keyword: &str, parent: f64) -> Option<f64> {
    if let Some(px) = absolute_font_size(keyword) {
        return Some(px);
    }
    let px = match keyword.cow_to_ascii_lowercase().as_ref() {
        "smaller" => parent / 1.2,
        "larger" => parent * 1.2,
        "math" => parent,
        _ => return None,
    };
    Some(px)
}

/// Turn a specified value into a computed one: resolve the relative lengths, do the arithmetic.
///
/// This is what makes a *computed* value computed. css-values says `em` and `rem` resolve at
/// computed-value time and that a math function is simplified there, so a consumer downstream
/// never sees either - `width: calc(2em + 10px)` on a 20px element leaves here as `50px`.
///
/// What survives is what genuinely cannot be decided yet: a percentage, which needs a containing
/// block, and the units nothing has a value for (`ch`, `lh`, the container-query units).
fn resolve_computed(value: CssValue, units: &calc::Units) -> CssValue {
    let recurse = |v: CssValue| resolve_computed(v, units);
    match value {
        // Every unit with a known conversion becomes the canonical one - px for a length, deg
        // for an angle, s for a time. That is what a computed value is: `margin: 12cm` computes
        // to `453.5433px`, the same as any expression that arrives at that length by another
        // route. Only `em` and `rem` needed an element to resolve against, and only they used to
        // be done here, so a bare `12cm` and a `round(10cm, 6cm)` that equals it disagreed.
        CssValue::Unit(val, unit) => {
            match calc::to_canonical(val, &unit, units) {
                Some((canonical, converted)) => CssValue::Unit(converted, canonical),
                // `ch`, `lh` and the container-query units have no value here, and a percentage
                // needs a containing block. They travel on as written.
                None => CssValue::Unit(val, unit),
            }
        }
        CssValue::List(values) => CssValue::List(values.into_iter().map(recurse).collect()),
        // A `calc()` body is arithmetic, not a list of arguments, so it is evaluated as a whole
        // rather than recursed into. A body that comes down to a single value *is* that value:
        // `getComputedStyle` reports `50px`, not `calc(50px)`, once nothing is left to decide.
        CssValue::Function(name, args) if name.eq_ignore_ascii_case("calc") => {
            match calc::evaluate(&args, units, true) {
                Some(reduced) => reduced,
                None => CssValue::Function(name, args),
            }
        }
        CssValue::Function(name, args) => {
            let args: Vec<CssValue> = args.into_iter().map(recurse).collect();
            // A math function is evaluated here rather than when the declaration was collected,
            // because only now is an `em` among its arguments worth anything. Parsing tries the
            // same thing with less to go on, and what it could not reduce lands here.
            if let Some(reduced) = calc::evaluate_call(&name, &args, units, true) {
                return reduced;
            }
            // What the evaluator cannot reduce here has a unit nothing can resolve yet - `ch`,
            // `lh`, a container unit - or a percentage. It stays as written. It used to fall to
            // `resolve_math`, which reduces through `unit_to_px`, and that treats an unknown
            // unit as px: `min(1ch, 2px)` came out as `1px`.
            CssValue::Function(name, args)
        }
        other => other,
    }
}

/// `font-size` is the one property whose `em` resolves against its parent rather than itself,
/// and the one whose percentage resolves before layout.
const FONT_SIZE: PropertyId = PropertyId::Longhand(LonghandId::FontSize);
/// `font` keeps a value of its own for `getComputedStyle`, whose size has to agree with the
/// `font-size` longhand.
const FONT: PropertyId = PropertyId::Shorthand(ShorthandId::Font);
/// `color` is the one property on which `currentcolor` means the inherited colour.
const COLOR: PropertyId = PropertyId::Longhand(LonghandId::Color);

impl CssProperty {
    #[must_use]
    pub fn new(id: PropertyId) -> Self {
        Self::with_id(Some(id))
    }

    /// The property `name` denotes, or `None` when this engine has no definition for it.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        PropertyId::from_name(name).map(Self::new)
    }

    /// The CSS name of this property, empty for one built straight from a value.
    #[must_use]
    pub fn name(&self) -> &'static str {
        self.id.map_or("", PropertyId::name)
    }

    fn with_id(id: Option<PropertyId>) -> Self {
        Self {
            id,
            dirty: true,
            declared: Vec::new(),
            computed: CssValue::None,
            inherited: CssValue::None,
            font_size_basis: DEFAULT_FONT_SIZE_PX,
            root_font_size_basis: DEFAULT_FONT_SIZE_PX,
            line_height_basis: None,
            root_line_height_basis: None,
        }
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub fn mark_clean(&mut self) {
        self.dirty = false;
    }

    /// Returns the computed value of the property. Will compute the value when needed
    pub fn compute_value(&mut self) -> &CssValue {
        if self.dirty {
            self.calculate_value();
            self.dirty = false;
        }

        &self.computed
    }

    fn calculate_value(&mut self) {
        // The cascaded and specified values are steps on the way to the computed one, not
        // answers anybody keeps: storing them cost two `CssValue`s on every declared property of
        // every element - 96 bytes each before whatever they own on the heap - to hold values
        // only this function and the style dump ever read. They are threaded through as locals,
        // and [`Self::cascaded_value`] and [`Self::specified_value`] recompute them for the dump.
        let cascaded = self.find_cascaded_value();
        let specified = self.find_specified_value(cascaded.as_deref());
        self.computed = self.find_computed_value(specified);
    }

    /// The cascaded value: the winner among the declarations that reached this property.
    ///
    /// Recomputed rather than stored. Only the style dump asks, and it asks once per property
    /// per run, where storing it cost every element on every page.
    #[must_use]
    pub fn cascaded_value(&self) -> Option<Arc<CssValue>> {
        self.find_cascaded_value()
    }

    /// The specified value: the cascaded value, or what the property falls back to.
    ///
    /// Recomputed, for the same reason as [`Self::cascaded_value`].
    #[must_use]
    pub fn specified_value(&self) -> CssValue {
        self.find_specified_value(self.find_cascaded_value().as_deref())
    }

    fn find_cascaded_value(&self) -> Option<Arc<CssValue>> {
        let winner = self.declared.iter().max()?;
        // `revert` is not a value: it says to take the value this property would have had if
        // the origin the winning declaration came from had said nothing at all (css-cascade-5
        // §7.2). `revert-layer` asks the narrower question, about the layer rather than the
        // whole origin. Either way the answer is the cascade run again over what is left.
        match css_wide_keyword(&winner.value) {
            Some(CssWide::Revert) => {
                let origin = winner.origin;
                self.declared
                    .iter()
                    .filter(|declaration| declaration.origin != origin)
                    .max()
                    .map(|declaration| Arc::clone(&declaration.value))
            }
            Some(CssWide::RevertLayer) => {
                let (origin, layer) = (winner.origin, winner.layer);
                self.declared
                    .iter()
                    .filter(|declaration| declaration.origin != origin || declaration.layer != layer)
                    .max()
                    .map(|declaration| Arc::clone(&declaration.value))
            }
            _ => Some(Arc::clone(&winner.value)),
        }
    }

    /// The specified value: the cascaded value, or what the property falls back to when nothing
    /// in the cascade set it (css-cascade-4 §4.3).
    ///
    /// This is where `inherit` and `unset` resolve. Both name the inherited value - `unset` only
    /// for a property that inherits, and the initial value otherwise - and `inherited` holds it
    /// when a parent map was passed in. A consumer that walks the tree itself sees the keyword
    /// travel on, and resolves it against the ancestor it has; what must not happen is for it to
    /// reach a value converter, which reads a keyword it does not know as "nothing declared".
    fn find_specified_value(&self, cascaded: Option<&CssValue>) -> CssValue {
        let Some(cascaded) = cascaded else {
            return self.inherited.clone();
        };
        match css_wide_keyword(cascaded) {
            Some(CssWide::Inherit) => self.inherited.clone(),
            // `unset` is `inherit` on a property that inherits and `initial` on one that does
            // not, which is the same thing as having no cascaded value at all.
            Some(CssWide::Unset) => {
                if self.property_inherits() {
                    self.inherited.clone()
                } else {
                    CssValue::Initial
                }
            }
            _ => cascaded.clone(),
        }
    }

    /// Whether this property inherits by default, which is what `unset` turns on.
    fn property_inherits(&self) -> bool {
        self.id.is_some_and(PropertyId::inherited)
    }

    fn find_computed_value(&self, specified: CssValue) -> CssValue {
        let specified = match &specified {
            // `initial` names the property's own initial value, whatever that is
            // (css-cascade §7.1), so it resolves here rather than travelling on as a keyword
            // nothing downstream recognises. It arrives as a string far more often than as the
            // dedicated variant, because that is what the parser lowers the CSS-wide keywords to.
            CssValue::Initial | CssValue::None => self.get_initial_value().unwrap_or(CssValue::None),
            CssValue::String(keyword) if keyword.eq_ignore_ascii_case("initial") => {
                self.get_initial_value().unwrap_or(CssValue::None)
            }
            specified => specified.clone(),
        };

        // The `font` shorthand's own value has to say the same size the `font-size` longhand
        // computes to: `font: xx-small serif` reads back as `9px serif`, as every browser has it.
        // A relative keyword is the size this element ended up with, which is the basis every
        // property but `font-size` itself is given.
        let specified = match specified {
            CssValue::List(mut items) if self.id == Some(FONT) => {
                let own = f64::from(self.font_size_basis);
                for item in &mut items {
                    if let CssValue::String(keyword) = item {
                        let size = absolute_font_size(keyword).or_else(|| font_size_keyword(keyword, own).map(|_| own));
                        if let Some(px) = size {
                            *item = CssValue::Unit(px, "px".to_string());
                            break;
                        }
                    }
                }
                CssValue::List(items)
            }
            other => other,
        };

        // A colour keyword computes to the colour it names (css-color-4 §15). It travels this
        // far as a plain keyword because that is what the *specified* value is - reading
        // `element.style.color` back after setting it to `red` has to answer `red` - and only
        // the computed value is the sRGB colour, which is what `getComputedStyle` reports and
        // what the painter needs. Whether the keyword is a colour at all depends on the
        // property: `red` names a grid line on `grid-row-start`.
        //
        // The whole value is walked, not just its top: the stops of a gradient are colours too,
        // and `linear-gradient(30deg, red, blue)` computes with each of them resolved.
        let specified = self.resolve_colors(specified);

        // The computed value of `font-size` is an absolute length (css-fonts-4 §3.5), so a
        // percentage resolves here rather than travelling on. It is the one percentage that can:
        // it is a fraction of the *parent's* font-size, which is exactly what `font_size_basis`
        // holds for this property, where every other percentage needs a containing block and has
        // to wait for layout.
        //
        // The keywords resolve here for the same reason. Left as keywords, a child inherited
        // `smaller` and applied it again, and an `em` on the element itself was measured against
        // its parent's size.
        if self.id == Some(FONT_SIZE) {
            let parent = f64::from(self.font_size_basis);
            match &specified {
                CssValue::Percentage(pct) => return CssValue::Unit(parent * pct / 100.0, "px".to_string()),
                CssValue::String(keyword) => {
                    if let Some(px) = font_size_keyword(keyword, parent) {
                        return CssValue::Unit(px, "px".to_string());
                    }
                }
                _ => {}
            }
        }

        // A `<line-width>` keyword computes to an absolute length. A keyword that reached the
        // consumer as a string measured as zero, so `border: solid red` drew no border at all.
        // Which properties take one is the grammar's to say: `thin` is a line width on
        // `column-gap` and a keyword of its own on `scrollbar-width`.
        let specified = if self.id.is_some_and(|id| get_css_definitions().takes_line_width(id)) {
            resolve_line_widths(specified)
        } else {
            specified
        };

        // Font-relative lengths become px here, which is what the computed stage is for. What
        // survives is what genuinely cannot be decided yet: a percentage, which needs a
        // containing block, and the units nothing has a value for.
        let mut units = calc::Units::computed(self.font_size_basis, self.root_font_size_basis);
        units.lh_px = self.line_height_basis;
        units.rlh_px = self.root_line_height_basis;
        let computed = resolve_computed(specified, &units);

        self.clamp_to_range(computed)
    }

    /// Resolve every colour in a value: a keyword to the colour it names, and a colour already
    /// parsed to its computed form. Recurses, because a colour can sit inside a function.
    ///
    /// Takes the value rather than borrowing it: most of a stylesheet is not a colour, and a
    /// value this leaves alone is handed straight back instead of being cloned to say so.
    fn resolve_colors(&self, value: CssValue) -> CssValue {
        match value {
            CssValue::String(keyword) => match self.color_keyword(&keyword) {
                Some(mut color) => {
                    color.computed = true;
                    CssValue::Color(color)
                }
                None => CssValue::String(keyword),
            },
            CssValue::Color(mut color) => {
                color.computed = true;
                CssValue::Color(color)
            }
            CssValue::Function(name, args) => {
                // A colour function still standing is folded here, where its `calc()` can be.
                if let Some(mut color) =
                    crate::stylesheet::fold_color_function(&name, &args, crate::stylesheet::ColorStage::Computed)
                {
                    color.computed = true;
                    return CssValue::Color(color);
                }
                // Resolving the arguments can turn one into a colour - `currentcolor` on `color`
                // - after which the function folds where it could not before.
                let args: Vec<CssValue> = args.into_iter().map(|a| self.resolve_colors(a)).collect();
                if let Some(mut color) =
                    crate::stylesheet::fold_color_function(&name, &args, crate::stylesheet::ColorStage::Computed)
                {
                    color.computed = true;
                    return CssValue::Color(color);
                }
                CssValue::Function(name, args)
            }
            CssValue::List(items) => CssValue::List(items.into_iter().map(|item| self.resolve_colors(item)).collect()),
            other => other,
        }
    }

    /// The colour a keyword names, when this property is one that takes a colour.
    ///
    /// `currentcolor` is not a colour of its own: it stands for the element's own `color`, which
    /// on `color` itself means the inherited one (css-color-4 §6.2). Every other property that
    /// mentions it resolves against this element's computed `color`, which this cannot see, so
    /// there the keyword travels on untouched.
    fn color_keyword(&self, keyword: &str) -> Option<CssColor> {
        let id = self.id?;
        if !get_css_definitions().takes_color(id) {
            return None;
        }
        if keyword.eq_ignore_ascii_case("currentcolor") {
            if id != COLOR {
                return None;
            }
            return match &self.inherited {
                CssValue::Color(inherited) => Some(*inherited),
                // Nothing above declared a colour, so `currentcolor` is the initial one.
                _ => RgbColor::try_from_str("black").map(CssColor::from),
            };
        }
        RgbColor::try_from_str(keyword).map(CssColor::from)
    }

    /// Bring a computed value inside the range its property allows (css-values-4 §10.12).
    ///
    /// This is what a property's `[0,∞]` does for a math function. The matcher cannot apply it
    /// when the declaration is parsed - `width: calc(-5px)` is a valid declaration whose result
    /// is not known yet - so the range waits here and clamps the answer instead of throwing the
    /// declaration away. `width: -5px` is still rejected outright, because a literal *is* known
    /// when it is parsed.
    ///
    /// Clamping is not conditional on the value having come from a math function. It does not
    /// need to be: a literal that the range would have caught never reaches here, and for the
    /// properties whose range comes from their computed-value line rather than their grammar
    /// (`opacity` and its kin) the clamp is meant to apply to every value - `opacity: 1.5`
    /// computes to `1`.
    fn clamp_to_range(&self, computed: CssValue) -> CssValue {
        let defs = get_css_definitions();
        let Some(id) = self.id else {
            return computed;
        };
        if defs.definition(id).is_none() {
            return computed;
        }
        // A property whose computed value is a number clipped to [0,1] takes a percentage as
        // that fraction: `opacity: 50%` computes to `0.5` (css-color-4 §3.2). Converted before
        // the range is applied, or the `50` would be clamped against `[0,1]` and come out `1%`.
        let computed = match computed {
            CssValue::Percentage(pct) if id.percentage_is_number() => CssValue::Number(pct / 100.0, NumberKind::Number),
            other => other,
        };
        // Only a single number has a magnitude to clamp. A list is several values, and the range
        // belongs to whichever grammar arm each one matched - which is not recorded.
        let magnitude = match &computed {
            CssValue::Number(n, _) | CssValue::Percentage(n) | CssValue::Unit(n, _) => *n,
            CssValue::Zero => 0.0,
            _ => return computed,
        };

        // A shorthand's own value is never what gets computed - it is expanded into longhands
        // first - and the range it reports is whatever range its longhands' grammars happened to
        // mention, which belongs to those longhands rather than to the shorthand.
        if id.is_shorthand() {
            return computed;
        }
        let Some((min, max)) = defs.computed_range(id) else {
            return computed;
        };

        // NaN comes out as the bound rather than travelling on: `f64::max` answers the operand
        // that is not NaN, which is what css-values-4 asks for - `animation-duration:
        // calc(NaN * 1s)` computes to `0s`, not to NaN.
        let mut clamped = magnitude;
        if let Some(min) = min {
            clamped = clamped.max(min);
        }
        if let Some(max) = max {
            clamped = clamped.min(max);
        }
        if clamped == magnitude {
            return computed;
        }

        match computed {
            CssValue::Number(_, kind) => CssValue::Number(clamped, kind),
            CssValue::Percentage(_) => CssValue::Percentage(clamped),
            CssValue::Unit(_, unit) => CssValue::Unit(clamped, unit),
            CssValue::Zero => CssValue::Number(clamped, NumberKind::Number),
            other => other,
        }
    }

    // /// Returns true if the given property is a shorthand property (ie: border, margin etc.)
    #[must_use]
    pub fn is_shorthand(&self) -> bool {
        self.id.is_some_and(PropertyId::is_shorthand)
    }

    /// The longhands this shorthand expands to, or nothing when it is not a shorthand.
    #[must_use]
    pub fn get_props_from_shorthand(&self) -> Vec<String> {
        self.id.map_or_else(Vec::new, |id| {
            id.longhands()
                .iter()
                .map(|longhand| longhand.name().to_string())
                .collect()
        })
    }

    // // Returns the initial value for the property, if any
    fn get_initial_value(&self) -> Option<CssValue> {
        get_css_definitions().initial_value(self.id?)
    }
}

impl From<CssValue> for CssProperty {
    fn from(value: CssValue) -> Self {
        let mut this = Self::with_id(None);

        this.declared = vec![DeclarationProperty {
            location: no_location(),
            important: false,
            value: Arc::new(value),
            origin: CssOrigin::Author,
            specificity: Specificity::new(0, 0, 0),
            shadow_depth: 0,
            order: 0,
            layer: None,
            attached: false,
        }];

        this.calculate_value();

        this
    }
}

/// The location of a declaration nobody wrote: a value built straight from a `CssValue`, or a
/// longhand a shorthand's expansion synthesized.
#[must_use]
pub fn no_location() -> Arc<str> {
    Arc::from("")
}

impl From<CssValue> for DeclarationProperty {
    fn from(value: CssValue) -> Self {
        Self {
            location: no_location(),
            important: false,
            value: Arc::new(value),
            origin: CssOrigin::Author,
            specificity: Specificity::new(0, 0, 0),
            shadow_depth: 0,
            order: 0,
            layer: None,
            attached: false,
        }
    }
}

impl Display for CssProperty {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Display::fmt(&self.computed, f)
    }
}

impl css3::CssProperty<Css3System> for CssProperty {
    fn compute_value(&mut self) {
        self.compute_value();
    }
    fn unit_to_px(&self) -> f32 {
        self.computed.unit_to_px()
    }

    fn as_string(&self) -> Option<&str> {
        if let CssValue::String(str) = &self.computed {
            Some(str)
        } else {
            None
        }
    }

    fn as_percentage(&self) -> Option<f32> {
        if let CssValue::Percentage(percent) = &self.computed {
            Some(*percent as f32)
        } else {
            None
        }
    }

    fn as_unit(&self) -> Option<(f32, &str)> {
        if let CssValue::Unit(value, unit) = &self.computed {
            Some((*value as f32, unit))
        } else {
            None
        }
    }

    fn as_color(&self) -> Option<(f32, f32, f32, f32)> {
        if let CssValue::Color(color) = &self.computed {
            let color = color.to_rgb();
            Some((color.r, color.g, color.b, color.a))
        } else {
            None
        }
    }

    fn parse_color(&self) -> Option<(f32, f32, f32, f32)> {
        self.computed
            .to_color()
            .map(|color| (color.r, color.g, color.b, color.a))
    }

    fn as_number(&self) -> Option<f32> {
        match &self.computed {
            CssValue::Number(num, _) => Some(*num as f32),
            // A bare `0` parses to the dedicated `Zero` variant; surface it as the number 0 so
            // consumers (e.g. unitless `top: 0`, `margin: 0`) see it instead of dropping the value.
            CssValue::Zero => Some(0.0),
            _ => None,
        }
    }

    fn as_list(&self) -> Option<&[CssValue]> {
        if let CssValue::List(list) = &self.computed {
            Some(list)
        } else {
            None
        }
    }

    fn as_function(&self) -> Option<(&str, &[CssValue])> {
        if let CssValue::Function(name, args) = &self.computed {
            Some((name.as_str(), args))
        } else {
            None
        }
    }

    fn is_none(&self) -> bool {
        matches!(self.computed, CssValue::None)
    }
}

/// What one element hands down to everything below it: the values its own cascade settled,
/// layered over what its parent handed it.
///
/// This is what `inherit` and `unset` resolve against. It used to be carried by writing the
/// parent's computed value into the child's map for every property that inherits, which gave a
/// deep element an entry for everything any ancestor had ever declared - and cost a value clone
/// per property per element, for values almost nothing ever reads. Here each element records
/// only what it settled itself, once, and shares it with all its children; the answer to "what
/// does this element inherit for x" is a walk up a list that is a handful of entries at each
/// step.
#[derive(Debug)]
pub struct InheritedValues {
    /// What the element above handed down, if there is one.
    parent: Option<Arc<InheritedValues>>,
    /// The properties that inherit which this element settled. Kept apart from the rest
    /// because these are the ones a walk up the chain reads at every level, and a level holds
    /// a handful of them where it holds dozens of properties in all.
    inheriting: Vec<(PropertyId, CssValue)>,
    /// Everything else this element settled, which only the element directly below can ask
    /// for.
    rest: Vec<(PropertyId, CssValue)>,
}

impl InheritedValues {
    /// What an element whose parent handed this down inherits for `id`.
    ///
    /// A property that inherits keeps travelling until some ancestor settled it. One that does
    /// not inherit stops at the first level: `inherit` on a `width` names the parent's computed
    /// value, and for a parent that declared none that is the initial value, not the
    /// grandparent's.
    #[must_use]
    pub fn get(&self, id: PropertyId) -> Option<&CssValue> {
        fn find(entries: &[(PropertyId, CssValue)], id: PropertyId) -> Option<&CssValue> {
            entries.iter().find(|(own, _)| *own == id).map(|(_, value)| value)
        }
        if !id.inherited() {
            return find(&self.rest, id);
        }
        let mut level = Some(self);
        while let Some(current) = level {
            if let Some(value) = find(&current.inheriting, id) {
                return Some(value);
            }
            level = current.parent.as_deref();
        }
        None
    }
}

/// Unlinks the chain one level at a time. The derived drop recursed once per level, so a document
/// nesting 20,000 elements aborted on the stack when its styles were dropped. Stops at the first
/// level something else still holds, which is where the derived drop would have stopped too.
impl Drop for InheritedValues {
    fn drop(&mut self) {
        let mut next = self.parent.take();
        while let Some(level) = next {
            next = match Arc::try_unwrap(level) {
                Ok(mut level) => level.parent.take(),
                Err(_) => None,
            };
        }
    }
}

/// Map of all declared values for a single node. Note that these are only the defined properties, not
/// the non-existing properties.
///
/// Keyed by [`PropertyId`] rather than by name, and densely: `slots` is one entry per property
/// this engine knows, holding where that property sits in `props`. A lookup is an array index
/// where it used to be a string hash, and an insert costs no allocation at all - which on a page
/// where every element carries a hundred declarations is most of what styling it used to do.
pub struct CssProperties {
    /// The properties this element has an entry for, in the order they were first declared.
    props: Vec<CssProperty>,
    /// One slot per [`PropertyId::index`]: the position in `props` plus one, or zero for a
    /// property this element has no entry for.
    slots: Box<[u16]>,
    pub dirty: bool,
    /// This element's computed `font-size` in px, resolved while the map was built.
    ///
    /// Kept on the map so a child can read its parent's basis without recomputing it - `em`
    /// resolves against the element's own font-size, which is itself inherited when undeclared,
    /// so every level needs the level above it.
    pub font_size_px: f32,
    /// The root element's computed `font-size` in px - what a `rem` is worth anywhere in the
    /// document. Carried down the tree rather than looked up, since the cascade walks top-down
    /// and only ever holds the parent's map.
    pub root_font_size_px: f32,
    /// This element's computed `line-height` in px, what an `lh` is worth on it, resolved while
    /// the map was built. A `normal` line-height is the engine's stand-in multiple of the
    /// font-size. Kept for the children, whose `line-height: 2lh` is twice this.
    pub line_height_px: f32,
    /// What a child that does not declare `line-height` inherits from this element: a length as
    /// the length, and a unitless number (or `normal`) as a multiple of the child's own
    /// font-size. Kept here because looking the value up through the inherited chain walks every
    /// ancestor up to the one that declared it, which is quadratic in a deep tree.
    pub line_height_inherits: InheritedLineHeight,
    /// The root element's computed `line-height` in px - what an `rlh` is worth anywhere in the
    /// document.
    pub root_line_height_px: f32,
    /// Custom properties (`--*`) in scope for this node, own declarations layered over the
    /// parent's. Shared with the parent when the node adds nothing: with frameworks that reset
    /// dozens of `--x` on `*`, copying them per element was the dominant cost of styling.
    pub custom: Arc<HashMap<String, CssValue>>,
    /// The document's `@property` registrations, built once at the root and handed down, or
    /// `None` when it has none.
    pub(crate) registry: Option<Arc<PropertyRegistry>>,
    /// This element's values for registered custom properties that do not inherit. They are
    /// not in `custom`, which is handed down, so a child is back at their initial values.
    pub custom_local: Vec<(String, CssValue)>,
    /// The node this map was computed for, so that the next element down can tell whether the
    /// map it was handed is really its parent's.
    pub(crate) node: Option<NodeId>,
    /// What the elements *above* `node` carry, as the ancestor filter summarises them.
    ///
    /// Kept here so the next element down does not walk and re-hash the whole ancestor chain:
    /// given the parent's map, a child's filter is the parent's plus the parent element's own
    /// keys, and a pseudo-element's is its originating element's unchanged. `None` when no
    /// candidate selector ever asked about an ancestor, in which case nothing was built - see
    /// [`crate::matcher::bloom::ancestor_filter`].
    pub(crate) ancestors: Option<Arc<AncestorFilter>>,
    /// What the parent element handed down, which is what this element inherits.
    inherited_from: Option<Arc<InheritedValues>>,
    /// What this element hands down, built the first time a child asks and shared from then on.
    /// A leaf never builds one.
    handed_down: OnceLock<Arc<InheritedValues>>,
}

impl Default for CssProperties {
    fn default() -> Self {
        Self::new()
    }
}

impl Debug for CssProperties {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The slot table is 665 mostly-empty entries and says nothing a reader wants; what a
        // property map is, is its properties.
        f.debug_struct("CssProperties")
            .field("properties", &self.props)
            .field("dirty", &self.dirty)
            .field("font_size_px", &self.font_size_px)
            .field("root_font_size_px", &self.root_font_size_px)
            .field("line_height_px", &self.line_height_px)
            .field("root_line_height_px", &self.root_line_height_px)
            .field("custom", &self.custom)
            .finish()
    }
}

impl CssProperties {
    #[must_use]
    pub fn new() -> Self {
        Self {
            props: Vec::new(),
            slots: vec![0; PROPERTY_COUNT].into_boxed_slice(),
            dirty: true,
            custom: Arc::new(HashMap::new()),
            registry: None,
            custom_local: Vec::new(),
            font_size_px: DEFAULT_FONT_SIZE_PX,
            root_font_size_px: DEFAULT_FONT_SIZE_PX,
            line_height_px: DEFAULT_LINE_HEIGHT_PX,
            line_height_inherits: InheritedLineHeight::FontSizeMultiple(
                #[expect(clippy::cast_possible_truncation, reason = "a line-height fits an f32")]
                {
                    calc::NORMAL_LINE_HEIGHT as f32
                },
            ),
            root_line_height_px: DEFAULT_LINE_HEIGHT_PX,
            inherited_from: None,
            handed_down: OnceLock::new(),
            node: None,
            ancestors: None,
        }
    }

    pub fn get(&mut self, name: &str) -> Option<&mut CssProperty> {
        self.get_id_mut(PropertyId::from_name(name)?)
    }

    /// The entry for `id`, if this element has one.
    #[must_use]
    pub fn get_id(&self, id: PropertyId) -> Option<&CssProperty> {
        self.props.get(self.slot(id)?)
    }

    /// The entry for `id`, if this element has one.
    pub fn get_id_mut(&mut self, id: PropertyId) -> Option<&mut CssProperty> {
        let slot = self.slot(id)?;
        self.props.get_mut(slot)
    }

    /// What this element inherits for `id`: the nearest ancestor's computed value, or `None`
    /// when nothing up the chain ever settled one and the property falls back to its initial
    /// value.
    ///
    /// Asked rather than read off the entry so a caller does not have to know whether the
    /// element has an entry for `id` at all - which depends on how inheritance is carried down,
    /// not on what the cascade decided.
    #[must_use]
    pub fn inherited_value(&self, id: PropertyId) -> Option<&CssValue> {
        self.inherited_from.as_ref()?.get(id)
    }

    /// What this element hands down to its children.
    ///
    /// Built from the computed values, so every caller resolves an element's map before styling
    /// the elements below it - which is what the cascade's top-down order means. Built once:
    /// every child of the same element shares the one record.
    #[must_use]
    pub fn handed_down(&self) -> Arc<InheritedValues> {
        Arc::clone(self.handed_down.get_or_init(|| {
            let (inheriting, rest) = self
                .props
                .iter()
                .filter_map(|property| {
                    let id = property.id?;
                    if matches!(property.computed, CssValue::None) {
                        return None;
                    }
                    // A property that does not inherit is only read from here by a child's
                    // explicit `inherit`, and a parent at the initial value hands down the same
                    // thing by having no entry. Shorthands reset most of their longhands to that
                    // value, so leaving those out keeps them from being copied into every
                    // element's record.
                    if !id.inherited()
                        && get_css_definitions()
                            .definition(id)
                            .and_then(|definition| definition.initial_value.as_ref())
                            == Some(&property.computed)
                    {
                        return None;
                    }
                    Some((id, property.computed.clone()))
                })
                .partition(|(id, _)| id.inherited());
            Arc::new(InheritedValues {
                parent: self.inherited_from.clone(),
                inheriting,
                rest,
            })
        }))
    }

    /// Point this element at what its parent hands down, and fill in the inherited value of
    /// every entry that can read one.
    ///
    /// Only two things ever read it: the CSS-wide keywords, which is what `inherit` and `unset`
    /// name, and `currentcolor` on `color`, which means the inherited colour. Every other entry
    /// is left alone rather than given a copy of a value nothing will ask for.
    pub fn inherit_from(&mut self, parent: &CssProperties) {
        let chain = parent.handed_down();
        for property in &mut self.props {
            let Some(id) = property.id else { continue };
            let wanted = id == COLOR || property.declared.iter().any(|d| css_wide_keyword(&d.value).is_some());
            if !wanted {
                continue;
            }
            if let Some(value) = chain.get(id) {
                property.inherited = value.clone();
                property.mark_dirty();
            }
        }
        self.inherited_from = Some(chain);
    }

    /// The entry for `id`, added with nothing declared if it is not there yet.
    pub fn entry(&mut self, id: PropertyId) -> &mut CssProperty {
        if let Some(slot) = self.slot(id) {
            // Reborrowed rather than returned from the branch above: the borrow checker reads
            // `get_mut` in an early return as borrowing `self` for the whole function.
            #[expect(clippy::indexing_slicing, reason = "the slot table only ever holds live positions")]
            return &mut self.props[slot];
        }
        self.push(id, CssProperty::new(id))
    }

    /// Replace the entry for `id`, or add it.
    pub fn insert_id(&mut self, id: PropertyId, mut value: CssProperty) {
        value.id = Some(id);
        match self.slot(id) {
            Some(slot) => {
                #[expect(clippy::indexing_slicing, reason = "the slot table only ever holds live positions")]
                {
                    self.props[slot] = value;
                }
            }
            None => {
                self.push(id, value);
            }
        }
    }

    /// How many properties this element has an entry for.
    #[must_use]
    pub fn len(&self) -> usize {
        self.props.len()
    }

    /// Whether this element has no entries at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.props.is_empty()
    }

    /// Every entry with its id, in the order the properties were first declared.
    pub fn iter_ids(&self) -> impl Iterator<Item = (PropertyId, &CssProperty)> + '_ {
        self.props.iter().filter_map(|prop| Some((prop.id?, prop)))
    }

    /// Every entry with its id, in the order the properties were first declared.
    pub fn iter_ids_mut(&mut self) -> impl Iterator<Item = (PropertyId, &mut CssProperty)> + '_ {
        self.props.iter_mut().filter_map(|prop| Some((prop.id?, prop)))
    }

    fn slot(&self, id: PropertyId) -> Option<usize> {
        match self.slots.get(id.index()).copied().unwrap_or(0) {
            0 => None,
            slot => Some(usize::from(slot) - 1),
        }
    }

    fn push(&mut self, id: PropertyId, value: CssProperty) -> &mut CssProperty {
        self.props.push(value);
        let position = self.props.len();
        if let Some(slot) = self.slots.get_mut(id.index()) {
            // Every position fits: there are fewer properties than there are ids, and there are
            // 665 of those.
            #[expect(clippy::cast_possible_truncation, reason = "a slot is at most PROPERTY_COUNT")]
            {
                *slot = position as u16;
            }
        }
        #[expect(clippy::indexing_slicing, reason = "just pushed")]
        &mut self.props[position - 1]
    }
}

impl CssPropertyMap<Css3System> for CssProperties {
    fn computed_style(&self, parent: Option<&ComputedStyle>) -> ComputedStyle {
        crate::matcher::computed_style::computed_style(self, parent)
    }

    fn insert_inherited(&mut self, name: &str, value: CssProperty) {
        let Some(id) = PropertyId::from_name(name) else {
            return;
        };
        if self.slot(id).is_none() {
            self.insert_id(id, value);
        }
    }

    fn insert(&mut self, name: &str, value: CssProperty) {
        let Some(id) = PropertyId::from_name(name) else {
            return;
        };
        self.insert_id(id, value);
    }

    fn get(&self, name: &str) -> Option<&CssProperty> {
        self.get_id(PropertyId::from_name(name)?)
    }

    fn get_mut(&mut self, name: &str) -> Option<&mut CssProperty> {
        self.get_id_mut(PropertyId::from_name(name)?)
    }

    fn make_dirty(&mut self) {
        self.dirty = true;
    }

    fn iter(&self) -> impl Iterator<Item = (&str, &CssProperty)> + '_ {
        self.iter_ids().map(|(id, prop)| (id.name(), prop))
    }

    fn iter_mut(&mut self) -> impl Iterator<Item = (&str, &mut CssProperty)> + '_ {
        self.iter_ids_mut().map(|(id, prop)| (id.name(), prop))
    }

    fn make_clean(&mut self) {
        self.dirty = false;
    }

    fn is_dirty(&self) -> bool {
        self.dirty
    }

    fn inherited_scope_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.custom, &other.custom) || *self.custom == *other.custom
    }
}

// ── Memory reporting ─────────────────────────────────────────────────────────
//
// These sit beside the types rather than in `crate::memory` because the fields they walk are
// private, and because a field added later should be obvious to whoever adds it that it needs
// counting. The collector that turns them into report rows is in `crate::memory`.

impl gosub_shared::memory::HeapSize for DeclarationProperty {
    fn heap_size(&self, walk: &mut gosub_shared::memory::Walk) {
        self.value.heap_size(walk);
        // One `Arc<str>` per stylesheet, shared by every declaration that came from it. Counting
        // it once is the whole reason it is an `Arc` - it used to be a `String` per declaration.
        walk.shared_once(Arc::as_ptr(&self.location).cast::<u8>() as usize, |walk| {
            walk.bytes(self.location.len() + 2 * size_of::<usize>());
        });
    }
}

impl gosub_shared::memory::HeapSize for CssProperty {
    fn heap_size(&self, walk: &mut gosub_shared::memory::Walk) {
        self.declared.heap_size(walk);
        self.computed.heap_size(walk);
        self.inherited.heap_size(walk);
    }
}

impl gosub_shared::memory::HeapSize for InheritedValues {
    fn heap_size(&self, walk: &mut gosub_shared::memory::Walk) {
        walk.bytes(self.inheriting.capacity() * size_of::<(PropertyId, CssValue)>());
        walk.bytes(self.rest.capacity() * size_of::<(PropertyId, CssValue)>());
        for (_, value) in &self.inheriting {
            value.heap_size(walk);
        }
        for (_, value) in &self.rest {
            value.heap_size(walk);
        }
        // Walking up the chain costs nothing after the first element to reach each level: the
        // `Arc` impl counts a record once per snapshot, which is what makes this row honest.
        if let Some(parent) = &self.parent {
            parent.heap_size(walk);
        }
    }
}

impl gosub_shared::memory::HeapSize for CssProperties {
    fn heap_size(&self, walk: &mut gosub_shared::memory::Walk) {
        self.props.heap_size(walk);
        walk.bytes(size_of_val(&*self.slots));
        self.custom.heap_size(walk);
        walk.bytes(self.custom_local.capacity() * size_of::<(String, CssValue)>());
        for (name, value) in &self.custom_local {
            name.heap_size(walk);
            value.heap_size(walk);
        }
        if let Some(filter) = &self.ancestors {
            filter.heap_size(walk);
        }
        if let Some(record) = &self.inherited_from {
            record.heap_size(walk);
        }
        if let Some(record) = self.handed_down.get() {
            record.heap_size(walk);
        }
    }
}

/// Accessors the memory collector needs to report a map's parts as separate rows. Nothing else
/// uses them, and they hand out shared references only.
impl CssProperties {
    pub(crate) fn props_slice(&self) -> &[CssProperty] {
        &self.props
    }

    /// The slots the `props` vector has allocated, which is what it costs. Its length is what
    /// the element declared; the difference is the doubling the last push paid for.
    pub(crate) fn props_capacity(&self) -> usize {
        self.props.capacity()
    }

    pub(crate) fn slot_len(&self) -> usize {
        self.slots.len()
    }

    pub(crate) fn inherited_record(&self) -> Option<&Arc<InheritedValues>> {
        self.inherited_from.as_ref()
    }

    pub(crate) fn handed_down_record(&self) -> Option<&Arc<InheritedValues>> {
        self.handed_down.get()
    }

    pub(crate) fn ancestor_filter(&self) -> Option<&Arc<crate::matcher::bloom::AncestorFilter>> {
        self.ancestors.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use crate::colors::RgbColor;
    use crate::system::prop_is_inherit;

    use super::*;

    /// The id of a property the tests name, which they do name by name.
    fn id(name: &str) -> PropertyId {
        PropertyId::from_name(name).expect("a property the tests name")
    }

    /// RFC 4647 extended filtering, as `:lang()` uses it.
    #[test]
    fn language_ranges_use_extended_filtering() {
        assert!(language_range_matches("en", "en"));
        assert!(language_range_matches("en", "en-US"), "a prefix range");
        assert!(language_range_matches("EN-us", "en-US"), "case-insensitive");
        assert!(!language_range_matches("en", "eng"), "whole subtags only");
        assert!(!language_range_matches("en-US", "en"), "a longer range");
        assert!(
            language_range_matches("de-DE", "de-Latn-DE"),
            "a range subtag may skip one"
        );
        assert!(language_range_matches("*-CH", "fr-CH"), "a wildcard primary subtag");
        assert!(language_range_matches("de-*-DE", "de-Latn-DE"));
        assert!(!language_range_matches("en-US", "en-x-US"), "never past a singleton");
        assert!(
            !language_range_matches("en", ""),
            "an unknown language matches no range"
        );
        assert!(language_range_matches("", ""), "except the empty one");
    }

    #[test]
    fn css_props() {
        let mut props = CssProperties::new();
        let prop = CssProperty::new(id("color"));
        props.insert_id(id("color"), prop);

        let prop = props.get("color").unwrap();
        assert_eq!(prop.name(), "color");

        let prop = props.get("not-exists");
        assert!(prop.is_none());
    }

    /// One level per element, so a document nesting 200,000 elements builds a chain this long.
    /// The derived drop recursed once per level and aborted the process on a 2 MiB stack.
    #[test]
    fn a_long_inherited_chain_drops_without_recursing() {
        std::thread::Builder::new()
            .stack_size(2 * 1024 * 1024)
            .spawn(|| {
                let mut chain: Option<Arc<InheritedValues>> = None;
                for _ in 0..200_000 {
                    chain = Some(Arc::new(InheritedValues {
                        parent: chain.take(),
                        inheriting: Vec::new(),
                        rest: Vec::new(),
                    }));
                }
                drop(chain);
            })
            .expect("spawn")
            .join()
            .expect("dropping the chain must not abort the process");
    }

    /// Dropping a child must leave the levels someone else still holds intact.
    #[test]
    fn dropping_a_level_keeps_shared_ancestors() {
        let color = id("color");
        let root = Arc::new(InheritedValues {
            parent: None,
            inheriting: vec![(color, CssValue::String("red".into()))],
            rest: Vec::new(),
        });
        let middle = Arc::new(InheritedValues {
            parent: Some(Arc::clone(&root)),
            inheriting: Vec::new(),
            rest: Vec::new(),
        });
        let leaf = InheritedValues {
            parent: Some(Arc::clone(&middle)),
            inheriting: Vec::new(),
            rest: Vec::new(),
        };
        drop(root);
        drop(leaf);
        assert_eq!(middle.get(color), Some(&CssValue::String("red".into())));
    }

    #[test]
    fn border_prop_test() {
        let mut prop = CssProperty::new(id("border"));

        prop.declared.push(DeclarationProperty {
            value: CssValue::List(vec![
                CssValue::Unit(1.0, "px".into()),
                CssValue::String("solid".into()),
                CssValue::Color(RgbColor::new(255.0, 0.0, 0.0, 255.0).into()),
            ])
            .into(),
            origin: CssOrigin::Author,
            important: false,
            location: no_location(),
            specificity: Specificity::new(1, 0, 0),
            shadow_depth: 0,
            order: 0,
            layer: None,
            attached: false,
        });

        assert_eq!(
            prop.compute_value(),
            &CssValue::List(vec![
                CssValue::Unit(1.0, "px".into()),
                CssValue::String("solid".into()),
                CssValue::Color(RgbColor::from("red").into()),
            ])
        );
        assert!(prop.is_shorthand());
        assert_eq!(prop.name(), "border");
        assert_eq!(prop.get_initial_value(), Some(CssValue::None));
        assert!(!prop_is_inherit(prop.name()));
    }

    #[test]
    fn color_prop_test() {
        let mut prop = CssProperty::new(id("color"));

        prop.declared.push(DeclarationProperty {
            value: CssValue::String("red".into()).into(),
            origin: CssOrigin::Author,
            important: false,
            location: no_location(),
            specificity: Specificity::new(1, 0, 0),
            shadow_depth: 0,
            order: 0,
            layer: None,
            attached: false,
        });

        // The computed value of a colour keyword is the colour it names (css-color-4 §15). The
        // keyword itself is the *specified* value, which is what `element.style` reads back.
        assert_eq!(prop.compute_value(), &CssValue::Color(RgbColor::from("red").into()));
        assert!(!prop.is_shorthand());
        assert_eq!(prop.name(), "color");
        // css-color-4 gives `color` an initial value of `canvastext`. This asserted `None`,
        // which was not a fact about the property but about the loader: it looked for an
        // `initial_value` key the definitions file has never had, so every initial value was
        // absent.
        assert_eq!(
            prop.get_initial_value(),
            Some(CssValue::String("canvastext".to_string()))
        );
        assert!(prop_is_inherit(prop.name()));
    }

    #[test]
    fn the_initial_keyword_resolves_to_the_property_s_initial_value() {
        // css-cascade §7.1. The parser lowers the CSS-wide keywords to a plain string, so that is
        // the form this has to recognise; the dedicated `CssValue::Initial` variant is checked too
        // because callers that build values directly produce it. `text-align`'s definition names
        // its initial value in prose, which resolved to nothing and so inherited instead.
        let cases = [("width", "auto"), ("text-align", "start")];
        for ((name, initial), keyword) in cases.into_iter().flat_map(|case| {
            [CssValue::String("initial".to_string()), CssValue::Initial].map(|keyword| (case, keyword))
        }) {
            let mut prop = CssProperty::new(id(name));
            prop.declared.push(DeclarationProperty {
                value: keyword.into(),
                origin: CssOrigin::Author,
                important: false,
                location: no_location(),
                specificity: Specificity::new(1, 0, 0),
                shadow_depth: 0,
                order: 0,
                layer: None,
                attached: false,
            });

            assert_eq!(prop.compute_value(), &CssValue::String(initial.to_string()), "{name}");
        }
    }

    /// Compute one declared value for `name`, the way the cascade would.
    /// css-color-4 §3.2: a percentage `opacity` is the same fraction as the number. The clamp
    /// used to read the `50` of `50%` against `[0,1]` and answer `1%`.
    #[test]
    fn a_percentage_opacity_computes_to_the_fraction() {
        assert_eq!(
            computed_for("opacity", CssValue::Percentage(50.0)),
            CssValue::Number(0.5, NumberKind::Number)
        );
        assert_eq!(
            computed_for("opacity", CssValue::Percentage(150.0)),
            CssValue::Number(1.0, NumberKind::Number)
        );
        // A percentage on a property whose range is in its own units is left as one.
        assert_eq!(
            computed_for("width", CssValue::Percentage(50.0)),
            CssValue::Percentage(50.0)
        );
    }

    fn computed_for(name: &str, value: CssValue) -> CssValue {
        let mut prop = CssProperty::new(id(name));
        prop.declared.push(DeclarationProperty {
            value: value.into(),
            origin: CssOrigin::Author,
            important: false,
            location: no_location(),
            specificity: Specificity::new(1, 0, 0),
            shadow_depth: 0,
            order: 0,
            layer: None,
            attached: false,
        });
        prop.compute_value().clone()
    }

    /// Which longhands take a `<line-width>` comes from their grammar, not their name.
    #[test]
    fn line_width_properties_come_from_the_grammar() {
        let defs = get_css_definitions();
        let mut found: Vec<&str> = crate::matcher::property_ids::ALL_LONGHAND_IDS
            .iter()
            .map(|&longhand| PropertyId::Longhand(longhand))
            .filter(|&id| defs.takes_line_width(id))
            .map(PropertyId::name)
            .collect();
        found.sort_unstable();
        let mut expected = vec![
            "-webkit-border-after-width",
            "-webkit-border-before-width",
            "-webkit-border-end-width",
            "-webkit-border-start-width",
            "-webkit-text-stroke-width",
            "border-block-end-width",
            "border-block-start-width",
            "border-bottom-width",
            "border-inline-end-width",
            "border-inline-start-width",
            "border-left-width",
            "border-right-width",
            "border-top-width",
            "column-gap",
            "column-rule-width",
            "grid-column-gap",
            "grid-row-gap",
            "outline-width",
            "row-gap",
            "stroke-width",
        ];
        expected.sort_unstable();
        assert_eq!(found, expected);
    }

    #[test]
    fn line_width_keywords_compute_to_lengths() {
        let px = |n| CssValue::Unit(n, "px".into());
        let keyword = |k: &str| CssValue::String(k.into());
        assert_eq!(computed_for("border-top-width", keyword("thick")), px(5.0));
        assert_eq!(computed_for("outline-width", keyword("thin")), px(1.0));
        assert_eq!(computed_for("column-gap", keyword("medium")), px(3.0));
        assert_eq!(
            computed_for(
                "stroke-width",
                CssValue::List(vec![keyword("thin"), CssValue::Comma, keyword("thick")])
            ),
            CssValue::List(vec![px(1.0), CssValue::Comma, px(5.0)])
        );
        // `thin` is a keyword of its own here, not a width.
        assert_eq!(computed_for("scrollbar-width", keyword("thin")), keyword("thin"));
        // `normal` on a gap is not a line width either.
        assert_eq!(computed_for("row-gap", keyword("normal")), keyword("normal"));
    }

    /// `hairline` is one device pixel: 1px on a 1x display, half of one on a 2x display, where
    /// `thin` stays 1px. Below 1x a device pixel is more than 1px, and hairline stops at 1px.
    #[test]
    fn hairline_is_one_device_pixel() {
        use crate::media_query::{media_environment, set_media_environment, MediaEnvironment};
        let px = |n| CssValue::Unit(n, "px".into());
        let keyword = |k: &str| CssValue::String(k.into());
        let saved = media_environment();

        set_media_environment(MediaEnvironment {
            device_pixel_ratio: 1.0,
            ..saved
        });
        assert_eq!(computed_for("border-top-width", keyword("hairline")), px(1.0));

        set_media_environment(MediaEnvironment {
            device_pixel_ratio: 2.0,
            ..saved
        });
        assert_eq!(computed_for("border-top-width", keyword("hairline")), px(0.5));
        assert_eq!(computed_for("border-top-width", keyword("thin")), px(1.0));

        set_media_environment(MediaEnvironment {
            device_pixel_ratio: 0.5,
            ..saved
        });
        assert_eq!(computed_for("border-top-width", keyword("hairline")), px(1.0));

        set_media_environment(saved);
    }

    /// The `font-size` keywords compute to px against the parent's size, which is the basis the
    /// cascade gives `font-size`.
    #[test]
    fn font_size_keywords_compute_to_lengths() {
        let font_size = |value: &str, parent: f32| {
            let mut prop = CssProperty::new(FONT_SIZE);
            prop.font_size_basis = parent;
            prop.declared.push(DeclarationProperty {
                value: CssValue::String(value.into()).into(),
                origin: CssOrigin::Author,
                important: false,
                location: no_location(),
                specificity: Specificity::new(1, 0, 0),
                shadow_depth: 0,
                order: 0,
                layer: None,
                attached: false,
            });
            match prop.compute_value() {
                CssValue::Unit(px, unit) if unit == "px" => *px,
                other => panic!("{value} computed to {other:?}"),
            }
        };
        assert_eq!(font_size("large", 20.0), 18.0);
        assert_eq!(font_size("XX-Small", 20.0), 9.0);
        assert_eq!(font_size("medium", 20.0), 16.0);
        assert_eq!(font_size("initial", 20.0), 16.0, "the initial value is `medium`");
        assert_eq!(font_size("larger", 20.0), 24.0);
        assert!((font_size("smaller", 20.0) - 20.0 / 1.2).abs() < 1e-9);
        assert_eq!(font_size("math", 20.0), 20.0, "no math-depth, so the parent's size");
    }

    /// The `font` shorthand's own value says the same size as the longhand. A relative keyword is
    /// the element's own size, which is the basis the shorthand is given.
    #[test]
    fn the_font_shorthand_says_its_size_in_px() {
        let font = |size: &str| {
            let mut prop = CssProperty::new(FONT);
            prop.font_size_basis = 24.0;
            prop.declared.push(DeclarationProperty {
                value: CssValue::List(vec![CssValue::String(size.into()), CssValue::String("serif".into())]).into(),
                origin: CssOrigin::Author,
                important: false,
                location: no_location(),
                specificity: Specificity::new(1, 0, 0),
                shadow_depth: 0,
                order: 0,
                layer: None,
                attached: false,
            });
            prop.compute_value().clone()
        };
        let sized = |px: f64| CssValue::List(vec![CssValue::Unit(px, "px".into()), CssValue::String("serif".into())]);
        assert_eq!(font("xx-small"), sized(9.0));
        assert_eq!(font("larger"), sized(24.0));
        assert_eq!(font("smaller"), sized(24.0));
    }

    #[test]
    fn a_computed_value_is_clamped_into_the_property_s_range() {
        // css-values-4 §10.12. `width` is `<length-percentage [0,∞]>`, and a math function is
        // not range-checked when it is parsed, so the range has to bite here instead.
        assert_eq!(
            computed_for("width", CssValue::Unit(-5.0, "px".into())),
            CssValue::Unit(0.0, "px".into())
        );
        assert_eq!(
            computed_for("width", CssValue::Percentage(-10.0)),
            CssValue::Percentage(0.0)
        );
        // `tab-size` is `<number [0,∞]>`, so the same rule reaches a bare number.
        assert_eq!(
            computed_for("tab-size", CssValue::Number(-8.0, NumberKind::Number)),
            CssValue::Number(0.0, NumberKind::Number)
        );
        // And `column-count` is `<integer [1,∞]>`, where the bound is not zero.
        assert_eq!(
            computed_for("column-count", CssValue::Number(0.0, NumberKind::Integer)),
            CssValue::Number(1.0, NumberKind::Integer)
        );

        // NaN comes out as the bound rather than travelling on: css-values-4 asks for
        // `animation-duration: calc(NaN * 1s)` to compute to `0s`.
        assert_eq!(
            computed_for("width", CssValue::Unit(f64::NAN, "px".into())),
            CssValue::Unit(0.0, "px".into())
        );

        // The other half of the rule, and the reason clamping is not a way round the matcher: a
        // *literal* out of range is still rejected outright, because its value is known when the
        // declaration is parsed. Only a math function gets to be clamped instead.
        let defs = get_css_definitions();
        let width = defs.find_property("width").expect("width is defined");
        assert!(!width.matches(&[CssValue::Unit(-5.0, "px".into())]));
        assert!(width.matches(&[CssValue::Unit(5.0, "px".into())]));
        assert!(width.matches(&[CssValue::Function(
            "calc".to_string(),
            vec![CssValue::Unit(-5.0, "px".into())]
        )]));
    }

    #[test]
    fn opacity_is_clamped_by_its_computed_value_line_rather_than_its_grammar() {
        // `<opacity-value>` is `<number> | <percentage>` with no bounds written on it at all -
        // the [0,1] lives in css-color-4's computed-value line, and unlike a grammar range it
        // applies to every value, not only to math results. So `opacity: 1.5` computes to `1`.
        assert_eq!(
            computed_for("opacity", CssValue::Number(1.5, NumberKind::Number)),
            CssValue::Number(1.0, NumberKind::Number)
        );
        assert_eq!(
            computed_for("opacity", CssValue::Number(-1.0, NumberKind::Number)),
            CssValue::Number(0.0, NumberKind::Number)
        );
        assert_eq!(
            computed_for("opacity", CssValue::Number(0.4, NumberKind::Number)),
            CssValue::Number(0.4, NumberKind::Number)
        );
    }

    #[test]
    fn a_property_with_no_range_is_left_alone() {
        // Negative values are meaningful for these, and nothing in their grammar says otherwise.
        assert_eq!(
            computed_for("letter-spacing", CssValue::Unit(-5.0, "px".into())),
            CssValue::Unit(-5.0, "px".into())
        );
        assert_eq!(
            computed_for("z-index", CssValue::Number(-5.0, NumberKind::Integer)),
            CssValue::Number(-5.0, NumberKind::Integer)
        );
        assert_eq!(
            computed_for("margin-left", CssValue::Unit(-5.0, "px".into())),
            CssValue::Unit(-5.0, "px".into())
        );
    }

    #[test]
    fn compare_declared() {
        let a = DeclarationProperty {
            value: CssValue::String("red".into()).into(),
            origin: CssOrigin::Author,
            important: false,
            location: no_location(),
            specificity: Specificity::new(1, 0, 0),
            shadow_depth: 0,
            order: 0,
            layer: None,
            attached: false,
        };
        let b = DeclarationProperty {
            value: CssValue::String("blue".into()).into(),
            origin: CssOrigin::UserAgent,
            important: false,
            location: no_location(),
            specificity: Specificity::new(1, 0, 0),
            shadow_depth: 0,
            order: 0,
            layer: None,
            attached: false,
        };
        let c = DeclarationProperty {
            value: CssValue::String("green".into()).into(),
            origin: CssOrigin::User,
            important: false,
            location: no_location(),
            specificity: Specificity::new(1, 0, 0),
            shadow_depth: 0,
            order: 0,
            layer: None,
            attached: false,
        };
        let d = DeclarationProperty {
            value: CssValue::String("yellow".into()).into(),
            origin: CssOrigin::Author,
            important: true,
            location: no_location(),
            specificity: Specificity::new(1, 0, 0),
            shadow_depth: 0,
            order: 0,
            layer: None,
            attached: false,
        };
        let e = DeclarationProperty {
            value: CssValue::String("orange".into()).into(),
            origin: CssOrigin::UserAgent,
            important: true,
            location: no_location(),
            specificity: Specificity::new(1, 0, 0),
            shadow_depth: 0,
            order: 0,
            layer: None,
            attached: false,
        };
        let f = DeclarationProperty {
            value: CssValue::String("purple".into()).into(),
            origin: CssOrigin::User,
            important: true,
            location: no_location(),
            specificity: Specificity::new(1, 0, 0),
            shadow_depth: 0,
            order: 0,
            layer: None,
            attached: false,
        };

        assert_eq!(3, a.priority());
        assert_eq!(1, b.priority());
        assert_eq!(2, c.priority());
        assert_eq!(5, d.priority());
        assert_eq!(7, e.priority());
        assert_eq!(6, f.priority());

        assert!(a > b);
        assert!(b < c);
        assert!(c < d);
        assert!(d < e);
        assert!(f < e);
        assert!(a < e);
        assert!(b < d);
        assert!(a < d);
        assert!(b < d);
        assert!(c < d);
        assert_eq!(c, c);
        assert_eq!(d, d);
    }

    #[test]
    fn is_inheritable() {
        let prop = CssProperty::new(id("border"));
        assert!(!prop_is_inherit(prop.name()));

        let prop = CssProperty::new(id("color"));
        assert!(prop_is_inherit(prop.name()));

        let prop = CssProperty::new(id("font"));
        assert!(prop_is_inherit(prop.name()));

        let prop = CssProperty::new(id("border-top-color"));
        assert!(!prop_is_inherit(prop.name()));
    }

    #[test]
    fn shorthand_props() {
        let prop = CssProperty::new(id("border"));
        assert!(prop.is_shorthand());
        assert_eq!(
            prop.get_props_from_shorthand(),
            vec!["border-width", "border-style", "border-color"]
        );
        // A name this engine has no property for has no id at all, so it never reaches a map.
        assert!(CssProperty::from_name("window").is_none());
        let prop = CssProperty::from(CssValue::None);
        assert!(!prop.is_shorthand());
        assert!(prop.get_props_from_shorthand().is_empty());

        let prop = CssProperty::new(id("border-color"));
        assert!(prop.is_shorthand());
        assert_eq!(
            prop.get_props_from_shorthand(),
            vec![
                "border-top-color",
                "border-right-color",
                "border-bottom-color",
                "border-left-color",
            ]
        );

        let prop = CssProperty::new(id("border-top-color"));
        assert!(!prop.is_shorthand());
        assert!(prop.get_props_from_shorthand().is_empty());
    }
}
