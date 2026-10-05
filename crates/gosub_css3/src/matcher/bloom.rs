//! Ancestor bloom filter: what the elements above this one carry.
//!
//! The selector index narrows a stylesheet to the rules whose rightmost compound could match
//! the element, and on a real-world sheet that still leaves a few hundred per element, of which
//! almost none match. Most are rejected only after the matcher has walked the whole ancestor
//! chain looking for a class or an id that no ancestor has.
//!
//! A complex selector's compounds left of the subject are conditions on the element's
//! *ancestors*, and an ancestor either carries a given id, class, tag name or attribute name or
//! it does not. Summarising all of the element's ancestors as one small bit set answers that in
//! a couple of bit tests: if a key the selector requires is not in the set, no ancestor has it
//! and the walk can only fail.
//!
//! A bloom filter has false positives and no false negatives, so a "maybe" costs one matcher
//! call that would have happened anyway and a "no" is always right. Styling therefore decides
//! exactly what it decided before.

use cow_utils::CowUtils as _;
use gosub_interface::config::HasDocument;
use gosub_interface::document::Document;
use gosub_shared::node::NodeId;
use std::sync::Arc;

use crate::stylesheet::{Combinator, CssSelectorPart};

/// Bits in the filter. 256 bytes: small enough to live on a property map and be copied from
/// parent to child, and wide enough that a page's worth of ancestor keys leaves it sparse - the
/// deepest chains on the benchmark fixtures set a few dozen bits of the 2048.
const BITS: u32 = 2048;
/// `BITS` as 64-bit words.
const WORDS: usize = 32;
/// Bits needed to index one bit, so that the two probes take disjoint halves of the hash.
const BIT_SHIFT: u32 = 11;
const BIT_MASK: u32 = BITS - 1;

/// Which kind of simple selector a key came from, so that `.main` and `<main>` are two keys
/// rather than one.
const KIND_ID: u8 = 1;
const KIND_CLASS: u8 = 2;
const KIND_TAG: u8 = 3;
const KIND_ATTR: u8 = 4;

/// The golden-ratio constant FxHash multiplies by.
const GOLDEN: u32 = 0x9E37_79B9;

/// The ids, classes, tag names and attribute names of every ancestor of one element.
///
/// Not the element's own: every compound the filter is asked about is a condition on a *strict*
/// ancestor, so including the element itself would only widen the filter for nothing. That is
/// also what lets a child's filter be built from its parent's - see [`ancestor_filter`].
///
/// Held behind an `Arc` by everything that keeps one. A property map carries the filter of the
/// node it was computed for, and a pseudo-element's map is that same filter again, so sharing it
/// costs a refcount where owning it would cost 256 bytes on every map and on every move of one.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct AncestorFilter {
    words: [u64; WORDS],
}

impl gosub_shared::memory::HeapSize for AncestorFilter {
    fn heap_size(&self, _walk: &mut gosub_shared::memory::Walk) {
        // A fixed `[u64; 32]` and nothing else: all of it inline in whatever holds the filter,
        // which for the usual `Arc` is the allocation the `Arc` impl has already counted.
    }
}

impl AncestorFilter {
    fn new() -> Self {
        Self { words: [0; WORDS] }
    }

    fn insert(&mut self, hash: u32) {
        for bit in probes(hash) {
            if let Some(word) = self.words.get_mut((bit >> 6) as usize) {
                *word |= 1_u64 << (bit & 63);
            }
        }
    }

    fn may_contain(&self, hash: u32) -> bool {
        probes(hash).into_iter().all(|bit| {
            self.words
                .get((bit >> 6) as usize)
                .is_some_and(|word| word & (1_u64 << (bit & 63)) != 0)
        })
    }

    /// Whether some ancestor may carry every one of these keys. `false` is definite: at least
    /// one of them is carried by no ancestor at all, so the selector cannot match.
    ///
    /// An empty key list is a selector with no ancestor conditions, or one the filter does not
    /// summarise; both pass.
    pub(crate) fn may_match(&self, keys: &[u32]) -> bool {
        keys.iter().all(|&hash| self.may_contain(hash))
    }
}

/// The two bit positions one key sets and tests, taken from disjoint halves of the hash.
fn probes(hash: u32) -> [u32; 2] {
    [hash & BIT_MASK, (hash >> BIT_SHIFT) & BIT_MASK]
}

/// FxHash-style rotate-xor-multiply over the bytes, finished with a mix so that the low and
/// middle bits the two probes read do not move together.
fn hash_key(kind: u8, name: &str) -> u32 {
    let mut hash = GOLDEN ^ u32::from(kind);
    for byte in name.as_bytes() {
        hash = (hash.rotate_left(5) ^ u32::from(*byte)).wrapping_mul(GOLDEN);
    }
    hash ^= hash >> 15;
    hash = hash.wrapping_mul(0x2545_F491);
    hash ^ (hash >> 13)
}

/// What the ancestors of `id` carry.
///
/// `known` is a filter a property map already carries: the node it was computed for, and what
/// *that* node's ancestors carry. Two shapes of it answer this question without walking
/// anything, and between them they cover every call the ordinary top-down pass makes:
///
/// * the map is this element's own, which is what the pseudo-element path hands down - the
///   originating element's ancestors are the pseudo-element's, so the filter is already right
///   and nothing is hashed at all;
/// * the map is the DOM parent's, which is the ordinary element pass - our ancestors are its
///   ancestors plus the parent itself, so one node's keys go into a copy of it.
///
/// Anything else walks `doc.parent` from `id` upward. That is what a slotted node needs, since
/// the render pipeline hands down its flat-tree parent - the slot - rather than its DOM parent,
/// and what `style_dump` and `getComputedStyle` need when the map they hold is some further
/// ancestor's. The walk stays the definition and the two shortcuts are checked against it by a
/// debug assertion, so every test run and every debug-build dump proves them equal.
///
/// The walk stops where the matcher's does, at a node with no parent - which for a shadow tree
/// is its shadow root, so a shadow tree's rules never see the host or the light DOM above it.
pub(crate) fn ancestor_filter<C: HasDocument>(
    doc: &C::Document,
    id: NodeId,
    known: Option<(NodeId, &Arc<AncestorFilter>)>,
) -> Arc<AncestorFilter> {
    if let Some((node, filter)) = known {
        if node == id {
            debug_assert!(
                **filter == walk_ancestors::<C>(doc, id),
                "own-map filter differs from the walk"
            );
            return Arc::clone(filter);
        }
        if Some(node) == doc.parent(id) {
            let mut extended = (**filter).clone();
            insert_node_keys::<C>(&mut extended, doc, node);
            debug_assert!(
                extended == walk_ancestors::<C>(doc, id),
                "parent-map filter differs from the walk"
            );
            return Arc::new(extended);
        }
    }
    Arc::new(walk_ancestors::<C>(doc, id))
}

/// The definition: every ancestor of `id`, found by walking `doc.parent`.
fn walk_ancestors<C: HasDocument>(doc: &C::Document, id: NodeId) -> AncestorFilter {
    let mut filter = AncestorFilter::new();
    let mut current = doc.parent(id);
    while let Some(ancestor) = current {
        insert_node_keys::<C>(&mut filter, doc, ancestor);
        current = doc.parent(ancestor);
    }
    filter
}

/// Add one node's own id, classes, tag name and attribute names.
fn insert_node_keys<C: HasDocument>(filter: &mut AncestorFilter, doc: &C::Document, node: NodeId) {
    if let Some(tag) = doc.tag_name(node) {
        filter.insert(hash_key(KIND_TAG, tag));
    }
    if let Some(value) = doc.attribute(node, "id") {
        filter.insert(hash_key(KIND_ID, value));
    }
    if let Some(classes) = doc.attribute(node, "class") {
        for class in classes.split_ascii_whitespace() {
            filter.insert(hash_key(KIND_CLASS, class));
        }
    }
    if let Some(attributes) = doc.attributes(node) {
        for name in attributes.keys() {
            filter.insert(hash_key(KIND_ATTR, name.cow_to_lowercase().as_ref()));
        }
    }
}

/// The keys some ancestor of the subject must carry for this complex selector to match.
///
/// In `C0 op0 C1 op1 ... Cn`, with `Cn` the subject, a compound `Ck` is an ancestor of the
/// subject exactly when `op_k` - the combinator immediately to its *right* - is a descendant or
/// child combinator. Every compound left of the subject either is an ancestor of the compound
/// next to it or is that compound's sibling, and siblings share their ancestors, so being an
/// ancestor of `C(k+1)` is the same as being an ancestor of the subject. `.a ~ .b .c` therefore
/// requires `.b` and not `.a`, while `.a .b ~ .c` requires `.a` and not `.b`.
///
/// An empty result means the selector places no condition the filter can check, so it is always
/// run.
pub(crate) fn ancestor_keys(complex: &[CssSelectorPart]) -> Box<[u32]> {
    // `::slotted()` leaves the ancestor chain: the walk jumps to the slot the element was
    // projected into, which lives in the shadow tree rather than above the element. `:host`
    // reaches the tree's host, which sits outside the chain as well. Neither is summarised
    // here, and a compound to their left is matched against something the filter never saw,
    // so a selector using either is never filtered.
    if complex
        .iter()
        .any(|part| matches!(part, CssSelectorPart::Host(_) | CssSelectorPart::Slotted(_)))
    {
        return Box::default();
    }

    let mut keys = Vec::new();
    let mut start = 0;
    for (i, part) in complex.iter().enumerate() {
        let CssSelectorPart::Combinator(combinator) = part else {
            continue;
        };
        if matches!(combinator, Combinator::Descendant | Combinator::Child) {
            if let Some(compound) = complex.get(start..i) {
                compound_keys(compound, &mut keys);
            }
        }
        start = i + 1;
    }
    keys.into_boxed_slice()
}

/// The positive simple selectors of one ancestor compound, hashed the way the element side of
/// the filter hashes what it finds on the ancestor.
fn compound_keys(compound: &[CssSelectorPart], keys: &mut Vec<u32>) {
    for part in compound {
        match part {
            // Compared exactly by the matcher (`v == name`, `has_class`, `t == name`), so
            // hashed exactly here.
            CssSelectorPart::Id(name) => keys.push(hash_key(KIND_ID, name)),
            CssSelectorPart::Class(name) => keys.push(hash_key(KIND_CLASS, name)),
            CssSelectorPart::Type(name) => keys.push(hash_key(KIND_TAG, name)),
            // Every attribute matcher first asks the element for the attribute and fails when
            // it has none, so the name alone is a requirement. Lowercased on both sides, as the
            // selector index lowercases it: the matcher's own lookup is case-sensitive, and
            // folding case can only put two names in one bucket, never split one in two.
            CssSelectorPart::Attribute(selector) => {
                keys.push(hash_key(KIND_ATTR, selector.name.cow_to_lowercase().as_ref()));
            }
            // Everything else names nothing the ancestor has to carry. `*` and a pseudo-class
            // put no name on it at all, a pseudo-element is not the element, and `:not()` is a
            // negation - a name it mentions is one the ancestor must *not* have, which must
            // never become a requirement.
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stylesheet::{AttributeSelector, MatcherType};

    fn attribute(name: &str) -> CssSelectorPart {
        CssSelectorPart::Attribute(Box::new(AttributeSelector {
            name: name.to_string(),
            matcher: MatcherType::None,
            value: String::new(),
            case_insensitive: false,
        }))
    }

    fn filter_of(keys: &[(u8, &str)]) -> AncestorFilter {
        let mut filter = AncestorFilter::new();
        for (kind, name) in keys {
            filter.insert(hash_key(*kind, name));
        }
        filter
    }

    #[test]
    fn a_sibling_of_an_ancestor_is_not_required() {
        use CssSelectorPart::*;
        // `.a ~ .b .c`: only `.b` is an ancestor of `.c`. `.a` is a sibling of `.b`, so it
        // sits beside an ancestor rather than above the subject.
        let keys = ancestor_keys(&[
            Class("a".into()),
            Combinator(crate::stylesheet::Combinator::SubsequentSibling),
            Class("b".into()),
            Combinator(crate::stylesheet::Combinator::Descendant),
            Class("c".into()),
        ]);
        assert_eq!(*keys, [hash_key(KIND_CLASS, "b")]);
    }

    #[test]
    fn an_ancestor_of_a_sibling_is_required() {
        use CssSelectorPart::*;
        // `.a .b ~ .c`: `.b` and `.c` share a parent, so anything above `.b` is above `.c`.
        // `.a` is required and `.b` is not.
        let keys = ancestor_keys(&[
            Class("a".into()),
            Combinator(crate::stylesheet::Combinator::Descendant),
            Class("b".into()),
            Combinator(crate::stylesheet::Combinator::SubsequentSibling),
            Class("c".into()),
        ]);
        assert_eq!(*keys, [hash_key(KIND_CLASS, "a")]);
    }

    #[test]
    fn a_child_combinator_makes_an_ancestor_too() {
        use CssSelectorPart::*;
        // `.a > .b ~ .c .d`: `.a` is `.b`'s parent and `.c` is `.b`'s sibling, so `.a` is above
        // `.c` and `.c` is above `.d`. Both are required; `.b` is not.
        let keys = ancestor_keys(&[
            Class("a".into()),
            Combinator(crate::stylesheet::Combinator::Child),
            Class("b".into()),
            Combinator(crate::stylesheet::Combinator::SubsequentSibling),
            Class("c".into()),
            Combinator(crate::stylesheet::Combinator::Descendant),
            Class("d".into()),
        ]);
        assert_eq!(*keys, [hash_key(KIND_CLASS, "a"), hash_key(KIND_CLASS, "c")]);
    }

    #[test]
    fn the_subject_compound_is_never_a_key() {
        use CssSelectorPart::*;
        // The element's own id, class and tag are not in the filter, so requiring them would
        // reject every selector that has no combinator at all.
        assert!(ancestor_keys(&[Id("x".into()), Class("y".into()), Type("div".into())]).is_empty());
    }

    #[test]
    fn a_negation_contributes_no_key() {
        use CssSelectorPart::*;
        // `:not(.x).y .z` requires an ancestor with `.y`, and requires one *without* `.x` -
        // which is not something the filter can be asked, and must not turn into a requirement
        // that `.x` be present.
        let keys = ancestor_keys(&[
            Not(vec![vec![Class("x".into())]]),
            Class("y".into()),
            Combinator(crate::stylesheet::Combinator::Descendant),
            Class("z".into()),
        ]);
        assert_eq!(*keys, [hash_key(KIND_CLASS, "y")]);
    }

    #[test]
    fn an_ancestor_compound_contributes_every_positive_part() {
        use CssSelectorPart::*;
        // Id, class, tag and attribute name all become keys; `*` and a pseudo-class do not.
        let keys = ancestor_keys(&[
            Universal,
            Id("nav".into()),
            Class("open".into()),
            Type("div".into()),
            attribute("DATA-X"),
            PseudoClass("hover".into()),
            Combinator(crate::stylesheet::Combinator::Child),
            Class("item".into()),
        ]);
        assert_eq!(
            *keys,
            [
                hash_key(KIND_ID, "nav"),
                hash_key(KIND_CLASS, "open"),
                hash_key(KIND_TAG, "div"),
                // Lowercased, as the selector index lowercases an attribute name.
                hash_key(KIND_ATTR, "data-x"),
            ]
        );
    }

    #[test]
    fn a_namespace_prefix_is_not_a_tag_name() {
        use CssSelectorPart::*;
        // `ns|div .foo`: the namespace combinator consumes the part on its left, so `ns` is a
        // namespace rather than a compound of its own. Only `div` is an ancestor requirement.
        let keys = ancestor_keys(&[
            Type("ns".into()),
            Combinator(crate::stylesheet::Combinator::Namespace),
            Type("div".into()),
            Combinator(crate::stylesheet::Combinator::Descendant),
            Class("foo".into()),
        ]);
        assert_eq!(*keys, [hash_key(KIND_TAG, "div")]);
    }

    #[test]
    fn shadow_crossing_selectors_are_never_filtered() {
        use CssSelectorPart::*;
        // `::slotted()` and `:host` both match against something the ancestor walk never
        // reaches, so nothing about them may be turned into a requirement.
        assert!(ancestor_keys(&[
            Class("a".into()),
            Combinator(crate::stylesheet::Combinator::Descendant),
            Type("slot".into()),
            Slotted(vec![vec![Type("p".into())]]),
        ])
        .is_empty());
        assert!(ancestor_keys(&[
            Host(Some(vec![vec![Class("a".into())]])),
            Combinator(crate::stylesheet::Combinator::Descendant),
            Class("b".into()),
        ])
        .is_empty());
    }

    #[test]
    fn a_key_the_filter_never_saw_is_rejected() {
        let filter = filter_of(&[(KIND_CLASS, "wrapper"), (KIND_TAG, "body"), (KIND_ID, "main")]);
        assert!(filter.may_match(&[hash_key(KIND_CLASS, "wrapper")]));
        assert!(filter.may_match(&[hash_key(KIND_CLASS, "wrapper"), hash_key(KIND_TAG, "body")]));
        assert!(!filter.may_match(&[hash_key(KIND_CLASS, "sidebar")]));
        // Present, but as a class rather than as a tag name.
        assert!(!filter.may_match(&[hash_key(KIND_TAG, "wrapper")]));
        // One missing key out of several is enough.
        assert!(!filter.may_match(&[hash_key(KIND_ID, "main"), hash_key(KIND_CLASS, "sidebar")]));
        // No conditions at all: always run.
        assert!(filter.may_match(&[]));
    }
}
