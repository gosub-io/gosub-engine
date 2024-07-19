use gosub_css3::colors::CSS_COLORNAMES;
use gosub_css3::stylesheet::CssValue;

use crate::syntax::{Group, GroupCombinators, SyntaxComponent, SyntaxComponentMultiplier, SyntaxComponentType};

const LENGTH_UNITS: [&str; 31] = [
    "cap", "ch", "em", "ex", "ic", "lh", "rcap", "rch", "rem", "rex", "ric", "rlh", "vh", "vw",
    "vmax", "vmin", "vb", "vi", "cqw", "cqh", "cqi", "cqb", "cqmin", "cqmax", "px", "cm", "mm",
    "Q", "in", "pc", "pt",
];

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
    match &component.type_ {
        SyntaxComponentType::GenericKeyword(keyword) => match value {
            CssValue::None if keyword.eq_ignore_ascii_case("none") => return Some(value.clone()),
            CssValue::String(v) if v.eq_ignore_ascii_case(keyword) => return Some(value.clone()),
            _ => {
                // Did not match the keyword
                // dbg!(keyword, value);
                // panic!("Unknown generic keyword: {:?}", keyword);
            }
        },
        SyntaxComponentType::Scalar(scalar) => match scalar.as_str() {
            "percentage" | "<percentage>" => {
                if matches!(value, CssValue::Percentage(_)) {
                    println!("Matched percentage! {:?}", value.clone());
                    return Some(value.clone());
                }
            }
            "number" | "<number>" => {
                if matches!(value, CssValue::Number(_)) {
                    return Some(value.clone());
                }
            }
            "named-color" => {
                let v = match value {
                    CssValue::String(v) => v,
                    CssValue::Color(_) => return Some(value.clone()),
                    _ => return None,
                };

                if CSS_COLORNAMES
                    .iter()
                    .any(|entry| entry.name.eq_ignore_ascii_case(v))
                {
                    return Some(value.clone()); //TODO: should we convert the color directly to a CssValue::Color?
                }
            }
            "system-color" => {
                return None; //TODO
                             // return Some(value.clone()) //TODO
            }

            "length" => match value {
                CssValue::Zero => return Some(value.clone()),
                CssValue::Unit(_, u) if LENGTH_UNITS.contains(&u.as_str()) => {
                    return Some(value.clone())
                }
                _ => {}
            },

            _ => panic!("Unknown scalar: {:?}", scalar),
        },
        // SyntaxComponentType::Property(_s) => {},
        // SyntaxComponentType::Function(_s, _t) => {}
        // SyntaxComponentType::TypeDefinition(_s, _t, _u) => {}
        SyntaxComponentType::Inherit => match value {
            CssValue::Inherit => return Some(value.clone()),
            CssValue::String(v) if v.eq_ignore_ascii_case("inherit") => return Some(value.clone()),
            _ => {}
        },
        SyntaxComponentType::Initial => match value {
            CssValue::Initial => return Some(value.clone()),
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
                CssValue::Number(n) if *n == 0.0 => return Some(CssValue::Zero),
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
                GroupCombinators::AtLeastOneAnyOrder => match_group_at_least_one_any_order(value, group),
                GroupCombinators::ExactlyOne => match_group_exactly_one(value, group),
            };
        }
        SyntaxComponentType::Literal(lit) => {
            match value {
                CssValue::String(v) if v.eq_ignore_ascii_case(lit) => return Some(value.clone()), //TODO: can we ignore case?
                _ => {}
            };
        }
        SyntaxComponentType::Function(name, args) => {
            let CssValue::Function(c_name, c_args) = value else {
                return None;
            };

            if !name.eq_ignore_ascii_case(c_name) {
                return None;
            }

            let Some(args) = args else {
                if c_args.is_empty() {
                    return Some(value.clone());
                }

                return None;
            };

            let list = CssValue::List(c_args.clone());

            return match_internal(&list, args);
        }
        SyntaxComponentType::Property(prop) => {
            dbg!(prop, value.clone());

            match &**prop {
                "border-color" => {
                    let v = match value {
                        CssValue::String(v) => v,
                        CssValue::Color(_) => return Some(value.clone()),
                        _ => return None,
                    };

                    if CSS_COLORNAMES
                        .iter()
                        .any(|entry| entry.name.eq_ignore_ascii_case(v))
                    {
                        return Some(value.clone()); //TODO: should we convert the color directly to a CssValue::Color?
                    }
                }

                _ => {
                    todo!("Property not implemented yet")
                }
            }
        }
        e => {
            println!("Unknown syntax component type: {:?}", e);
        }
    }

    None
}

#[derive(Debug)]
struct Matches {
    /// Entry is either -1 for each element in the value, or the index of the component that matched
    entries: Vec<isize>,
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
fn resolve_group(value: &CssValue, group: &Group) -> Matches {
    let mut values: Vec<isize> = vec![];

    println!("Resolving group");

    // Iterate all values and see where they match in our group
    value.iter().for_each(|v| {
        println!("  Iterating element: {:?}", v);

        // Assume the value cannot be matched in the group
        let mut value_in_group_index = -1;

        // Iterate the whole group
        for (c_idx, component) in group.components.iter().enumerate() {
            match match_internal(v, component) {
                Some(_) => {
                    value_in_group_index = c_idx as isize;
                    break;
                }
                None => {}
            }
        }

        // Add the index of the matched component to the list, or -1 when it is not matched
        values.push(value_in_group_index);
    });

    Matches {
        entries: values,
    }
}

fn match_group_exactly_one(value: &CssValue, group: &Group) -> Option<CssValue> {
    let matches = resolve_group(value, group);
    println!("Matching exactly one");
    dbg!(&value, &matches);


    // We must have exactly one element
    if matches.entries.len() != 1 {
        return None;
    }

    // Check if there are -1's in the list (the list is always size 1)
    if matches.entries.iter().any(|&x| x == -1) {
        return None;
    }

    Some(value.clone())
}

fn match_group_at_least_one_any_order(value: &CssValue, group: &Group) -> Option<CssValue> {
    let matches = resolve_group(value, group);
    println!("Matching at least one any order");
    dbg!(&value, &matches);

    // We must have at least one element
    if matches.entries.is_empty() {
        return None;
    }

    // Check if there are -1's in the list
    if matches.entries.iter().all(|&x| x == -1) {
        return None;
    }

    // One (or more) elements found in the value that matched. There are no elements that do not match
    Some(value.clone())
}

fn match_group_all_any_order(value: &CssValue, group: &Group) -> Option<CssValue> {
    let matches = resolve_group(value, group);
    println!("Matching all any order");
    dbg!(&value, &matches);

    // If we do not the same length in our matches, we definitely don't have a match
    if matches.entries.len() != group.components.len() {
        return None;
    }

    // check if there are -1 in the list
    if matches.entries.iter().any(|&x| x == -1) {
        return None;
    }

    // We have the same number of matches as the elements in the group. We also have no -1's in the
    // list so we have a match
    Some(value.clone())
}

fn match_group_juxtaposition(value: &CssValue, group: &Group) -> Option<CssValue> {
    // Step 1: convert to matches
    let matches = resolve_group(value, group);
    println!("Matching juxtaposition");
    dbg!(&value, &matches);

    // Step 2: early return when we found a group with a single element
    //FIXME: this is a hack, since our parser of the css value syntax sometimes inserts additional juxtapositions when it encounters a space.
    if group.components.len() == 1 && group.components[0].is_group() {
        return Some(value.clone());
    }

    // Step 3: Check if there are -1 in the list. If so, we found unknown values, and thus we can return immediately
    if matches.entries.iter().any(|&x| x == -1) {
        return None;
    }

    // Step 4: Validate multipliers based on the matches
    let items = convert_to_counts(matches);

    // Check multipliers and see if we got the correct number of matches per component, and the values are in the correct (sequential) order
    for (c_idx, group_component) in group.components.iter().enumerate() {
        // Find (the first) value count for the given index (or 0 if the value count is not found for that index)
        let value_count = items.iter().find(|(idx, _count)| *idx == c_idx).unwrap_or(&(c_idx, 0)).1;

        // Check if this count is correct for the group validator
        if !check_multiplier(group_component, value_count) {
            return None;
        }
    }

    // Step 5: check if the order is correct. Juxtaposition means we must have incremental values.
    let mut last_idx = 0;
    for (idx, _) in items.iter() {
        if *idx >= last_idx {
            last_idx = *idx;
        } else {
            return None;
        }
    }

    Some(value.clone())
}

// Convert the matches into counts
fn convert_to_counts(matches: Matches) -> Vec<(usize, usize)> {
    let mut items: Vec<(usize, usize)> = vec![];
    for idx in matches.entries.iter() {
        if *idx == -1 {
            continue;
        }

        if items.is_empty() || items.last().unwrap().0 != *idx as usize {
            items.push((*idx as usize, 1));
        } else {
            items.last_mut().unwrap().1 += 1;
        }
    }

    items
}


/// This function checks if the given component matches the given count of values. For instance, it will return true
/// when the multiplier is Once, and there is a count of 1,   or when the multiplier is OneOrMore, when the count is 3.
fn check_multiplier(component: &SyntaxComponent, count: usize) -> bool {
    match component.multipliers {
        SyntaxComponentMultiplier::Once => {
            count == 1
        }
        SyntaxComponentMultiplier::ZeroOrMore => {
            // Zero or more always matches
            true
        }
        SyntaxComponentMultiplier::OneOrMore => {
            count >= 1
        }
        SyntaxComponentMultiplier::Optional => {
            count <= 1
        }
        SyntaxComponentMultiplier::Between(s, e) => {
            count >= s && count <= e
        }
        SyntaxComponentMultiplier::AtLeastOneValue => {
            // @TODO: What's the difference between this and OneOrMore?
            count >= 1
        }
        SyntaxComponentMultiplier::CommaSeparatedRepeat(_s, _e) => {
            panic!("CommaSeparatedRepeat not implemented yet");
        }
    }
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
        // TODO: this might be correct, since we have at least one in any order, thus being `CssValue::None`
        // assert_none!(tree.matches(&CssValue::List(vec![str!("foo"), CssValue::None])));
        // assert_none!(tree.matches(&CssValue::List(vec![CssValue::None, str!("foo")])));
        assert_some!(tree.matches(&CssValue::List(vec![str!("auto"), CssValue::None])));
        // assert_none!(tree.matches(&CssValue::List(vec![
        //     str!("auto"),
        //     CssValue::Comma,
        //     str!("none")
        // ])));
        // assert_none!(tree.matches(&CssValue::List(vec![
        //     str!("auto"),
        //     CssValue::Comma,
        //     CssValue::None,
        //     CssValue::Comma,
        //     str!("block")
        // ])));
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
            let values = resolve_group(&CssValue::List(vec![str!("auto")]), group).entries;
            assert_eq!(values, [0]);

            let values =
                resolve_group(&CssValue::List(vec![str!("auto"), str!("none")]), group).entries;
            assert_eq!(values, [0, 1]);

            let values = resolve_group(
                &CssValue::List(vec![str!("auto"), str!("none"), str!("block")]),
                group,
            )
            .entries;
            assert_eq!(values, [0, 1, 2]);

            let values = resolve_group(
                &CssValue::List(vec![str!("none"), str!("block"), str!("auto")]),
                group,
            )
            .entries;
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
            )
            .entries;
            assert_eq!(values, [1, 2, 2, 0, 1]);

            let values = resolve_group(
                &CssValue::List(vec![
                    str!("none"),
                    str!("banana"),
                    str!("car"),
                    str!("block"),
                ]),
                group,
            )
            .entries;
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
        }
    }

    #[test]
    fn test_multipliers_optional() {
        let tree = CssSyntax::new("foo bar baz").compile().unwrap();
        assert_none!(tree.clone().matches(&CssValue::String("foo".into())));
        assert_none!(tree.clone().matches(&CssValue::List(vec![
            CssValue::String("foo".into())
        ])));
        assert_some!(tree.clone().matches(&CssValue::List(vec![
            CssValue::String("foo".into()),
            CssValue::String("bar".into()),
            CssValue::String("baz".into()),
        ])));
        assert_none!(tree.clone().matches(&CssValue::List(vec![
            CssValue::String("foo".into()),
            CssValue::String("baz".into()),
        ])));

        let tree = CssSyntax::new("foo bar?").compile().unwrap();
        assert_some!(tree.clone().matches(&CssValue::String("foo".into())));
        assert_some!(tree.clone().matches(&CssValue::List(vec![
            CssValue::String("foo".into())
        ])));
        assert_some!(tree.clone().matches(&CssValue::List(vec![
            CssValue::String("foo".into()),
            CssValue::String("bar".into()),
        ])));
        assert_none!(tree.clone().matches(&CssValue::List(vec![
            CssValue::String("foo".into()),
            CssValue::String("bar".into()),
            CssValue::String("bar".into()),
        ])));
        assert_none!(tree.clone().matches(&CssValue::List(vec![
            CssValue::String("bar".into()),
            CssValue::String("foo".into()),
        ])));

        let tree = CssSyntax::new("foo bar? baz").compile().unwrap();
        assert_none!(tree.clone().matches(&CssValue::String("foo".into())));
        assert_some!(tree.clone().matches(&CssValue::List(vec![
            CssValue::String("foo".into()),
            CssValue::String("baz".into())
        ])));
        assert_some!(tree.clone().matches(&CssValue::List(vec![
            CssValue::String("foo".into()),
            CssValue::String("bar".into()),
            CssValue::String("baz".into()),
        ])));

        assert_none!(tree.clone().matches(&CssValue::List(vec![
            CssValue::String("foo".into()),
            CssValue::String("bar".into()),
            CssValue::String("bar".into()),
            CssValue::String("baz".into()),
        ])));

        assert_none!(tree.clone().matches(&CssValue::List(vec![
            CssValue::String("foo".into()),
            CssValue::String("bar".into()),
            CssValue::String("baz".into()),
            CssValue::String("baz".into()),
        ])));
    }


    #[test]
    fn test_convert_to_counts() {
        let matches = Matches {
            entries: vec![0, 1, 1, 2, 3, 1],
        };

        let counts = convert_to_counts(matches);
        assert_eq!(counts, vec![
            (0, 1),
            (1, 2),
            (2, 1),
            (3, 1),
            (1, 1),
        ]);


        let matches = Matches {
            entries: vec![0, 0, 0, 1, 1, 2, 2, 2, 3, 3, 3, 1, 2, 3, 4, 3, 3, 3],
        };

        let counts = convert_to_counts(matches);
        assert_eq!(counts, vec![
            (0, 3),
            (1, 2),
            (2, 3),
            (3, 3),
            (1, 1),
            (2, 1),
            (3, 1),
            (4, 1),
            (3, 3),
        ]);
    }
}
