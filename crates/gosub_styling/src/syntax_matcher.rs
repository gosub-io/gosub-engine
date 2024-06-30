use gosub_css3::stylesheet::CssValue;
use serde_json::Value;

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
    if let CssValue::List(list) = value {
        for value in list.iter() {
            match_internal(value, component)?;
        }

        return Some(value.clone());
    }

    match &component.type_ {
        SyntaxComponentType::GenericKeyword(keyword) => match value {
            CssValue::None if keyword.eq_ignore_ascii_case("none") => return Some(value.clone()),
            CssValue::String(v) if v.eq_ignore_ascii_case(keyword) => return Some(value.clone()),
            _ => {}
        },
        SyntaxComponentType::Scalar(scalar) => match scalar.as_str() {
            "<number>" => {
                if matches!(value, CssValue::Number(_)) {
                    return Some(value.clone());
                }
            }
            _ => todo!(),
        },
        // SyntaxComponentType::Property(_s) => {},
        // SyntaxComponentType::Function(_s, _t) => {}
        // SyntaxComponentType::TypeDefinition(_s, _t, _u) => {}
        SyntaxComponentType::Inherit => match value {
            CssValue::String(v) if v.eq_ignore_ascii_case("inherit") => return Some(value.clone()),
            _ => {}
        },
        SyntaxComponentType::Initial => match value {
            CssValue::String(v) if v.eq_ignore_ascii_case("initial") => return Some(value.clone()),
            _ => {}
        },
        SyntaxComponentType::Unset => match value {
            CssValue::String(v) if v.eq_ignore_ascii_case("unset") => return Some(value.clone()),
            _ => {}
        },
        // SyntaxComponentType::Value(_s) => {}
        SyntaxComponentType::Unit(from, to, units) => {
            let f32min = f32::MIN;
            let f32max = f32::MAX;

            match value {
                CssValue::Number(n) if *n == 0.0 => return Some(value.clone()),
                CssValue::Unit(n, u)
                    if units.contains(u)
                        && n >= &from.unwrap_or(f32min)
                        && n <= &to.unwrap_or(f32max) =>
                {
                    return Some(value.clone())
                }
                _ => {}
            };
        }
        SyntaxComponentType::Group(group) => {
            return match group.combinator {
                GroupCombinators::Juxtaposition => match_group_juxtaposition(value, group),
                GroupCombinators::AllAnyOrder => match_group_all_any_order(value, group),
                GroupCombinators::AtLeastOneAnyOrder => {
                    match_group_at_least_one_any_order(value, group)
                }
                GroupCombinators::ExactlyOne => match_group_exactly_one(value, group),
            };
        }

        SyntaxComponentType::Literal(lit) => {
            match value {
                CssValue::String(v) if v.eq_ignore_ascii_case(lit) => return Some(value.clone()), //TODO: can we ignore case?
                _ => {}
            };
        }
        e => {
            panic!("Unknown syntax component type: {:?}", e);
        }
    }

    None
}

fn match_group_exactly_one(value: &CssValue, group: &Group) -> Option<CssValue> {
    let entries = resolve_group(value, group);

    // We must have exactly one element
    if entries.len() != 1 {
        return None;
    }

    // check if there are -1 in the list
    if entries.iter().any(|&x| x == -1) {
        return None;
    }
    if let CssValue::List(list) = value.as_list() {
        return Some(list[0].clone());
    }

    return None;
}

/// Resolves a group of values against a group of components based on their position. So if the
/// first element matches the first component a 0 will be inserted on the first (0) position.
///
/// Example:
///     resolve_group([auto, none, block], [auto, none, block]) => [0, 1, 2]
///     resolve_group([none, block, auto], [auto, none, block]) => [1, 2, 0]
///     resolve_group([none, banana, car, block], [auto, none, block]) => [1, -1, -1, 2]
///     resolve_group([none, block, block, auto, none], [auto, none, block]) => [1, 2, 2, 0, 1]
///
fn resolve_group(value: &CssValue, group: &Group) -> Vec<isize> {
    let mut values: Vec<isize> = vec![];

    if let CssValue::List(list) = value.as_list() {
        for value in list.iter() {
            let mut v_idx = -1;
            for (c_idx, component) in group.components.iter().enumerate() {
                if !match_internal(value, component).is_some() {
                    continue;
                }
                v_idx = c_idx as isize;
            }
            values.push(v_idx);
        }
    }
    return values;
}

fn match_group_at_least_one_any_order(value: &CssValue, group: &Group) -> Option<CssValue> {
    let values = resolve_group(value, group);

    // We must have at least one element
    if values.is_empty() {
        return None;
    }

    // check if there are -1 in the list
    if values.iter().all(|&x| x == -1) {
        return None;
    }

    return Some(value.clone());
}

fn match_group_all_any_order(value: &CssValue, group: &Group) -> Option<CssValue> {
    let values = resolve_group(value, group);

    // If it's not the same length, we definitely don't have a match
    if values.len() != group.components.len() {
        return None;
    }

    // check if there are -1 in the list
    if values.iter().any(|&x| x == -1) {
        return None;
    }

    return Some(value.clone());
}

fn match_group_juxtaposition(value: &CssValue, group: &Group) -> Option<CssValue> {
    let values = resolve_group(value, group);

    // If it's not the same length, we definitely don't have a match
    if values.len() != group.components.len() {
        return None;
    }

    // check if there are -1 in the list
    if values.iter().any(|&x| x == -1) {
        return None;
    }

    // Check if the values are in the correct order
    let mut c_idx = 0;
    while c_idx < values.len() {
        if values[c_idx] != c_idx as isize {
            return None;
        }
        c_idx += 1;
    }

    return Some(value.clone());
}

#[cfg(test)]
mod tests {
    use gosub_css3::stylesheet::CssValue;

    use crate::syntax::CssSyntax;

    use super::*;

    macro_rules! str {
        ($s:expr) => {
            CssValue::String($s.to_string())
        };
    }

    macro_rules! assert_none {
        ($e:expr) => {
            assert!($e.is_none());
        };
    }

    macro_rules! assert_some {
        ($e:expr) => {
            assert!($e.is_some());
        };
    }

    #[test]
    fn test_match_group1() {
        // Exactly one
        let tree = CssSyntax::new("auto | none | block").compile().unwrap();
        assert_some!(tree.matches(&str!("auto")));
        assert_some!(tree.matches(&CssValue::None));
        assert_some!(tree.matches(&str!("block")));
        assert_none!(tree.matches(&str!("inline")));
        assert_none!(tree.matches(&str!("")));
        assert_none!(tree.matches(&str!("foobar")));
        assert_none!(tree.matches(&CssValue::List(vec![str!("foo"), CssValue::None])));
        assert_none!(tree.matches(&CssValue::List(vec![CssValue::None, str!("foo")])));
        assert_none!(tree.matches(&CssValue::List(vec![str!("auto"), CssValue::None])));
        assert_none!(tree.matches(&CssValue::List(vec![
            str!("auto"),
            CssValue::Comma,
            str!("none")
        ])));
        assert_none!(tree.matches(&CssValue::List(vec![
            str!("auto"),
            CssValue::Comma,
            CssValue::None,
            CssValue::Comma,
            str!("block")
        ])));
    }

    #[test]
    fn test_match_group2() {
        // juxtaposition
        let tree = CssSyntax::new("auto none block").compile().unwrap();
        assert_none!(tree.matches(&str!("auto")));
        assert_none!(tree.matches(&CssValue::None));
        assert_none!(tree.matches(&str!("block")));
        assert_some!(tree.matches(&CssValue::List(vec![
            str!("auto"),
            CssValue::None,
            str!("block")
        ])));
        assert_none!(tree.matches(&CssValue::List(vec![
            str!("block"),
            CssValue::None,
            str!("block")
        ])));
        assert_none!(tree.matches(&CssValue::List(vec![
            str!("auto"),
            CssValue::None,
            str!("auto")
        ])));
    }

    #[test]
    fn test_match_group3() {
        // all any order
        let tree = CssSyntax::new("auto && none && block").compile().unwrap();
        assert_none!(tree.matches(&str!("auto")));
        assert_none!(tree.matches(&CssValue::None));
        assert_none!(tree.matches(&str!("block")));
        assert_none!(tree.matches(&str!("inline")));
        assert_none!(tree.matches(&str!("")));
        assert_none!(tree.matches(&str!("foobar")));
        assert_none!(tree.matches(&CssValue::List(vec![str!("foo"), CssValue::None])));
        assert_none!(tree.matches(&CssValue::List(vec![CssValue::None, str!("foo")])));
        assert_none!(tree.matches(&CssValue::List(vec![str!("auto"), CssValue::None])));
        assert_none!(tree.matches(&CssValue::List(vec![
            str!("auto"),
            CssValue::Comma,
            str!("none")
        ])));
        assert_none!(tree.matches(&CssValue::List(vec![
            str!("auto"),
            CssValue::Comma,
            CssValue::None,
            CssValue::Comma,
            str!("block")
        ])));
        assert_some!(tree.matches(&CssValue::List(vec![
            str!("block"),
            str!("auto"),
            CssValue::None
        ])));
        assert_some!(tree.matches(&CssValue::List(vec![
            str!("auto"),
            str!("block"),
            CssValue::None
        ])));
        assert_some!(tree.matches(&CssValue::List(vec![
            str!("block"),
            CssValue::None,
            str!("auto")
        ])));
        assert_some!(tree.matches(&CssValue::List(vec![
            CssValue::None,
            str!("auto"),
            str!("block")
        ])));
        assert_none!(tree.matches(&CssValue::List(vec![
            str!("block"),
            str!("block"),
            CssValue::None,
            CssValue::None
        ])));
    }

    #[test]
    fn test_match_group4() {
        // At least one in any order
        let tree = CssSyntax::new("auto || none || block").compile().unwrap();
        assert_some!(tree.matches(&str!("auto")));
        assert_some!(tree.matches(&CssValue::None));
        assert_some!(tree.matches(&str!("block")));
        assert_none!(tree.matches(&str!("inline")));
        assert_none!(tree.matches(&str!("")));
        assert_none!(tree.matches(&str!("foobar")));
        assert_none!(tree.matches(&CssValue::List(vec![str!("foo"), CssValue::None])));
        assert_none!(tree.matches(&CssValue::List(vec![CssValue::None, str!("foo")])));
        assert_some!(tree.matches(&CssValue::List(vec![str!("auto"), CssValue::None])));
        assert_none!(tree.matches(&CssValue::List(vec![
            str!("auto"),
            CssValue::Comma,
            str!("none")
        ])));
        assert_none!(tree.matches(&CssValue::List(vec![
            str!("auto"),
            CssValue::Comma,
            CssValue::None,
            CssValue::Comma,
            str!("block")
        ])));
        assert_some!(tree.matches(&CssValue::List(vec![
            str!("block"),
            str!("auto"),
            CssValue::None
        ])));
        assert_some!(tree.matches(&CssValue::List(vec![
            str!("block"),
            str!("block"),
            CssValue::None,
            CssValue::None
        ])));
    }

    #[test]
    fn test_resolve_group() {
        let tree = CssSyntax::new("auto none block").compile().unwrap();
        if let SyntaxComponentType::Group(group) = &tree.components[0].type_ {
            let values = resolve_group(&CssValue::List(vec![str!("auto")]), group);
            assert_eq!(values, [0]);

            let values = resolve_group(&CssValue::List(vec![str!("auto"), str!("none")]), group);
            assert_eq!(values, [0, 1]);

            let values = resolve_group(
                &CssValue::List(vec![str!("auto"), str!("none"), str!("block")]),
                group,
            );
            assert_eq!(values, [0, 1, 2]);

            let values = resolve_group(
                &CssValue::List(vec![str!("none"), str!("block"), str!("auto")]),
                group,
            );
            assert_eq!(values, [1, 2, 0]);

            let values = resolve_group(
                &CssValue::List(vec![
                    str!("none"),
                    str!("block"),
                    str!("block"),
                    str!("auto"),
                    str!("none"),
                ]),
                group,
            );
            assert_eq!(values, [1, 2, 2, 0, 1]);

            let values = resolve_group(
                &CssValue::List(vec![
                    str!("none"),
                    str!("banana"),
                    str!("car"),
                    str!("block"),
                ]),
                group,
            );
            assert_eq!(values, [1, -1, -1, 2]);
        }
    }

    #[test]
    fn test_match_group_juxtaposition() {
        let tree = CssSyntax::new("auto none block").compile().unwrap();
        if let SyntaxComponentType::Group(group) = &tree.components[0].type_ {
            let res = match_group_juxtaposition(&CssValue::List(vec![str!("auto")]), group);
            assert_none!(res);

            let res =
                match_group_juxtaposition(&CssValue::List(vec![str!("auto"), str!("none")]), group);
            assert_none!(res);

            let res = match_group_juxtaposition(
                &CssValue::List(vec![str!("auto"), str!("none"), str!("block")]),
                group,
            );
            assert_some!(res);

            let res = match_group_juxtaposition(
                &CssValue::List(vec![str!("none"), str!("block"), str!("auto")]),
                group,
            );
            assert_none!(res);

            let res = match_group_juxtaposition(
                &CssValue::List(vec![
                    str!("none"),
                    str!("block"),
                    str!("block"),
                    str!("auto"),
                    str!("none"),
                ]),
                group,
            );
            assert_none!(res);

            let res = match_group_juxtaposition(
                &CssValue::List(vec![
                    str!("none"),
                    str!("banana"),
                    str!("car"),
                    str!("block"),
                ]),
                group,
            );
            assert_none!(res);
        }
    }

    #[test]
    fn test_match_group_all_any_order() {
        let tree = CssSyntax::new("auto none block").compile().unwrap();
        if let SyntaxComponentType::Group(group) = &tree.components[0].type_ {
            let res = match_group_all_any_order(&CssValue::List(vec![str!("auto")]), group);
            assert_none!(res);

            let res =
                match_group_all_any_order(&CssValue::List(vec![str!("auto"), str!("none")]), group);
            assert_none!(res);

            let res = match_group_all_any_order(
                &CssValue::List(vec![str!("auto"), str!("none"), str!("block")]),
                group,
            );
            assert_some!(res);

            let res = match_group_all_any_order(
                &CssValue::List(vec![str!("none"), str!("block"), str!("auto")]),
                group,
            );
            assert_some!(res);

            let res = match_group_all_any_order(
                &CssValue::List(vec![
                    str!("none"),
                    str!("block"),
                    str!("block"),
                    str!("auto"),
                    str!("none"),
                ]),
                group,
            );
            assert_none!(res);

            let res = match_group_all_any_order(
                &CssValue::List(vec![
                    str!("none"),
                    str!("banana"),
                    str!("car"),
                    str!("block"),
                ]),
                group,
            );
            assert_none!(res);
        }
    }

    #[test]
    fn test_match_group_at_least_one_any_order() {
        let tree = CssSyntax::new("auto none block").compile().unwrap();
        if let SyntaxComponentType::Group(group) = &tree.components[0].type_ {
            let res =
                match_group_at_least_one_any_order(&CssValue::List(vec![str!("auto")]), group);
            assert_some!(res);

            let res = match_group_at_least_one_any_order(
                &CssValue::List(vec![str!("auto"), str!("none")]),
                group,
            );
            assert_some!(res);

            let res = match_group_at_least_one_any_order(
                &CssValue::List(vec![str!("auto"), str!("none"), str!("block")]),
                group,
            );
            assert_some!(res);

            let res = match_group_at_least_one_any_order(
                &CssValue::List(vec![str!("none"), str!("block"), str!("auto")]),
                group,
            );
            assert_some!(res);

            let res = match_group_at_least_one_any_order(
                &CssValue::List(vec![
                    str!("none"),
                    str!("block"),
                    str!("block"),
                    str!("auto"),
                    str!("none"),
                ]),
                group,
            );
            assert_some!(res);

            let res = match_group_at_least_one_any_order(
                &CssValue::List(vec![
                    str!("none"),
                    str!("banana"),
                    str!("car"),
                    str!("block"),
                ]),
                group,
            );
            assert_none!(res);
        }
    }
}
