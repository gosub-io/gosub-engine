//! Rule index: which rules of a stylesheet can possibly match an element.
//!
//! A complex selector matches an element only if its rightmost compound matches that element
//! itself, so every selector is bucketed by the most selective simple selector in that compound
//! (id, else class, else type, else attribute name, else universal). Style computation then
//! tests only the rules in the buckets an element falls into instead of every rule in the
//! sheet. The buckets are a superset filter: the full matcher still decides.
//!
//! The buckets are split once more, by pseudo-element. A selector carrying `::before` can only
//! apply when the cascade is asked for that pseudo-element, and a selector carrying none can
//! never apply when it is - which is exactly what [`match_selector`] does with its `pseudo`
//! argument. Keeping the two apart means a plain element never looks at the sheet's
//! pseudo-element rules, of which a real-world sheet has thousands.
//!
//! [`match_selector`]: crate::matcher::styling

use crate::stylesheet::{CssRule, CssSelectorPart};
use cow_utils::CowUtils as _;
use std::collections::HashMap;

/// How many bucket slices the merge keeps on the stack. An element falls into one bucket per
/// class, one per attribute and three more, so this is far above what any page reaches; past
/// it the merge falls back to appending and sorting, which answers the same list.
const MAX_SOURCES: usize = 32;

#[derive(Debug, Default, PartialEq)]
pub struct SelectorIndex {
    /// Selectors carrying no pseudo-element - the only ones an element itself can match.
    normal: Buckets,
    /// Selectors carrying a pseudo-element, keyed by its ASCII-lowercased name. A selector
    /// naming two of them is filed under both.
    pseudo: HashMap<String, Buckets>,
    /// Number of rules indexed; a stylesheet whose rule count differs has a stale index.
    rule_count: usize,
}

/// One set of buckets: the rules of a sheet, filed by what their rightmost compound needs.
#[derive(Debug, Default, PartialEq)]
struct Buckets {
    by_id: HashMap<String, Vec<usize>>,
    by_class: HashMap<String, Vec<usize>>,
    by_tag: HashMap<String, Vec<usize>>,
    /// Keyed by attribute name, lowercased on both sides so the bucket can only ever be
    /// wider than the matcher's own (case-sensitive) attribute lookup.
    by_attr: HashMap<String, Vec<usize>>,
    universal: Vec<usize>,
}

/// The element-side keys an index lookup needs.
pub struct ElementKeys<'a> {
    pub id: Option<&'a str>,
    pub classes: &'a str,
    pub tag: Option<&'a str>,
    /// The element's attributes. Only the names are read, for the attribute buckets.
    pub attributes: Option<&'a HashMap<String, String>>,
    /// The pseudo-element the cascade is being asked for, if any.
    pub pseudo: Option<&'a str>,
}

impl SelectorIndex {
    /// Bucket every selector of `rules` by its rightmost compound and its pseudo-element.
    pub fn build(rules: &[CssRule]) -> Self {
        let mut index = Self {
            rule_count: rules.len(),
            ..Self::default()
        };
        for (rule_idx, rule) in rules.iter().enumerate() {
            for selector in &rule.selectors {
                for complex in selector.complexes() {
                    let key = rightmost_key(complex);
                    let mut has_pseudo = false;
                    for part in complex {
                        if let CssSelectorPart::PseudoElement(name) = part {
                            has_pseudo = true;
                            index
                                .pseudo
                                .entry(name.cow_to_lowercase().into_owned())
                                .or_default()
                                .insert(&key, rule_idx);
                        }
                    }
                    if !has_pseudo {
                        index.normal.insert(&key, rule_idx);
                    }
                }
            }
        }
        index
    }

    /// How many rules this index was built from.
    #[must_use]
    pub fn rule_count(&self) -> usize {
        self.rule_count
    }

    /// Write the rule indices that may match the element into `out`, ascending and unique (so
    /// the cascade sees rules in stylesheet order, exactly as a full scan would). `out` is the
    /// caller's buffer so that a lookup costs no allocation; whatever it held is discarded.
    pub fn candidates(&self, keys: &ElementKeys<'_>, out: &mut Vec<usize>) {
        out.clear();
        let buckets = match keys.pseudo {
            Some(pseudo) => {
                // Nothing in the sheet names this pseudo-element, so nothing can style it.
                let Some(buckets) = self.pseudo.get(pseudo.cow_to_lowercase().as_ref()) else {
                    return;
                };
                buckets
            }
            None => &self.normal,
        };
        buckets.gather(keys, out);
    }
}

impl Buckets {
    fn insert(&mut self, key: &Key<'_>, rule_idx: usize) {
        let bucket = match key {
            Key::Id(name) => self.by_id.entry((*name).to_string()).or_default(),
            Key::Class(name) => self.by_class.entry((*name).to_string()).or_default(),
            Key::Tag(name) => self.by_tag.entry((*name).to_string()).or_default(),
            Key::Attr(name) => self.by_attr.entry(name.to_string()).or_default(),
            Key::Universal => &mut self.universal,
        };
        push_unique(bucket, rule_idx);
    }

    /// Merge the buckets this element falls into, in one pass.
    ///
    /// Every bucket is built by walking the rules in order, so each one is already ascending
    /// and free of repeats; a k-way merge of them therefore produces the ascending, repeat-free
    /// union the cascade needs without ever sorting. A rule can sit in two buckets (one per
    /// selector of a list), so emitting a value steps every source that held it.
    fn gather(&self, keys: &ElementKeys<'_>, out: &mut Vec<usize>) {
        let id = keys.id.and_then(|id| self.by_id.get(id));
        let tag = keys.tag.and_then(|tag| self.by_tag.get(tag));
        let classes = keys
            .classes
            .split_ascii_whitespace()
            .filter_map(|class| self.by_class.get(class));
        // Skipped entirely for the usual sheet that keys nothing by attribute, which saves
        // walking the element's attributes at all.
        let attrs = keys
            .attributes
            .filter(|_| !self.by_attr.is_empty())
            .into_iter()
            .flat_map(HashMap::keys)
            .filter_map(|name| self.by_attr.get(name.cow_to_lowercase().as_ref()));
        let mut lists = id
            .into_iter()
            .chain(classes)
            .chain(tag)
            .chain(attrs)
            .chain(std::iter::once(&self.universal))
            .filter(|bucket| !bucket.is_empty());

        let mut sources: [&[usize]; MAX_SOURCES] = [&[]; MAX_SOURCES];
        let mut len = 0;
        while let Some(bucket) = lists.next() {
            if len == MAX_SOURCES {
                // More buckets than the stack holds. Append everything and sort instead: the
                // answer is the same list, and no real page gets here.
                out.clear();
                for source in &sources[..len] {
                    out.extend_from_slice(source);
                }
                out.extend_from_slice(bucket);
                for rest in lists {
                    out.extend_from_slice(rest);
                }
                out.sort_unstable();
                out.dedup();
                return;
            }
            if let Some(slot) = sources.get_mut(len) {
                *slot = bucket;
            }
            len += 1;
        }

        while len > 0 {
            let mut min = usize::MAX;
            for source in &sources[..len] {
                if let Some(&head) = source.first() {
                    min = min.min(head);
                }
            }
            out.push(min);

            let mut i = 0;
            while i < len {
                let Some(source) = sources.get_mut(i) else {
                    break;
                };
                if let Some((&head, rest)) = source.split_first() {
                    if head == min {
                        *source = rest;
                        if rest.is_empty() {
                            // Exhausted: pull the last source into its place and shrink.
                            sources.swap(i, len - 1);
                            len -= 1;
                            continue;
                        }
                    }
                }
                i += 1;
            }
        }
    }
}

enum Key<'a> {
    Id(&'a str),
    Class(&'a str),
    Tag(&'a str),
    /// An attribute name, already lowercased.
    Attr(std::borrow::Cow<'a, str>),
    Universal,
}

/// The most selective simple selector of the rightmost compound (everything after the last
/// combinator). Anything unexpected degrades to the universal bucket, never to a miss.
fn rightmost_key(complex: &[CssSelectorPart]) -> Key<'_> {
    let start = complex
        .iter()
        .rposition(|p| matches!(p, CssSelectorPart::Combinator(_)))
        .map_or(0, |i| i + 1);
    let compound = &complex[start..];

    // `::slotted()` breaks the assumption this index rests on: the compound's other simple
    // selectors describe the *slot*, while the element being matched is the light-DOM node
    // projected into it. Indexing `slot[name=x]::slotted(*)` under the tag `slot` would file it
    // against an element it can never be tested on, so it goes in the universal bucket - where
    // its own argument still gets checked by the full matcher.
    if compound.iter().any(|p| matches!(p, CssSelectorPart::Slotted(_))) {
        return Key::Universal;
    }

    let mut class = None;
    let mut tag = None;
    let mut attr = None;
    for part in compound {
        match part {
            CssSelectorPart::Id(name) => return Key::Id(name),
            CssSelectorPart::Class(name) if class.is_none() => class = Some(name.as_str()),
            CssSelectorPart::Type(name) if tag.is_none() => tag = Some(name.as_str()),
            // Every attribute matcher, `[a]` and `[a=b]` alike, first asks the element for the
            // attribute and fails when it has none - so requiring it is a superset of what the
            // matcher accepts. A compound requires all of its parts, so any one of several
            // attributes will do; the first keeps the choice deterministic.
            CssSelectorPart::Attribute(selector) if attr.is_none() => attr = Some(selector.name.as_str()),
            _ => {}
        }
    }
    if let Some(name) = class {
        return Key::Class(name);
    }
    if let Some(name) = tag {
        return Key::Tag(name);
    }
    if let Some(name) = attr {
        return Key::Attr(name.cow_to_lowercase());
    }
    Key::Universal
}

fn push_unique(bucket: &mut Vec<usize>, rule_idx: usize) {
    if bucket.last() != Some(&rule_idx) {
        bucket.push(rule_idx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stylesheet::{AttributeSelector, CssSelector, MatcherType};

    fn rule(selectors: Vec<Vec<CssSelectorPart>>) -> CssRule {
        CssRule::new(vec![CssSelector::new(selectors)], vec![], None, None)
    }

    fn attribute(name: &str) -> CssSelectorPart {
        CssSelectorPart::Attribute(Box::new(AttributeSelector {
            name: name.to_string(),
            matcher: MatcherType::None,
            value: String::new(),
            case_insensitive: false,
        }))
    }

    fn keys<'a>(id: Option<&'a str>, classes: &'a str, tag: Option<&'a str>) -> ElementKeys<'a> {
        ElementKeys {
            id,
            classes,
            tag,
            attributes: None,
            pseudo: None,
        }
    }

    fn candidates(index: &SelectorIndex, keys: &ElementKeys<'_>) -> Vec<usize> {
        let mut out = Vec::new();
        index.candidates(keys, &mut out);
        out
    }

    #[test]
    fn buckets_by_rightmost_compound() {
        use CssSelectorPart::*;
        let rules = vec![
            rule(vec![vec![Type("div".into())]]),                     // 0
            rule(vec![vec![Id("nav".into()), Class("open".into())]]), // 1: id wins
            rule(vec![vec![
                Type("ul".into()),
                Combinator(crate::stylesheet::Combinator::Child),
                Class("item".into()),
            ]]), // 2
            rule(vec![vec![Universal]]),                              // 3
            rule(vec![vec![Class("a".into())], vec![Type("p".into())]]), // 4: two selectors
            rule(vec![vec![Type("a".into()), PseudoClass("hover".into())]]), // 5
            rule(vec![vec![PseudoElement("before".into())]]),         // 6: pseudo-element only
        ];
        let index = SelectorIndex::build(&rules);

        // Rule 6 carries `::before`, so no plain element sees it.
        assert_eq!(candidates(&index, &keys(None, "", Some("div"))), vec![0, 3]);
        assert_eq!(candidates(&index, &keys(None, "item  a", Some("li"))), vec![2, 3, 4]);
        assert_eq!(candidates(&index, &keys(Some("nav"), "open", Some("nav"))), vec![1, 3]);
        assert_eq!(candidates(&index, &keys(None, "", Some("p"))), vec![3, 4]);
        assert_eq!(candidates(&index, &keys(None, "", Some("a"))), vec![3, 5]);
    }

    #[test]
    fn merge_is_ascending_and_free_of_repeats() {
        use CssSelectorPart::*;
        // Every rule reaches the same element through a different bucket, and rules 1 and 3
        // reach it through two at once - the case a merge has to collapse.
        let rules = vec![
            rule(vec![vec![Universal]]),                                 // 0: universal
            rule(vec![vec![Type("p".into())], vec![Class("a".into())]]), // 1: tag and class
            rule(vec![vec![Class("b".into())]]),                         // 2: class
            rule(vec![vec![Id("x".into())], vec![Class("a".into())]]),   // 3: id and class
            rule(vec![vec![Universal]]),                                 // 4: universal
            rule(vec![vec![Type("p".into())]]),                          // 5: tag
        ];
        let index = SelectorIndex::build(&rules);
        let found = candidates(&index, &keys(Some("x"), "a b", Some("p")));
        assert_eq!(found, vec![0, 1, 2, 3, 4, 5]);

        // The buffer is the caller's, and a lookup starts from whatever it holds.
        let mut out = vec![99, 98];
        index.candidates(&keys(None, "", Some("p")), &mut out);
        assert_eq!(out, vec![0, 1, 4, 5]);
    }

    #[test]
    fn pseudo_element_rules_are_kept_apart() {
        use CssSelectorPart::*;
        let rules = vec![
            rule(vec![vec![Class("c".into())]]),                                 // 0: plain
            rule(vec![vec![Class("c".into()), PseudoElement("before".into())]]), // 1: ::before
            rule(vec![vec![Class("c".into()), PseudoElement("After".into())]]),  // 2: ::after
            rule(vec![vec![Universal, PseudoElement("before".into())]]),         // 3: *::before
            rule(vec![vec![Type("li".into()), PseudoElement("marker".into())]]), // 4: ::marker
        ];
        let index = SelectorIndex::build(&rules);

        // The element itself sees only the rule that names no pseudo-element.
        assert_eq!(candidates(&index, &keys(None, "c", Some("li"))), vec![0]);

        let before = ElementKeys {
            pseudo: Some("before"),
            ..keys(None, "c", Some("li"))
        };
        assert_eq!(candidates(&index, &before), vec![1, 3]);

        // The name is matched case-insensitively, as `match_selector` matches it.
        let after = ElementKeys {
            pseudo: Some("after"),
            ..keys(None, "c", Some("li"))
        };
        assert_eq!(candidates(&index, &after), vec![2]);

        let marker = ElementKeys {
            pseudo: Some("marker"),
            ..keys(None, "c", Some("li"))
        };
        assert_eq!(candidates(&index, &marker), vec![4]);

        // A pseudo-element the sheet never names has nothing at all.
        let selection = ElementKeys {
            pseudo: Some("selection"),
            ..keys(None, "c", Some("li"))
        };
        assert!(candidates(&index, &selection).is_empty());
    }

    #[test]
    fn pseudo_element_inside_not_stays_with_the_element() {
        use CssSelectorPart::*;
        // `:not(::before)` is not a pseudo-element selector: the matcher tests the negation
        // against the element itself, so the rule has to keep reaching plain elements.
        let rules = vec![rule(vec![vec![
            Type("p".into()),
            Not(vec![vec![PseudoElement("before".into())]]),
        ]])];
        let index = SelectorIndex::build(&rules);
        assert_eq!(candidates(&index, &keys(None, "", Some("p"))), vec![0]);
    }

    #[test]
    fn attribute_rules_reach_only_elements_carrying_the_attribute() {
        use CssSelectorPart::*;
        let rules = vec![
            rule(vec![vec![attribute("data-x")]]),                   // 0
            rule(vec![vec![attribute("DATA-Y")]]),                   // 1: name cased oddly
            rule(vec![vec![Type("p".into()), attribute("data-x")]]), // 2: tag is more selective
        ];
        let index = SelectorIndex::build(&rules);

        let mut with = HashMap::new();
        with.insert("data-x".to_string(), String::new());
        let mut without = HashMap::new();
        without.insert("href".to_string(), String::new());

        let has_attr = ElementKeys {
            attributes: Some(&with),
            ..keys(None, "", Some("div"))
        };
        assert_eq!(candidates(&index, &has_attr), vec![0]);

        let no_attr = ElementKeys {
            attributes: Some(&without),
            ..keys(None, "", Some("div"))
        };
        assert!(candidates(&index, &no_attr).is_empty());

        // Both sides are lowercased, so an oddly cased selector still reaches the element.
        let mut upper = HashMap::new();
        upper.insert("data-y".to_string(), String::new());
        let has_upper = ElementKeys {
            attributes: Some(&upper),
            ..keys(None, "", Some("div"))
        };
        assert_eq!(candidates(&index, &has_upper), vec![1]);

        // Rule 2 is filed under its tag, so it reaches a `<p>` with or without the attribute.
        assert_eq!(candidates(&index, &keys(None, "", Some("p"))), vec![2]);
    }

    #[test]
    fn a_universal_rule_still_reaches_every_element() {
        use CssSelectorPart::*;
        // The shapes `rightmost_key` refuses to key: `::slotted()`, a bare pseudo-class, and
        // a compound of nothing but a negation.
        let rules = vec![
            rule(vec![vec![Type("slot".into()), Slotted(vec![vec![Universal]])]]), // 0
            rule(vec![vec![PseudoClass("hover".into())]]),                         // 1
            rule(vec![vec![Not(vec![vec![Class("x".into())]])]]),                  // 2
        ];
        let index = SelectorIndex::build(&rules);
        for element in [
            keys(None, "", Some("div")),
            keys(Some("main"), "a b c", Some("section")),
            keys(None, "x", None),
        ] {
            assert_eq!(candidates(&index, &element), vec![0, 1, 2]);
        }
    }
}
