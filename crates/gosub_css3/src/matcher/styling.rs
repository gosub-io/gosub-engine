use core::fmt::Debug;
use cow_utils::CowUtils;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt::Display;
use std::sync::Arc;

use gosub_interface::config::HasDocument;
use gosub_interface::css3;
use gosub_interface::css3::{CssOrigin, CssPropertyMap};
use gosub_interface::document::Document;
use gosub_interface::node::NodeType;
use gosub_shared::node::NodeId;

use crate::matcher::property_definitions::get_css_definitions;
use crate::stylesheet::{Combinator, CssSelector, CssSelectorPart, CssValue, MatcherType, Specificity};
use crate::system::Css3System;

// Matches a complete selector (all parts) against the given node(id).
//
// `pseudo` selects what we are matching against:
//   * `None`           - match the element itself. Any selector containing a `::pseudo-element`
//                        part never matches (pseudo-elements are not the element).
//   * `Some("before")` - match the `::before` pseudo-element of `node_id`. Only selectors that
//                        explicitly carry the matching `::before` part match; the rest of the
//                        compound is matched against the originating element as usual.
pub(crate) fn match_selector<C: HasDocument>(
    document: &C::Document,
    node_id: NodeId,
    selector: &CssSelector,
    pseudo: Option<&str>,
    scope: ScopeContext,
) -> (bool, Specificity) {
    // A selector list (`a, b`) matches with the highest specificity of its matching parts.
    let mut best: Option<Specificity> = None;
    for part in &selector.parts {
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
            let specificity = Specificity::from(part.as_slice());
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
                _wanted_buf = attr.name.cow_to_lowercase().to_string();
                _got_buf = got_raw.cow_to_lowercase().to_string();
                (&_wanted_buf, &_got_buf)
            } else {
                (&attr.value, got_raw)
            };

            match attr.matcher {
                MatcherType::None => true,
                MatcherType::Equals => wanted_attr_value == got_attr_value,
                MatcherType::Includes => wanted_attr_value.split_whitespace().any(|s| s == got_attr_value),
                MatcherType::DashMatch => {
                    got_attr_value == wanted_attr_value || got_attr_value.starts_with(&format!("{wanted_attr_value}-"))
                }
                MatcherType::PrefixMatch => got_attr_value.starts_with(wanted_attr_value),
                MatcherType::SuffixMatch => got_attr_value.ends_with(wanted_attr_value),
                MatcherType::SubstringMatch => got_attr_value.contains(wanted_attr_value),
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
            // Structural pseudo-classes
            "first-child" => {
                if let Some(parent_id) = doc.parent(current_id) {
                    let siblings = doc.children(parent_id);
                    siblings.first().is_some_and(|&id| id == current_id)
                } else {
                    false
                }
            }
            "last-child" => {
                if let Some(parent_id) = doc.parent(current_id) {
                    let siblings = doc.children(parent_id);
                    siblings.last().is_some_and(|&id| id == current_id)
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
            "checked" => doc.attribute(current_id, "checked").is_some(),
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
            "focus" | "focus-visible" => doc.is_focused(current_id),
            // :focus-within needs the focus chain; not tracked yet.
            "focus-within" => false,
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
                let Some(mut parent_id) = doc.parent(current_id) else {
                    return false;
                };

                let Some(last) = consume(parts) else {
                    return false;
                };

                loop {
                    *next_id = Some(parent_id);

                    if match_selector_part::<C>(last, parent_id, doc, next_id, parts, pseudo, scope) {
                        return true;
                    }

                    let Some(p) = doc.parent(parent_id) else {
                        return false;
                    };

                    parent_id = p;
                }
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

                if my_index == 0 {
                    return false;
                }

                let Some(&prev_id) = children.get(my_index - 1) else {
                    return false;
                };

                let Some(last) = consume(parts) else {
                    return false;
                };

                *next_id = Some(prev_id);

                match_selector_part::<C>(last, prev_id, doc, next_id, parts, pseudo, scope)
            }
            Combinator::SubsequentSibling => {
                let Some(parent_id) = doc.parent(current_id) else {
                    return false;
                };

                let children: Vec<NodeId> = doc.children(parent_id).to_vec();

                let Some(last) = consume(parts) else {
                    return false;
                };

                for child_id in children {
                    if child_id == current_id {
                        break;
                    }

                    if match_selector_part::<C>(last, child_id, doc, next_id, parts, pseudo, scope) {
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
    /// The actual value of the property (@todo: should this be a vec? or do we need to (re-)implement `CssValue::List`?)
    pub value: CssValue,
    /// Origin of the declaration (user stylesheet, author stylesheet etc.)
    pub origin: CssOrigin,
    /// Whether the declaration is !important
    pub important: bool,
    // @TODO: location should be a Location
    /// The location of the declaration in the stylesheet (name.css:123) or empty
    pub location: String,
    /// The specificity of the selector that declared this property
    pub specificity: Specificity,
    /// How many shadow boundaries deep the declaring stylesheet sits: 0 for the document,
    /// 1 for a sheet in a shadow tree hosted by a document element, and so on. Feeds the
    /// cross-tree half of the cascade; see [`DeclarationProperty::tree_rank`].
    pub shadow_depth: u16,
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
            .then_with(|| self.specificity.cmp(&other.specificity))
    }
}

/// A value entry contains all values for a single property for a single node. It contains the declared values, and
/// all the computed values.
#[derive(Debug, Clone)]
pub struct CssProperty {
    /// The name of the property
    pub name: String,
    /// True when this property needs to be recalculated
    pub dirty: bool,
    /// List of all declared values for this property
    pub declared: Vec<DeclarationProperty>,
    /// Cascaded value from the declared values (if any)
    pub cascaded: Option<CssValue>,
    // Specified value from the cascaded value (if any), or inherited value, or initial value
    pub specified: CssValue,
    // Computed value from the specified value (needs viewport size etc.)
    pub computed: CssValue,
    pub used: CssValue,
    // Actual value used in the rendering (after rounding, clipping etc.)
    pub actual: CssValue,
    pub inherited: CssValue,
}

impl CssProperty {
    #[must_use]
    pub fn new(prop_name: &str) -> Self {
        Self {
            name: prop_name.to_string(),
            dirty: true,
            declared: Vec::new(),
            cascaded: None,
            specified: CssValue::None,
            computed: CssValue::None,
            used: CssValue::None,
            actual: CssValue::None,
            inherited: CssValue::None,
        }
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub fn mark_clean(&mut self) {
        self.dirty = false;
    }

    /// Returns the actual value of the property. Will compute the value when needed
    pub fn compute_value(&mut self) -> &CssValue {
        if self.dirty {
            self.calculate_value();
            self.dirty = false;
        }

        &self.actual
    }

    fn calculate_value(&mut self) {
        self.cascaded = self.find_cascaded_value();
        self.specified = self.find_specified_value();
        self.computed = self.find_computed_value();
        self.used = self.find_used_value();
        self.actual = self.find_actual_value();
    }

    fn find_cascaded_value(&self) -> Option<CssValue> {
        self.declared.iter().max().map(|v| v.value.clone())
    }

    fn find_specified_value(&self) -> CssValue {
        self.cascaded.as_ref().unwrap_or(&self.inherited).clone()
    }

    fn find_computed_value(&self) -> CssValue {
        if self.specified != CssValue::None {
            return self.specified.clone();
        }

        self.get_initial_value().unwrap_or(CssValue::None)
    }

    fn find_used_value(&self) -> CssValue {
        self.computed.clone()
    }

    fn find_actual_value(&self) -> CssValue {
        // @TODO: stuff like clipping and such should occur as well
        // Bare numbers and percentages are ratios/multipliers and must keep their fractional
        // value: rounding `opacity: 0.15` to 0 makes an element vanish, `line-height: 1.7`
        // to 2.0 inflates every paragraph, `flex-grow: 0.5` to 1 doubles an item's share.
        // Relative units (em, rem, vw, vh) must not be rounded either - 1.5em rounded to
        // 2.0em would make h2 render at h1 size. Only absolute lengths (px, pt, in, cm, mm)
        // are snapped to whole values here.
        match &self.used {
            CssValue::Unit(value, unit) => {
                let absolute = matches!(unit.as_str(), "px" | "pt" | "in" | "cm" | "mm" | "pc" | "q");
                if absolute {
                    CssValue::Unit(value.round(), unit.clone())
                } else {
                    self.used.clone()
                }
            }
            _ => self.used.clone(),
        }
    }

    // /// Returns true if the given property is a shorthand property (ie: border, margin etc.)
    #[must_use]
    pub fn is_shorthand(&self) -> bool {
        let defs = get_css_definitions();
        match defs.find_property(&self.name) {
            Some(def) => def.expanded_properties().len() > 1,
            None => false,
        }
    }

    /// Returns the list of properties from a shorthand property, or just the property itself if it isn't a shorthand property.
    #[must_use]
    pub fn get_props_from_shorthand(&self) -> Vec<String> {
        let defs = get_css_definitions();
        match defs.find_property(&self.name) {
            Some(def) => {
                let props = def.expanded_properties();
                if props.len() == 1 {
                    vec![]
                } else {
                    props
                }
            }
            None => vec![],
        }
    }

    // // Returns the initial value for the property, if any
    fn get_initial_value(&self) -> Option<CssValue> {
        let defs = get_css_definitions();
        defs.find_property(&self.name)
            .map(super::property_definitions::PropertyDefinition::initial_value)
    }
}

impl From<CssValue> for CssProperty {
    fn from(value: CssValue) -> Self {
        let mut this = Self::new("unknown");

        this.declared = vec![DeclarationProperty {
            location: String::new(),
            important: false,
            value,
            origin: CssOrigin::Author,
            specificity: Specificity::new(0, 0, 0),
            shadow_depth: 0,
        }];

        this.calculate_value();

        this
    }
}

impl From<CssValue> for DeclarationProperty {
    fn from(value: CssValue) -> Self {
        Self {
            location: String::new(),
            important: false,
            value,
            origin: CssOrigin::Author,
            specificity: Specificity::new(0, 0, 0),
            shadow_depth: 0,
        }
    }
}

impl Display for CssProperty {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Display::fmt(&self.actual, f)
    }
}

impl css3::CssProperty<Css3System> for CssProperty {
    fn compute_value(&mut self) {
        self.compute_value();
    }
    fn unit_to_px(&self) -> f32 {
        self.actual.unit_to_px()
    }

    fn as_string(&self) -> Option<&str> {
        if let CssValue::String(str) = &self.actual {
            Some(str)
        } else {
            None
        }
    }

    fn as_percentage(&self) -> Option<f32> {
        if let CssValue::Percentage(percent) = &self.actual {
            Some(*percent)
        } else {
            None
        }
    }

    fn as_unit(&self) -> Option<(f32, &str)> {
        if let CssValue::Unit(value, unit) = &self.actual {
            Some((*value, unit))
        } else {
            None
        }
    }

    fn as_color(&self) -> Option<(f32, f32, f32, f32)> {
        if let CssValue::Color(color) = &self.actual {
            Some((color.r, color.g, color.b, color.a))
        } else {
            None
        }
    }

    fn parse_color(&self) -> Option<(f32, f32, f32, f32)> {
        self.actual.to_color().map(|color| (color.r, color.g, color.b, color.a))
    }

    fn as_number(&self) -> Option<f32> {
        match &self.actual {
            CssValue::Number(num) => Some(*num),
            // A bare `0` parses to the dedicated `Zero` variant; surface it as the number 0 so
            // consumers (e.g. unitless `top: 0`, `margin: 0`) see it instead of dropping the value.
            CssValue::Zero => Some(0.0),
            _ => None,
        }
    }

    fn as_list(&self) -> Option<&[CssValue]> {
        if let CssValue::List(list) = &self.actual {
            Some(list)
        } else {
            None
        }
    }

    fn as_function(&self) -> Option<(&str, &[CssValue])> {
        if let CssValue::Function(name, args) = &self.actual {
            Some((name.as_str(), args))
        } else {
            None
        }
    }

    fn is_none(&self) -> bool {
        matches!(self.actual, CssValue::None)
    }
}

/// Map of all declared values for a single node. Note that these are only the defined properties, not
/// the non-existing properties.
#[derive(Debug)]
pub struct CssProperties {
    pub properties: HashMap<String, CssProperty>,
    pub dirty: bool,
    /// Custom properties (`--*`) in scope for this node, own declarations layered over the
    /// parent's. Shared with the parent when the node adds nothing: with frameworks that reset
    /// dozens of `--x` on `*`, copying them per element was the dominant cost of styling.
    pub custom: Arc<HashMap<String, CssValue>>,
}

impl Default for CssProperties {
    fn default() -> Self {
        Self::new()
    }
}

impl CssProperties {
    #[must_use]
    pub fn new() -> Self {
        Self {
            properties: HashMap::new(),
            dirty: true,
            custom: Arc::new(HashMap::new()),
        }
    }

    pub fn get(&mut self, name: &str) -> Option<&mut CssProperty> {
        self.properties.get_mut(name)
    }
}

impl CssPropertyMap<Css3System> for CssProperties {
    fn insert_inherited(&mut self, name: &str, value: CssProperty) {
        self.properties.entry(name.to_string()).or_insert(value);
    }

    fn insert(&mut self, name: &str, value: CssProperty) {
        self.properties.insert(name.to_string(), value);
    }

    fn get(&self, name: &str) -> Option<&CssProperty> {
        self.properties.get(name)
    }

    fn get_mut(&mut self, name: &str) -> Option<&mut CssProperty> {
        self.properties.get_mut(name)
    }

    fn make_dirty(&mut self) {
        self.dirty = true;
    }

    fn iter(&self) -> impl Iterator<Item = (&str, &CssProperty)> + '_ {
        self.properties.iter().map(|(k, v)| (k.as_str(), v))
    }

    fn iter_mut(&mut self) -> impl Iterator<Item = (&str, &mut CssProperty)> + '_ {
        self.properties.iter_mut().map(|(k, v)| (k.as_str(), v))
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

#[cfg(test)]
mod tests {
    use crate::colors::RgbColor;
    use crate::system::prop_is_inherit;

    use super::*;

    #[test]
    fn css_props() {
        let mut props = CssProperties::new();
        let prop = CssProperty::new("color");
        props.properties.insert("color".into(), prop);

        let prop = props.get("color").unwrap();
        assert_eq!(prop.name, "color");

        let prop = props.get("not-exists");
        assert!(prop.is_none());
    }

    #[test]
    fn border_prop_test() {
        let mut prop = CssProperty::new("border");

        prop.declared.push(DeclarationProperty {
            value: CssValue::List(vec![
                CssValue::Unit(1.0, "px".into()),
                CssValue::String("solid".into()),
                CssValue::Color(RgbColor::new(255.0, 0.0, 0.0, 255.0)),
            ]),
            origin: CssOrigin::Author,
            important: false,
            location: String::new(),
            specificity: Specificity::new(1, 0, 0),
            shadow_depth: 0,
        });

        assert_eq!(
            prop.compute_value(),
            &CssValue::List(vec![
                CssValue::Unit(1.0, "px".into()),
                CssValue::String("solid".into()),
                CssValue::Color("red".into()),
            ])
        );
        assert!(prop.is_shorthand());
        assert_eq!(prop.name, "border");
        assert_eq!(prop.get_initial_value(), Some(CssValue::None));
        assert!(!prop_is_inherit(&prop.name));
    }

    #[test]
    fn color_prop_test() {
        let mut prop = CssProperty::new("color");

        prop.declared.push(DeclarationProperty {
            value: CssValue::String("red".into()),
            origin: CssOrigin::Author,
            important: false,
            location: String::new(),
            specificity: Specificity::new(1, 0, 0),
            shadow_depth: 0,
        });

        assert_eq!(prop.compute_value(), &CssValue::String("red".into()));
        assert!(!prop.is_shorthand());
        assert_eq!(prop.name, "color");
        assert_eq!(prop.get_initial_value(), Some(&CssValue::None).cloned());
        assert!(prop_is_inherit(&prop.name));
    }

    #[test]
    fn compare_declared() {
        let a = DeclarationProperty {
            value: CssValue::String("red".into()),
            origin: CssOrigin::Author,
            important: false,
            location: String::new(),
            specificity: Specificity::new(1, 0, 0),
            shadow_depth: 0,
        };
        let b = DeclarationProperty {
            value: CssValue::String("blue".into()),
            origin: CssOrigin::UserAgent,
            important: false,
            location: String::new(),
            specificity: Specificity::new(1, 0, 0),
            shadow_depth: 0,
        };
        let c = DeclarationProperty {
            value: CssValue::String("green".into()),
            origin: CssOrigin::User,
            important: false,
            location: String::new(),
            specificity: Specificity::new(1, 0, 0),
            shadow_depth: 0,
        };
        let d = DeclarationProperty {
            value: CssValue::String("yellow".into()),
            origin: CssOrigin::Author,
            important: true,
            location: String::new(),
            specificity: Specificity::new(1, 0, 0),
            shadow_depth: 0,
        };
        let e = DeclarationProperty {
            value: CssValue::String("orange".into()),
            origin: CssOrigin::UserAgent,
            important: true,
            location: String::new(),
            specificity: Specificity::new(1, 0, 0),
            shadow_depth: 0,
        };
        let f = DeclarationProperty {
            value: CssValue::String("purple".into()),
            origin: CssOrigin::User,
            important: true,
            location: String::new(),
            specificity: Specificity::new(1, 0, 0),
            shadow_depth: 0,
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
        let prop = CssProperty::new("border");
        assert!(!prop_is_inherit(&prop.name));

        let prop = CssProperty::new("color");
        assert!(prop_is_inherit(&prop.name));

        let prop = CssProperty::new("font");
        assert!(prop_is_inherit(&prop.name));

        let prop = CssProperty::new("border-top-color");
        assert!(!prop_is_inherit(&prop.name));
    }

    #[test]
    fn shorthand_props() {
        let prop = CssProperty::new("border");
        assert!(prop.is_shorthand());
        assert_eq!(
            prop.get_props_from_shorthand(),
            vec!["border-width", "border-style", "border-color"]
        );
        let prop = CssProperty::new("window");
        assert!(!prop.is_shorthand());
        assert!(prop.get_props_from_shorthand().is_empty());

        let prop = CssProperty::new("border-color");
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

        let prop = CssProperty::new("border-top-color");
        assert!(!prop.is_shorthand());
        assert!(prop.get_props_from_shorthand().is_empty());
    }
}
