use gosub_css3::stylesheet::CssValue;
use crate::syntax::{GroupCombinators, SyntaxComponent, SyntaxComponentMultiplier, SyntaxComponentType};

/// A CSS Syntax Tree is a tree sof CSS syntax components that can be used to match against CSS values.
#[derive(Clone, Debug, PartialEq)]
pub struct CssSyntaxTree {
    /// The components of the syntax tree
    pub components: Vec<SyntaxComponent>,
}

impl CssSyntaxTree {
    /// Creates a new CSS Syntax Tree
    pub fn new(components: Vec<SyntaxComponent>) -> Self {
        CssSyntaxTree { components }
    }

    /// Matches a CSS value (or set of values) against the syntax tree. Will return a normalized version of the value(s) if it matches.
    pub fn matches(&self, value: &CssValue) -> Option<CssValue> {
        if self.components.len() != 1 {
            panic!("Syntax tree must have exactly one root component");
        }

        match_internal(value, &self.components[0])
    }

    /// Matches a CSS value against the syntax tree, but the CSS value is provided as a (unparsed) string
    pub fn matches_str(&self, value: &str) -> Option<CssValue> {
        match CssValue::parse_str(value) {
            Ok(value) => self.matches(&value),
            Err(_) => None,
        }
    }
}

fn match_internal(value: &CssValue, component: &SyntaxComponent) -> Option<CssValue> {
    // dbg!(&component);
    dbg!(&value);

    match &component.component {
        SyntaxComponentType::GenericKeyword(keyword) => {
            print!("Matching keyword: {}", keyword);
            print!(" with value: {:?}", value);
            match value {
                CssValue::None => {
                    if keyword == "none" {
                        return Some(value.clone());
                    }
                }
                CssValue::String(v) => {
                    if v == keyword {
                        println!("Matched keyword {}", v);
                        return Some(value.clone());
                    }
                }
                _ => {}
            }

            return None;
        }
        SyntaxComponentType::Property(_s) => {}
        SyntaxComponentType::Function(_s, _t) => {}
        SyntaxComponentType::TypeDefinition(_s, _t, _u) => {}
        SyntaxComponentType::Inherit => {}
        SyntaxComponentType::Initial => {}
        SyntaxComponentType::Unset => {}
        SyntaxComponentType::Literal(_s) => {}
        SyntaxComponentType::Value(_s) => {}
        SyntaxComponentType::Unit(_s, _t, _u) => {}
        SyntaxComponentType::Group(group) => {
            let mut elements: Vec<CssValue> = vec![];

            for c in group.components.iter() {
                match match_internal(value, c) {
                    Some(val) => {
                        // @TODO: check combinator
                        elements.push(val.clone());
                    }
                    None => {
                        // not a valid element
                    }
                }
            }

            println!("matching combinator: {:?}", group.combinator);
            dbg!(&elements);
            match group.combinator {
                GroupCombinators::Juxtaposition => {
                    if group.components.len() == group.components.len() {
                        // Check the ordering
                        for (_i, _c) in group.components.iter().enumerate() {
                            // match_internal(value[i], c)?;
                            // if ! in_order() {
                            //     return Err("Incorrect order of values".to_string());
                            // }
                        }
                    }

                }
                GroupCombinators::AllAnyOrder => {
                    if elements.len() == group.components.len() {
                        return Some(elements[0].clone());
                    }
                }
                GroupCombinators::AtLeastOneAnyOrder => {
                    if elements.len() >= 1 {
                        return Some(elements[0].clone());
                    }
                }
                GroupCombinators::ExactlyOne => {
                    if elements.len() == 1 {
                        return Some(elements[0].clone());
                    }
                }
            }
        }
    }

    match component.multipliers {
        SyntaxComponentMultiplier::Once => {}
        SyntaxComponentMultiplier::ZeroOrMore => {}
        SyntaxComponentMultiplier::OneOrMore => {}
        SyntaxComponentMultiplier::Optional => {}
        SyntaxComponentMultiplier::Between(_n, _m) => {}
        SyntaxComponentMultiplier::AtLeastOneValue => {}
        SyntaxComponentMultiplier::CommaSeparatedRepeat(_n, _m) => {}
    }

    return None;
}

#[cfg(test)]
mod tests {
    use gosub_css3::stylesheet::CssValue;
    use crate::syntax::CssSyntax;

    macro_rules! str {
        ($s:expr) => {
            CssValue::String($s.to_string())
        };
    }

    #[test]
    fn test_simple_group() {
        let tree = CssSyntax::new("auto | none | block").compile().unwrap();
        assert!(tree.matches(&str!("auto")).is_some());
        assert!(tree.matches(&CssValue::None).is_some());
        assert!(tree.matches(&str!("block")).is_some());
        assert!(tree.matches(&str!("inline")).is_none());
        assert!(tree.matches(&str!("")).is_none());
        assert!(tree.matches(&str!("foobar")).is_none());
        assert!(tree.matches(&CssValue::List(vec![str!("auto"), CssValue::None])).is_none());
        assert!(tree.matches(&CssValue::List(vec![str!("auto"), CssValue::Comma, str!("none")])).is_none());
        assert!(tree.matches(&CssValue::List(vec![str!("auto"), CssValue::Comma, CssValue::None, CssValue::Comma, str!("block") ])).is_none());
    }

    #[test]
    fn test_double_group() {
        let tree = CssSyntax::new("auto none block").compile().unwrap();
        dbg!(&tree);
        // assert!(tree.matches(&str!("auto")).is_none());
        // assert!(tree.matches(&CssValue::None).is_none());
        // assert!(tree.matches(&str!("block")).is_none());
        assert!(tree.matches(&CssValue::List(vec![
            str!("auto"),
            CssValue::None,
            str!("block"),
        ])).is_some());
        assert!(tree.matches(&CssValue::List(vec![
            str!("block"),
            CssValue::None,
            str!("block"),
        ])).is_none());
        assert!(tree.matches(&CssValue::List(vec![
            str!("auto"),
            CssValue::None,
            str!("auto"),
        ])).is_none());
    }

}