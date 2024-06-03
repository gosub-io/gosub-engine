use gosub_css3::stylesheet::CssValue;
use crate::syntax::{Group, GroupCombinators, SyntaxComponent, SyntaxComponentType};

/// A CSS Syntax Tree is a tree sof CSS syntax components that can be used to match against CSS values.
#[derive(Clone, Debug, PartialEq)]
pub struct CssSyntaxTree {
    /// The components of the syntax tree
    pub components: Vec<SyntaxComponent>,
}

impl CssSyntaxTree {
    /// Creates a new CSS Syntax tree from the given components
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
}

fn match_internal(value: &CssValue, component: &SyntaxComponent) -> Option<CssValue> {
    return match &component.component {
        SyntaxComponentType::GenericKeyword(keyword) => match value {
            CssValue::None if keyword.eq_ignore_ascii_case("none") => Some(value.clone()),
            CssValue::String(v) if v.eq_ignore_ascii_case(&keyword) => Some(value.clone()),
            _ => None,
        }
        // SyntaxComponentType::Property(_s) => {},
        // SyntaxComponentType::Function(_s, _t) => {}
        // SyntaxComponentType::TypeDefinition(_s, _t, _u) => {}
        SyntaxComponentType::Inherit => return match value {
           CssValue::String(v) if v.eq_ignore_ascii_case("inherit") => Some(value.clone()),
            _ => None,
        },
        SyntaxComponentType::Initial => return match value {
            CssValue::String(v) if v.eq_ignore_ascii_case("initial") => Some(value.clone()),
            _ => None,
        },
        SyntaxComponentType::Unset => return match value {
            CssValue::String(v) if v.eq_ignore_ascii_case("unset") => Some(value.clone()),
            _ => None,
        },
        // SyntaxComponentType::Literal(_s) => {}
        // SyntaxComponentType::Value(_s) => {}
        // SyntaxComponentType::Unit(_s, _t, _u) => {}
        SyntaxComponentType::Group(group) => {
            return match group.combinator {
                GroupCombinators::Juxtaposition => {
                    match_group_juxtaposition(value, group)
                }
                GroupCombinators::AllAnyOrder => {
                    match_group_all_any_order(value, group)
                }
                GroupCombinators::AtLeastOneAnyOrder => {
                    match_group_at_least_one_any_order(value, group)
                }
                GroupCombinators::ExactlyOne => {
                    match_group_exactly_one(value, group)
                }
            };
        }
        _ => None,
    };

    // match component.multipliers {
    //     SyntaxComponentMultiplier::Once => {}
    //     SyntaxComponentMultiplier::ZeroOrMore => {}
    //     SyntaxComponentMultiplier::OneOrMore => {}
    //     SyntaxComponentMultiplier::Optional => {}
    //     SyntaxComponentMultiplier::Between(_n, _m) => {}
    //     SyntaxComponentMultiplier::AtLeastOneValue => {}
    //     SyntaxComponentMultiplier::CommaSeparatedRepeat(_n, _m) => {}
    // }

    // return None;
}

fn match_group_exactly_one(value: &CssValue, group: &Group) -> Option<CssValue> {
    let entries = resolve_group(value, group);

    // We must have exactly one element
    if entries.len() == 1 {
        let (v_idx, _) = entries[0];

        if let CssValue::List(list) = value.as_list() {
            return Some(list[v_idx].clone());
        }
    }

    return None;
}

fn resolve_group(value: &CssValue, group: &Group) -> Vec<(usize, usize)> {
    let mut values: Vec<(usize, usize)> = vec![];

    if let CssValue::List(list) = value.as_list() {
        for (v_idx, value) in list.iter().enumerate() {
            for (c_idx, component) in group.components.iter().enumerate() {
                if match_internal(value, component).is_some() {
                    values.push((v_idx, c_idx));
                    break;
                }
            }
        }
    }

    dbg!(&values);
    return values;
}

fn match_group_at_least_one_any_order(value: &CssValue, group: &Group) -> Option<CssValue> {
    let values = resolve_group(value, group);

    // We must have at least one element
    if values.len() >= 1 {
        return Some(CssValue::String("foobar".into()));
    }

    return None;
}

fn match_group_all_any_order(value: &CssValue, group: &Group) -> Option<CssValue> {
    let values = resolve_group(value, group);

    // We must have resolved all values, but we don't care about the ordering
    if values.len() == group.components.len() {
        return Some(CssValue::String("foobar".into()));
    }

    return None;
}

fn match_group_juxtaposition(value: &CssValue, group: &Group) -> Option<CssValue> {
    let values = resolve_group(value, group);

    // We must have resolved all values in the correct order
    if values.len() != group.components.len() {
        return None;
    }

    // Check the ordering...
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
        assert!(tree.matches(&str!("auto")).is_none());
        assert!(tree.matches(&CssValue::None).is_none());
        assert!(tree.matches(&str!("block")).is_none());
        assert!(tree.matches(&CssValue::List(vec![str!("auto"), CssValue::None, str!("block")])).is_some());
        assert!(tree.matches(&CssValue::List(vec![str!("block"), CssValue::None, str!("block")])).is_none());
        assert!(tree.matches(&CssValue::List(vec![str!("auto"), CssValue::None, str!("auto")])).is_none());
    }

}