//! Rule index: which rules of a stylesheet can possibly match an element.
//!
//! A complex selector matches an element only if its rightmost compound matches that element
//! itself, so every selector is bucketed by the most selective simple selector in that compound
//! (id, else class, else type, else universal). Style computation then tests only the rules in
//! the buckets an element falls into instead of every rule in the sheet. The buckets are a
//! superset filter: the full matcher still decides.

use crate::stylesheet::{CssRule, CssSelectorPart};
use std::collections::HashMap;

#[derive(Debug, Default, PartialEq)]
pub struct SelectorIndex {
    by_id: HashMap<String, Vec<usize>>,
    by_class: HashMap<String, Vec<usize>>,
    by_tag: HashMap<String, Vec<usize>>,
    universal: Vec<usize>,
    /// Number of rules indexed; a stylesheet whose rule count differs has a stale index.
    rule_count: usize,
}

/// The element-side keys an index lookup needs.
pub struct ElementKeys<'a> {
    pub id: Option<&'a str>,
    pub classes: &'a str,
    pub tag: Option<&'a str>,
}

impl SelectorIndex {
    /// Bucket every selector of `rules` by its rightmost compound.
    pub fn build(rules: &[CssRule]) -> Self {
        let mut index = Self {
            rule_count: rules.len(),
            ..Self::default()
        };
        for (rule_idx, rule) in rules.iter().enumerate() {
            for selector in &rule.selectors {
                for complex in &selector.parts {
                    match rightmost_key(complex) {
                        Key::Id(name) => push_unique(index.by_id.entry(name.to_string()).or_default(), rule_idx),
                        Key::Class(name) => push_unique(index.by_class.entry(name.to_string()).or_default(), rule_idx),
                        Key::Tag(name) => push_unique(index.by_tag.entry(name.to_string()).or_default(), rule_idx),
                        Key::Universal => push_unique(&mut index.universal, rule_idx),
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

    /// Rule indices that may match the element, ascending and unique (so the cascade sees
    /// rules in stylesheet order, exactly as a full scan would).
    pub fn candidates(&self, keys: &ElementKeys<'_>) -> Vec<usize> {
        let mut out: Vec<usize> = self.universal.clone();
        if let Some(rules) = keys.id.and_then(|id| self.by_id.get(id)) {
            out.extend_from_slice(rules);
        }
        for class in keys.classes.split_ascii_whitespace() {
            if let Some(rules) = self.by_class.get(class) {
                out.extend_from_slice(rules);
            }
        }
        if let Some(rules) = keys.tag.and_then(|tag| self.by_tag.get(tag)) {
            out.extend_from_slice(rules);
        }
        out.sort_unstable();
        out.dedup();
        out
    }
}

enum Key<'a> {
    Id(&'a str),
    Class(&'a str),
    Tag(&'a str),
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

    let mut class = None;
    let mut tag = None;
    for part in compound {
        match part {
            CssSelectorPart::Id(name) => return Key::Id(name),
            CssSelectorPart::Class(name) if class.is_none() => class = Some(name.as_str()),
            CssSelectorPart::Type(name) if tag.is_none() => tag = Some(name.as_str()),
            _ => {}
        }
    }
    if let Some(name) = class {
        return Key::Class(name);
    }
    if let Some(name) = tag {
        return Key::Tag(name);
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
    use crate::stylesheet::CssSelector;

    fn rule(selectors: Vec<Vec<CssSelectorPart>>) -> CssRule {
        CssRule {
            selectors: vec![CssSelector { parts: selectors }],
            declarations: vec![],
        }
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
            rule(vec![vec![PseudoElement("before".into())]]),         // 6: universal
        ];
        let index = SelectorIndex::build(&rules);

        let div = ElementKeys {
            id: None,
            classes: "",
            tag: Some("div"),
        };
        assert_eq!(index.candidates(&div), vec![0, 3, 6]);

        let item = ElementKeys {
            id: None,
            classes: "item  a",
            tag: Some("li"),
        };
        assert_eq!(index.candidates(&item), vec![2, 3, 4, 6]);

        let nav = ElementKeys {
            id: Some("nav"),
            classes: "open",
            tag: Some("nav"),
        };
        assert_eq!(index.candidates(&nav), vec![1, 3, 6]);

        let p = ElementKeys {
            id: None,
            classes: "",
            tag: Some("p"),
        };
        assert_eq!(index.candidates(&p), vec![3, 4, 6]);

        let a = ElementKeys {
            id: None,
            classes: "",
            tag: Some("a"),
        };
        assert_eq!(index.candidates(&a), vec![3, 5, 6]);
    }
}
