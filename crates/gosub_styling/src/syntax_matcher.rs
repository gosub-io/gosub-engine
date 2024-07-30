use gosub_css3::stylesheet::CssValue;

use crate::syntax::{GroupCombinators, SyntaxComponent, SyntaxComponentMultiplier};

#[allow(dead_code)]
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
    // println!("Matching: {:?} against {:?}", value, component);
    match &component {
        SyntaxComponent::GenericKeyword { keyword, .. } => match value {
            CssValue::None if keyword.eq_ignore_ascii_case("none") => return Some(value.clone()),
            CssValue::String(v) if v.eq_ignore_ascii_case(keyword) => return Some(value.clone()),
            _ => {
                // Did not match the keyword
                // panic!("Unknown generic keyword: {:?}", keyword);
            }
        },
        SyntaxComponent::Definition { .. } => {
            // if let CssValue::String(v) = value {
            //     if v.eq_ignore_ascii_case(value) {
            //         return Some(value.clone());
            //     }
            // }
        }

        SyntaxComponent::Builtin { datatype, .. } => match datatype.as_str() {
            "percentage" => {
                match value {
                    CssValue::Percentage(_) => return Some(value.clone()),
                    _ => {}
                }
            }
        //     "number" | "<number>" => {
        //         if matches!(value, CssValue::Number(_)) {
        //             return Some(value.clone());
        //         }
        //     }
            "angle" => match value {
                CssValue::Zero => return Some(value.clone()),
                CssValue::Unit(_, u) if u.eq_ignore_ascii_case("deg") => return Some(value.clone()),
                CssValue::Unit(_, u) if u.eq_ignore_ascii_case("grad") => return Some(value.clone()),
                CssValue::Unit(_, u) if u.eq_ignore_ascii_case("rad") => return Some(value.clone()),
                CssValue::Unit(_, u) if u.eq_ignore_ascii_case("turn") => return Some(value.clone()),
                _ => {}
            }
            "hex-color" => match value {
                CssValue::Color(_) => return Some(value.clone()),
                CssValue::String(v) if v.starts_with('#') => return Some(value.clone()),
                _ => {}
            },
            "length" => match value {
                CssValue::Zero => return Some(value.clone()),
                CssValue::Unit(_, u) if LENGTH_UNITS.contains(&u.as_str()) => {
                    return Some(value.clone())
                }
                _ => {}
            },
            _ => panic!("Unknown built-in datatype: {:?}", datatype),
        },
        // SyntaxComponent::Property(_s) => {},
        // SyntaxComponent::Function(_s, _t) => {}
        // SyntaxComponent::TypeDefinition(_s, _t, _u) => {}
        SyntaxComponent::Inherit { .. } => match value {
            CssValue::Inherit => return Some(value.clone()),
            CssValue::String(v) if v.eq_ignore_ascii_case("inherit") => return Some(value.clone()),
            _ => {}
        },
        SyntaxComponent::Initial { .. } => match value {
            CssValue::Initial => return Some(value.clone()),
            CssValue::String(v) if v.eq_ignore_ascii_case("initial") => return Some(value.clone()),
            _ => {}
        },
        SyntaxComponent::Unset { .. } => match value {
            CssValue::String(v) if v.eq_ignore_ascii_case("unset") => return Some(value.clone()),
            _ => {}
        },
        // SyntaxComponentType::Value(_s) => {}
        SyntaxComponent::Unit { from, to, unit, .. } => {
            let f32min = f32::MIN;
            let f32max = f32::MAX;

            match value {
                CssValue::Number(n) if *n == 0.0 => return Some(CssValue::Zero),
                CssValue::Unit(n, u) => {
                    if unit.contains(u)
                        && *n >= from.unwrap_or(f32min)
                        && *n <= to.unwrap_or(f32max)
                    {
                        return Some(value.clone());
                    }
                }
                _ => {}
            };
        }
        SyntaxComponent::Group {
            components,
            combinator,
            ..
        } => {
            return match combinator {
                GroupCombinators::Juxtaposition => match_group_juxtaposition(value, components),
                GroupCombinators::AllAnyOrder => match_group_all_any_order(value, components),
                GroupCombinators::AtLeastOneAnyOrder => {
                    match_group_at_least_one_any_order(value, components)
                }
                GroupCombinators::ExactlyOne => match_group_exactly_one(value, components),
            };
        }
        SyntaxComponent::Literal { literal, .. } => match value {
            CssValue::String(v) if v.eq(literal) => return Some(value.clone()),
            CssValue::String(v) if v.eq_ignore_ascii_case(literal) => {
                log::warn!("Case insensitive literal matched");
                return Some(value.clone());
            }
            _ => {}
        },
        SyntaxComponent::Function { name, .. } => {
            let CssValue::Function(c_name, c_args) = value else {
                return None;
            };

            if !name.eq_ignore_ascii_case(c_name) {
                return None;
            }

            if c_args.is_empty() {
                return Some(value.clone());
            }

            todo!("Function not implemented yet. We must match the arguments");
            // let list = CssValue::List(c_args.clone());
            // return match_internal(&list, arguments);
        }
        // SyntaxComponent::Property { property, .. } => {
        //     match &**property {
        //         "border-color" => {
        //             let v = match value {
        //                 CssValue::String(v) => v,
        //                 CssValue::Color(_) => return Some(value.clone()),
        //                 _ => return None,
        //             };
        //
        //             if CSS_COLORNAMES
        //                 .iter()
        //                 .any(|entry| entry.name.eq_ignore_ascii_case(v))
        //             {
        //                 return Some(value.clone()); //TODO: should we convert the color directly to a CssValue::Color?
        //             }
        //         }
        //
        //         p => {
        //             todo!("Property {p} not implemented yet")
        //         }
        //     }
        // }
        SyntaxComponent::Value { value: css_value, ..} => {
            if value == css_value {
                return Some(css_value.clone());
            }
        }
        e => {
            panic!("Unknown syntax component: {:?}", e);
        }
    }

    None
}

#[derive(Debug)]
struct Matches {
    /// Entry is either -1 for each element in the value, or the index of the component that matched
    entries: Vec<isize>,
    all: isize,     // @todo: "all" is not the best name. We need to change this.
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
fn resolve_group(value: &CssValue, components: &Vec<SyntaxComponent>) -> Matches {
    // println!("Resolving group: {:?} against {:?}", value, components);
    let mut values: Vec<isize> = vec![];

    let mut subgroup = false;

    // Iterate all values and see where they match in our group
    value.iter().for_each(|v| {
        // Assume the value cannot be matched in the group
        let mut value_in_group_index = -1;

        // Iterate the whole group
        for (c_idx, component) in components.iter().enumerate() {
            if !subgroup && component.is_group() {
                subgroup = true;
            }

            if match_internal(v, component).is_some() {
                value_in_group_index = c_idx as isize;
                break;
            }
        }

        // Add the index of the matched component to the list, or -1 when it is not matched
        values.push(value_in_group_index);
    });

    let mut all = -1;

    if value.is_list() && subgroup {
        // We need to mach the complete value against the components, because it might not be correct to "destructure" the list right here.
        // That can only be the case if the value is a list and we have another group as a component
        for (c_idx, component) in components.iter().enumerate() {
            if match_internal(value, component).is_some() {
                all = c_idx as isize;
                break;
            }
        }
    }

    Matches {
        entries: values,
        all,
    }
}

/// Returns element if exactly one element matches in the group
fn match_group_exactly_one(
    value: &CssValue,
    components: &Vec<SyntaxComponent>,
) -> Option<CssValue> {
    let matches = resolve_group(value, components);
    // println!("match_group_exactly_one: {:?}", matches);

    if matches.all != -1 {
        return Some(value.clone());
    }

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

/// Returns element, when at least one of the elements in the group matches
fn match_group_at_least_one_any_order(
    value: &CssValue,
    components: &Vec<SyntaxComponent>,
) -> Option<CssValue> {
    // Step: convert to matches
    let matches = resolve_group(value, components);
    // println!("match_group_at_least_one_any_order: {:?}", matches);

    let len = if let CssValue::List(list) = value {
        list.len()
    } else {
        1
    };

    // border: 1px black; 2 passing 2 values => valid
    // border: 1px ldsjkfj asfaf asdad black 2 passing but 5 values => invalid => if we have more values than passing it is invalid
    let c = matches.entries.iter().filter(|x| **x != -1).count();
    if c < len {
        return None;
    }

    // Step: Validate multipliers based on the matches
    let items = convert_to_counts(matches);

    // let mut multiplier_passed = false;
    //TODO: is this right for multipliers that are not Once?

    // Check multipliers and see if we got the correct number of matches per component, and the values are in the correct (sequential) order
    for (c_idx, c) in &items {
        if *c == 0 {
            continue;
        }

        let component = &components[*c_idx];

        if !check_multiplier(component, *c) {
            return None;
        }
    }

    // Step: There must be at least one item. Order doesn't matter
    if !items.is_empty() {
        return Some(value.clone());
    }

    None
}

fn match_group_all_any_order(
    value: &CssValue,
    components: &Vec<SyntaxComponent>,
) -> Option<CssValue> {
    let matches = resolve_group(value, components);
    // println!("match_group_all_any_order: {:?}", matches);

    // If we do not the same length in our matches, we definitely don't have a match
    if matches.entries.len() != components.len() {
        return None;
    }

    // Check if there are -1 in the list, if so, return early
    if matches.entries.iter().any(|&x| x == -1) {
        return None;
    }

    // Convert to counts, and check if there are any elements that have more than 1 match
    let items = convert_to_counts(matches);
    if items.iter().filter(|(_, c)| *c > 1).count() > 0 {
        return None;
    }

    // All elements have (at most) one match
    Some(value.clone())
}

fn match_group_juxtaposition(
    value: &CssValue,
    components: &Vec<SyntaxComponent>,
) -> Option<CssValue> {
    // Step: Convert to matches
    let matches = resolve_group(value, components);
    // println!("\n\nmatch_group_juxtaposition: {:?}: {:?} {:?}", matches, value, components);

    // Step: Check if there are -1 in the list. If so, we found unknown values, and thus we can return immediately
    if matches.entries.iter().any(|x| *x == -1) {
        // println!("Found -1 in the list, returning None");
        return None;
    }

    // Step: Validate multipliers based on the matches
    let items = convert_to_counts(matches);

    // Check multipliers and see if we got the correct number of matches per component, and the values are in the correct (sequential) order
    for (c_idx, group_component) in components.iter().enumerate() {
        // Find (the first) value count for the given index (or 0 if the value count is not found for that index)
        let value_count = items
            .iter()
            .find(|(idx, _count)| *idx == c_idx)
            .unwrap_or(&(c_idx, 0))
            .1;

        // println!("\n\nChecking component: {:?} with count: {:?}", group_component, value_count);

        // Check if this count is correct for the group validator
        if !check_multiplier(group_component, value_count) {
            // println!("Multiplier check failed for component: {:?}", group_component);
            return None;
        }
    }

    // Step: check if the order is correct. Juxtaposition means we must have incremental values.
    let mut last_idx = 0;
    for (idx, _) in items.iter() {
        if *idx >= last_idx {
            last_idx = *idx;
        } else {
            // println!("Order check failed: {:?} < {:?}", idx, last_idx);
            return None;
        }
    }

    // println!("Returning value: {:?}", value);
    Some(value.clone())
}

/// Convert the matches into counts
/// (index, count)
/// Note that when we find a value that we already have, we create a new increment:
///
/// Example:
///    [0, 0, 1, 1, 1, 2, 2, 1, 2] => [(0, 2), (1, 3), (2, 2), (1, 1), (2, 1)]
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
/// when the multiplier is Once, and there is a count of 1, or when the multiplier is OneOrMore if the count is 3.
fn check_multiplier(component: &SyntaxComponent, count: usize) -> bool {
    match component.get_multiplier() {
        SyntaxComponentMultiplier::Once => count == 1,
        SyntaxComponentMultiplier::ZeroOrMore => {
            // Zero or more always matches
            true
        }
        SyntaxComponentMultiplier::OneOrMore => count >= 1,
        SyntaxComponentMultiplier::Optional => count <= 1,
        SyntaxComponentMultiplier::Between(s, e) => count >= s && count <= e,
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
        assert_none!(tree.matches(&CssValue::List(vec![
            str!("block"),
            str!("block"),
            CssValue::None,
            CssValue::None
        ])));
    }

    #[test]
    fn test_resolve_group() {
        let tree = CssSyntax::new("auto none block").compile().unwrap();
        if let SyntaxComponent::Group { components, .. } = &tree.components[0] {
            let values = resolve_group(&CssValue::List(vec![str!("auto")]), components).entries;
            assert_eq!(values, [0]);

            let values = resolve_group(
                &CssValue::List(vec![str!("auto"), str!("none")]),
                components,
            )
            .entries;
            assert_eq!(values, [0, 1]);

            let values = resolve_group(
                &CssValue::List(vec![str!("auto"), str!("none"), str!("block")]),
                components,
            )
            .entries;
            assert_eq!(values, [0, 1, 2]);

            let values = resolve_group(
                &CssValue::List(vec![str!("none"), str!("block"), str!("auto")]),
                components,
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
                components,
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
                components,
            )
            .entries;
            assert_eq!(values, [1, -1, -1, 2]);
        }
    }

    #[test]
    fn test_match_group_juxtaposition() {
        let tree = CssSyntax::new("auto none block").compile().unwrap();
        if let SyntaxComponent::Group { components, .. } = &tree.components[0] {
            let res = match_group_juxtaposition(&CssValue::List(vec![str!("auto")]), components);
            assert_none!(res);

            let res = match_group_juxtaposition(
                &CssValue::List(vec![str!("auto"), str!("none")]),
                components,
            );
            assert_none!(res);

            let res = match_group_juxtaposition(
                &CssValue::List(vec![str!("auto"), str!("none"), str!("block")]),
                components,
            );
            assert_some!(res);

            let res = match_group_juxtaposition(
                &CssValue::List(vec![str!("none"), str!("block"), str!("auto")]),
                components,
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
                components,
            );
            assert_none!(res);

            let res = match_group_juxtaposition(
                &CssValue::List(vec![
                    str!("none"),
                    str!("banana"),
                    str!("car"),
                    str!("block"),
                ]),
                components,
            );
            assert_none!(res);
        }
    }

    #[test]
    fn test_match_group_all_any_order() {
        let tree = CssSyntax::new("auto none block").compile().unwrap();
        if let SyntaxComponent::Group { components, .. } = &tree.components[0] {
            let res = match_group_all_any_order(&CssValue::List(vec![str!("auto")]), components);
            assert_none!(res);

            let res = match_group_all_any_order(
                &CssValue::List(vec![str!("auto"), str!("none")]),
                components,
            );
            assert_none!(res);

            let res = match_group_all_any_order(
                &CssValue::List(vec![str!("auto"), str!("none"), str!("block")]),
                components,
            );
            assert_some!(res);

            let res = match_group_all_any_order(
                &CssValue::List(vec![str!("none"), str!("block"), str!("auto")]),
                components,
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
                components,
            );
            assert_none!(res);

            let res = match_group_all_any_order(
                &CssValue::List(vec![
                    str!("none"),
                    str!("banana"),
                    str!("car"),
                    str!("block"),
                ]),
                components,
            );
            assert_none!(res);
        }
    }

    #[test]
    fn test_match_group_at_least_one_any_order() {
        let tree = CssSyntax::new("auto none block").compile().unwrap();
        if let SyntaxComponent::Group { components, .. } = &tree.components[0] {
            let res =
                match_group_at_least_one_any_order(&CssValue::List(vec![str!("auto")]), components);
            assert_some!(res);

            let res = match_group_at_least_one_any_order(
                &CssValue::List(vec![str!("auto"), str!("none")]),
                components,
            );
            assert_some!(res);

            let res = match_group_at_least_one_any_order(
                &CssValue::List(vec![str!("auto"), str!("none"), str!("block")]),
                components,
            );
            assert_some!(res);

            let res = match_group_at_least_one_any_order(
                &CssValue::List(vec![str!("none"), str!("block"), str!("auto")]),
                components,
            );
            assert_some!(res);

            let res = match_group_at_least_one_any_order(
                &CssValue::List(vec![
                    str!("none"),
                    str!("block"),
                    // str!("block"),
                    str!("auto"),
                    // str!("none"),
                ]),
                components,
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
                components,
            );
            assert_none!(res);
        }
    }

    #[test]
    fn test_multipliers_optional() {
        let tree = CssSyntax::new("foo bar baz").compile().unwrap();
        assert_none!(tree.clone().matches(&CssValue::String("foo".into())));
        assert_none!(tree
            .clone()
            .matches(&CssValue::List(vec![CssValue::String("foo".into())])));
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
        assert_some!(tree
            .clone()
            .matches(&CssValue::List(vec![CssValue::String("foo".into())])));
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
    fn test_multipliers_zero_or_more() {
        let tree = CssSyntax::new("foo bar* baz").compile().unwrap();
        assert_none!(tree.clone().matches(&CssValue::String("foo".into())));
        assert_none!(tree
            .clone()
            .matches(&CssValue::List(vec![CssValue::String("foo".into())])));
        assert_some!(tree.clone().matches(&CssValue::List(vec![
            CssValue::String("foo".into()),
            CssValue::String("bar".into()),
            CssValue::String("baz".into()),
        ])));
        assert_some!(tree.clone().matches(&CssValue::List(vec![
            CssValue::String("foo".into()),
            CssValue::String("baz".into()),
        ])));
        assert_some!(tree.clone().matches(&CssValue::List(vec![
            CssValue::String("foo".into()),
            CssValue::String("bar".into()),
            CssValue::String("bar".into()),
            CssValue::String("bar".into()),
            CssValue::String("bar".into()),
            CssValue::String("baz".into()),
        ])));
        assert_none!(tree.clone().matches(&CssValue::List(vec![
            CssValue::String("foo".into()),
            CssValue::String("bar".into()),
            CssValue::String("bar".into()),
            CssValue::String("bar".into()),
            CssValue::String("baz".into()),
            CssValue::String("bar".into()),
        ])));

        let tree = CssSyntax::new("foo bar*").compile().unwrap();
        assert_some!(tree.clone().matches(&CssValue::String("foo".into())));
        assert_some!(tree
            .clone()
            .matches(&CssValue::List(vec![CssValue::String("foo".into())])));
        assert_some!(tree.clone().matches(&CssValue::List(vec![
            CssValue::String("foo".into()),
            CssValue::String("bar".into()),
        ])));
        assert_some!(tree.clone().matches(&CssValue::List(vec![
            CssValue::String("foo".into()),
            CssValue::String("bar".into()),
            CssValue::String("bar".into()),
        ])));
        assert_none!(tree.clone().matches(&CssValue::List(vec![
            CssValue::String("bar".into()),
            CssValue::String("foo".into()),
        ])));
    }

    #[test]
    fn test_multipliers_one_or_more() {
        let tree = CssSyntax::new("foo bar+ baz").compile().unwrap();
        assert_none!(tree.clone().matches(&CssValue::String("foo".into())));
        assert_none!(tree
            .clone()
            .matches(&CssValue::List(vec![CssValue::String("foo".into())])));
        assert_some!(tree.clone().matches(&CssValue::List(vec![
            CssValue::String("foo".into()),
            CssValue::String("bar".into()),
            CssValue::String("baz".into()),
        ])));
        assert_none!(tree.clone().matches(&CssValue::List(vec![
            CssValue::String("foo".into()),
            CssValue::String("baz".into()),
        ])));
        assert_some!(tree.clone().matches(&CssValue::List(vec![
            CssValue::String("foo".into()),
            CssValue::String("bar".into()),
            CssValue::String("bar".into()),
            CssValue::String("bar".into()),
            CssValue::String("bar".into()),
            CssValue::String("baz".into()),
        ])));
        assert_none!(tree.clone().matches(&CssValue::List(vec![
            CssValue::String("foo".into()),
            CssValue::String("bar".into()),
            CssValue::String("bar".into()),
            CssValue::String("bar".into()),
            CssValue::String("baz".into()),
            CssValue::String("bar".into()),
        ])));

        let tree = CssSyntax::new("foo bar+").compile().unwrap();
        assert_none!(tree.clone().matches(&CssValue::String("foo".into())));
        assert_none!(tree
            .clone()
            .matches(&CssValue::List(vec![CssValue::String("foo".into())])));
        assert_none!(tree
            .clone()
            .matches(&CssValue::List(vec![CssValue::String("bar".into())])));
        assert_some!(tree.clone().matches(&CssValue::List(vec![
            CssValue::String("foo".into()),
            CssValue::String("bar".into()),
        ])));
        assert_some!(tree.clone().matches(&CssValue::List(vec![
            CssValue::String("foo".into()),
            CssValue::String("bar".into()),
            CssValue::String("bar".into()),
        ])));
        assert_none!(tree.clone().matches(&CssValue::List(vec![
            CssValue::String("bar".into()),
            CssValue::String("foo".into()),
        ])));
    }

    #[test]
    fn test_multipliers_between() {
        let tree = CssSyntax::new("foo bar{1,3} baz").compile().unwrap();
        assert_none!(tree.clone().matches(&CssValue::String("foo".into())));
        assert_none!(tree
            .clone()
            .matches(&CssValue::List(vec![CssValue::String("foo".into())])));
        assert_some!(tree.clone().matches(&CssValue::List(vec![
            CssValue::String("foo".into()),
            CssValue::String("bar".into()),
            CssValue::String("baz".into()),
        ])));
        assert_none!(tree.clone().matches(&CssValue::List(vec![
            CssValue::String("foo".into()),
            CssValue::String("baz".into()),
        ])));
        assert_some!(tree.clone().matches(&CssValue::List(vec![
            CssValue::String("foo".into()),
            CssValue::String("bar".into()),
            CssValue::String("bar".into()),
            CssValue::String("baz".into()),
        ])));
        assert_some!(tree.clone().matches(&CssValue::List(vec![
            CssValue::String("foo".into()),
            CssValue::String("bar".into()),
            CssValue::String("bar".into()),
            CssValue::String("bar".into()),
            CssValue::String("baz".into()),
        ])));
        assert_none!(tree.clone().matches(&CssValue::List(vec![
            CssValue::String("foo".into()),
            CssValue::String("bar".into()),
            CssValue::String("bar".into()),
            CssValue::String("bar".into()),
            CssValue::String("bar".into()),
            CssValue::String("baz".into()),
        ])));
        assert_none!(tree.clone().matches(&CssValue::List(vec![
            CssValue::String("foo".into()),
            CssValue::String("bar".into()),
            CssValue::String("bar".into()),
            CssValue::String("baz".into()),
            CssValue::String("bar".into()),
            CssValue::String("bar".into()),
        ])));

        let tree = CssSyntax::new("foo bar{0,3}").compile().unwrap();
        assert_some!(tree.clone().matches(&CssValue::String("foo".into())));
        assert_some!(tree
            .clone()
            .matches(&CssValue::List(vec![CssValue::String("foo".into())])));
        assert_some!(tree.clone().matches(&CssValue::List(vec![
            CssValue::String("foo".into()),
            CssValue::String("bar".into()),
        ])));
        assert_some!(tree.clone().matches(&CssValue::List(vec![
            CssValue::String("foo".into()),
            CssValue::String("bar".into()),
            CssValue::String("bar".into()),
        ])));
        assert_none!(tree.clone().matches(&CssValue::List(vec![
            CssValue::String("foo".into()),
            CssValue::String("bar".into()),
            CssValue::String("bar".into()),
            CssValue::String("bar".into()),
            CssValue::String("bar".into()),
        ])));
        assert_none!(tree.clone().matches(&CssValue::List(vec![
            CssValue::String("bar".into()),
            CssValue::String("foo".into()),
        ])));
    }

    #[test]
    fn test_convert_to_counts() {
        let matches = Matches {
            entries: vec![0, 1, 1, 2, 3, 1],
            all: -1,
        };

        let counts = convert_to_counts(matches);
        assert_eq!(counts, vec![(0, 1), (1, 2), (2, 1), (3, 1), (1, 1),]);

        let matches = Matches {
            entries: vec![0, 0, 0, 1, 1, 2, 2, 2, 3, 3, 3, 1, 2, 3, 4, 3, 3, 3],
            all: -1,
        };

        let counts = convert_to_counts(matches);
        assert_eq!(
            counts,
            vec![
                (0, 3),
                (1, 2),
                (2, 3),
                (3, 3),
                (1, 1),
                (2, 1),
                (3, 1),
                (4, 1),
                (3, 3),
            ]
        );
    }
}
