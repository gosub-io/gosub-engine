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
        dbg!(&self);
        dbg!(&value);
        match_component(value, &self.components[0], 0)
    }
}

fn match_component(value: &CssValue, component: &SyntaxComponent, depth: usize) -> Option<CssValue> {
    // println!("[{}]{}MATCH_INTERNAL: {:?} against {:?}", depth, "  ".repeat(depth), value, component);
    match &component {
        SyntaxComponent::GenericKeyword { keyword, .. } => match value {
            CssValue::None if keyword.eq_ignore_ascii_case("none") => return Some(value.clone()),
            CssValue::String(v) if v.eq_ignore_ascii_case(keyword) => {
                // println!("double fin.. matching keyword  {} {}", v, keyword);
                return Some(value.clone());
            }
            _ => {
                // Did not match the keyword
                // panic!("Unknown generic keyword: {:?}", keyword);
            }
        },
        SyntaxComponent::Definition { .. } => {
            panic!("Definition not implemented yet");
            // if let CssValue::String(v) = value {
            //     if v.eq_ignore_ascii_case(value) {
            //         return Some(value.clone());
            //     }
            // }
        }
        SyntaxComponent::Builtin { datatype, .. } => match datatype.as_str() {
            "percentage" => match value {
                CssValue::Percentage(_) => return Some(value.clone()),
                _ => {}
            },
            //     "number" | "<number>" => {
            //         if matches!(value, CssValue::Number(_)) {
            //             return Some(value.clone());
            //         }
            //     }
            "angle" => match value {
                CssValue::Zero => return Some(value.clone()),
                CssValue::Unit(_, u) if u.eq_ignore_ascii_case("deg") => {
                    return Some(value.clone())
                }
                CssValue::Unit(_, u) if u.eq_ignore_ascii_case("grad") => {
                    return Some(value.clone())
                }
                CssValue::Unit(_, u) if u.eq_ignore_ascii_case("rad") => {
                    return Some(value.clone())
                }
                CssValue::Unit(_, u) if u.eq_ignore_ascii_case("turn") => {
                    return Some(value.clone())
                }
                _ => {}
            },
            "hex-color" => match value {
                CssValue::Color(_) => return Some(value.clone()),
                CssValue::String(v) if v.starts_with('#') => return Some(value.clone()),
                _ => {}
            },
            "length" => match value {
                CssValue::Zero => return Some(value.clone()),
                CssValue::Unit(_, u) if LENGTH_UNITS.contains(&u.as_str()) => {
                    // println!("oh fun.. we matched length: {}", value.clone());
                    return Some(value.clone());
                }
                _ => {}
            },
            _ => panic!("Unknown built-in datatype: {:?}", datatype),
        },
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
                        // println!("matched the unit");
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
                GroupCombinators::Juxtaposition => {
                    match_group_juxtaposition(value, components, depth)
                }
                GroupCombinators::AllAnyOrder => {
                    match_group_all_any_order(value, components, depth)
                }
                GroupCombinators::AtLeastOneAnyOrder => {
                    match_group_at_least_one_any_order(value, components, depth)
                }
                GroupCombinators::ExactlyOne => match_group_exactly_one(value, components, depth),
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
        SyntaxComponent::Value {
            value: css_value, ..
        } => {
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


/// Returns element if exactly one element matches in the group
fn match_group_exactly_one(
    _value: &CssValue,
    _components: &Vec<SyntaxComponent>,
    _depth: usize,
) -> Option<CssValue> {
    todo!("implement me")
}

/// Returns element, when at least one of the elements in the group matches
fn match_group_at_least_one_any_order(
    _value: &CssValue,
    _components: &Vec<SyntaxComponent>,
    _depth: usize,
) -> Option<CssValue> {
    todo!("implement me")
}

fn match_group_all_any_order(
    _value: &CssValue,
    _components: &Vec<SyntaxComponent>,
    _depth: usize,
) -> Option<CssValue> {
    todo!("implement me")
}

fn match_group_juxtaposition(
    value: &CssValue,
    components: &Vec<SyntaxComponent>,
    depth: usize,
) -> Option<CssValue> {
    // Component index we are currently matching against
    let mut c_idx = 0;
    // Value index we are currently matching against
    let mut v_idx = 0;
    // Values as vec[]
    let values = value.as_vec();

    let mut multiplier_count = 0;
    loop {
        let v = values.get(v_idx).unwrap();
        let component = &components[c_idx];
        print!("value '{:?}' against '{:?}': ", v, component);

        if match_component(v, component, depth + 1).is_some() {
            print!("matches: ");
            multiplier_count += 1;

            let mff = multiplier_fulfilled(component, multiplier_count);
            println!("multiplier {} fulfilled: {:?}", multiplier_count, mff);

            match mff {
                Fulfillment::NotYetFulfilled => {
                    // The multiplier is not yet fulfilled. We need more values
                    v_idx += 1;
                },
                Fulfillment::FulfilledButMoreAllowed => {
                    // More elements are allowed. Let's check if we have one
                    v_idx += 1;
                    multiplier_count = 0;
                },
                Fulfillment::Fulfilled => {
                    // no more values are allowed. Continue with the next elements.
                    c_idx += 1;
                    v_idx += 1;
                    multiplier_count = 0;
                },
                Fulfillment::NotFulfilled => {
                    // The multiplier is not fulfilled. This is a failure
                    break;
                },
            }
        } else {
            // Element didn't match
            println!("no match");

            match multiplier_fulfilled(component, 0) {
                Fulfillment::NotYetFulfilled => {
                    println!("needed a match and found none (notyetfulfilled)");
                    break;
                }
                Fulfillment::Fulfilled => {
                    println!("multiplier fulfilled");
                    v_idx += 1;
                    multiplier_count = 0;
                }
                Fulfillment::FulfilledButMoreAllowed => {
                    println!("multiplier fulfilled, more values allowed, but this wasn't one of them.");
                    c_idx += 1;
                }
                Fulfillment::NotFulfilled => {
                    println!("needed a match and found none (notfulfilled)");
                    break;
                }
            }
        }

        // Reached the end of either components or values
        if c_idx >= components.len() || v_idx >= values.len() {
            break;
        }
    }

    println!("Group checks follow (cidx: {} vidx: {})", c_idx, v_idx);

    while c_idx < components.len() {
        println!("Not all components have been checked");
        let component = &components[c_idx];
        match multiplier_fulfilled(component, 0) {
            Fulfillment::NotYetFulfilled => {
                println!(" - Multiplier not yet fulfilled");
                return None;
            },
            Fulfillment::Fulfilled => {
                println!(" - Multiplier fulfilled");
            },
            Fulfillment::FulfilledButMoreAllowed => {
                println!(" - Multiplier fulfilled, but more values allowed");
            },
            Fulfillment::NotFulfilled => {
                println!(" - Multiplier not fulfilled");
                return None;
            }
        }

        c_idx += 1;
    }

    if v_idx < values.len() {
        println!(" - Not all values have been checked");
        return None;
    }

    Some(value.clone())
}

#[derive(Debug)]
enum Fulfillment {
    /// The multiplier is not yet fulfilled. There must be more values
    NotYetFulfilled,
    /// The multiplier is fulfilled. There cannot be any more values
    Fulfilled,
    /// The multiplied is fulfilled, but there may be more values added
    FulfilledButMoreAllowed,
    /// The multiplier is not fulfilled (ie: too many values).
    NotFulfilled,
}

/// Returns true when the given cnt fullfills the multiplier of the component
fn multiplier_fulfilled(component: &SyntaxComponent, cnt: usize) -> Fulfillment {
    for m in component.get_multipliers() {
        match m {
            SyntaxComponentMultiplier::Once => {
                match cnt {
                    0 => return Fulfillment::NotYetFulfilled,
                    1 => return Fulfillment::Fulfilled,
                    _ => return Fulfillment::NotFulfilled,
                }
            },
            SyntaxComponentMultiplier::ZeroOrMore => {
                match cnt {
                    _ => return Fulfillment::FulfilledButMoreAllowed,
                }
            },
            SyntaxComponentMultiplier::OneOrMore => {
                match cnt {
                    0 => return Fulfillment::NotYetFulfilled,
                    _ => return Fulfillment::FulfilledButMoreAllowed,
                }
            },
            SyntaxComponentMultiplier::Optional => {
                match cnt {
                    0 => return Fulfillment::FulfilledButMoreAllowed,
                    1 => return Fulfillment::Fulfilled,
                    _ => return Fulfillment::NotFulfilled,
                }
            },
            SyntaxComponentMultiplier::Between(from, to) => {
                match cnt {
                    _ if cnt <= from => return Fulfillment::NotYetFulfilled,
                    _ if cnt >= from && cnt <= to => return Fulfillment::FulfilledButMoreAllowed,
                    _ => return Fulfillment::NotFulfilled,
                }
            },
            SyntaxComponentMultiplier::AtLeastOneValue => {
                match cnt {
                    0 => return Fulfillment::NotYetFulfilled,
                    _ => return Fulfillment::FulfilledButMoreAllowed,
                }
            },
            SyntaxComponentMultiplier::CommaSeparatedRepeat(from, to) => {
                match cnt {
                    _ if cnt <= from => return Fulfillment::NotYetFulfilled,
                    _ if cnt >= from && cnt <= to => return Fulfillment::FulfilledButMoreAllowed,
                    _ => return Fulfillment::NotFulfilled,
                }
            }
        }
    }

    Fulfillment::NotFulfilled
}

#[cfg(test)]
mod tests {
    use gosub_css3::stylesheet::CssValue;

    use crate::css_definitions::{parse_definition_files, PropertyDefinition};
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
    fn test_match_group_juxtaposition() {
        let tree = CssSyntax::new("auto none block").compile().unwrap();
        if let SyntaxComponent::Group { components, .. } = &tree.components[0] {
            let res = match_group_juxtaposition(&CssValue::List(vec![str!("auto")]), components, 0);
            assert_none!(res);

            let res = match_group_juxtaposition(
                &CssValue::List(vec![str!("auto"), str!("none")]),
                components,
                0,
            );
            assert_none!(res);

            let res = match_group_juxtaposition(
                &CssValue::List(vec![str!("auto"), str!("none"), str!("block")]),
                components,
                0,
            );
            assert_some!(res);

            let res = match_group_juxtaposition(
                &CssValue::List(vec![str!("none"), str!("block"), str!("auto")]),
                components,
                0,
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
                0,
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
                0,
            );
            assert_none!(res);
        }
    }

    #[test]
    fn test_match_group_all_any_order() {
        let tree = CssSyntax::new("auto none block").compile().unwrap();
        if let SyntaxComponent::Group { components, .. } = &tree.components[0] {
            let res = match_group_all_any_order(&CssValue::List(vec![str!("auto")]), components, 0);
            assert_none!(res);

            let res = match_group_all_any_order(
                &CssValue::List(vec![str!("auto"), str!("none")]),
                components,
                0,
            );
            assert_none!(res);

            let res = match_group_all_any_order(
                &CssValue::List(vec![str!("auto"), str!("none"), str!("block")]),
                components,
                0,
            );
            assert_some!(res);

            let res = match_group_all_any_order(
                &CssValue::List(vec![str!("none"), str!("block"), str!("auto")]),
                components,
                0,
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
                0,
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
                0,
            );
            assert_none!(res);
        }
    }

    #[test]
    fn test_match_group_at_least_one_any_order() {
        let tree = CssSyntax::new("auto none block").compile().unwrap();
        if let SyntaxComponent::Group { components, .. } = &tree.components[0] {
            let res = match_group_at_least_one_any_order(
                &CssValue::List(vec![str!("auto")]),
                components,
                0,
            );
            assert_some!(res);

            let res = match_group_at_least_one_any_order(
                &CssValue::List(vec![str!("auto"), str!("none")]),
                components,
                0,
            );
            assert_some!(res);

            let res = match_group_at_least_one_any_order(
                &CssValue::List(vec![str!("auto"), str!("none"), str!("block")]),
                components,
                0,
            );
            assert_some!(res);

            let res = match_group_at_least_one_any_order(
                &CssValue::List(vec![str!("none"), str!("block"), str!("auto")]),
                components,
                0,
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
                0,
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
                0,
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
        // assert_none!(tree.clone().matches(&CssValue::String("foo".into())));
        // assert_none!(treef
        //     .clone()
        //     .matches(&CssValue::List(vec![CssValue::String("foo".into())])));
        assert_some!(tree.clone().matches(&CssValue::List(vec![
            CssValue::String("foo".into()),
            CssValue::String("bar".into()),
            CssValue::String("baz".into()),
        ])));
        // assert_none!(tree.clone().matches(&CssValue::List(vec![
        //     CssValue::String("foo".into()),
        //     CssValue::String("baz".into()),
        // ])));
        // assert_some!(tree.clone().matches(&CssValue::List(vec![
        //     CssValue::String("foo".into()),
        //     CssValue::String("bar".into()),
        //     CssValue::String("bar".into()),
        //     CssValue::String("bar".into()),
        //     CssValue::String("bar".into()),
        //     CssValue::String("baz".into()),
        // ])));
        // assert_none!(tree.clone().matches(&CssValue::List(vec![
        //     CssValue::String("foo".into()),
        //     CssValue::String("bar".into()),
        //     CssValue::String("bar".into()),
        //     CssValue::String("bar".into()),
        //     CssValue::String("baz".into()),
        //     CssValue::String("bar".into()),
        // ])));
        //
        // let tree = CssSyntax::new("foo bar+").compile().unwrap();
        // assert_none!(tree.clone().matches(&CssValue::String("foo".into())));
        // assert_none!(tree
        //     .clone()
        //     .matches(&CssValue::List(vec![CssValue::String("foo".into())])));
        // assert_none!(tree
        //     .clone()
        //     .matches(&CssValue::List(vec![CssValue::String("bar".into())])));
        // assert_some!(tree.clone().matches(&CssValue::List(vec![
        //     CssValue::String("foo".into()),
        //     CssValue::String("bar".into()),
        // ])));
        // assert_some!(tree.clone().matches(&CssValue::List(vec![
        //     CssValue::String("foo".into()),
        //     CssValue::String("bar".into()),
        //     CssValue::String("bar".into()),
        // ])));
        // assert_none!(tree.clone().matches(&CssValue::List(vec![
        //     CssValue::String("bar".into()),
        //     CssValue::String("foo".into()),
        // ])));
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
    fn test_matcher() {
        let mut definitions = parse_definition_files();
        definitions.add_property(
            "testprop",
            PropertyDefinition {
                name: "testprop".to_string(),
                computed: vec![],
                syntax: CssSyntax::new(
                    "[ left | right ] <length>? | [ top | bottom ] <length> | [ left | bottom ]",
                )
                .compile()
                .unwrap(),
                inherited: false,
                initial_value: None,
                resolved: false,
            },
        );

        let prop = definitions.find_property("testprop").unwrap();

        assert_some!(prop.clone().matches(&CssValue::List(vec![
            CssValue::String("left".into()),
            CssValue::Unit(5.0, "px".into()),
        ])));
        assert_some!(prop.clone().matches(&CssValue::List(vec![
            CssValue::String("top".into()),
            CssValue::Unit(5.0, "px".into()),
        ])));
        assert_some!(prop.clone().matches(&CssValue::List(vec![
            CssValue::String("bottom".into()),
            CssValue::Unit(5.0, "px".into()),
        ])));
        assert_some!(prop.clone().matches(&CssValue::List(vec![
            CssValue::String("right".into()),
            CssValue::Unit(5.0, "px".into()),
        ])));
        assert_some!(prop
            .clone()
            .matches(&CssValue::List(vec![CssValue::String("left".into()),])));

        assert_some!(prop
            .clone()
            .matches(&CssValue::List(vec![CssValue::String("bottom".into()),])));

        assert_some!(prop
            .clone()
            .matches(&CssValue::List(vec![CssValue::String("right".into()),])));

        assert_none!(prop
            .clone()
            .matches(&CssValue::List(vec![CssValue::String("top".into()),])));
    }

    #[test]
    fn test_matcher_2() {
        let mut definitions = parse_definition_files();
        definitions.add_property("testprop", PropertyDefinition{
            name: "testprop".to_string(),
            computed: vec![],
            syntax: CssSyntax::new("[ [ left | center | right | top | bottom | <length-percentage> ] | [ left | center | right | <length-percentage> ] [ top | center | bottom | <length-percentage> ] ]").compile().unwrap(),
            inherited: false,
            initial_value: None,
            resolved: false,
        });
        definitions.resolve();

        let prop = definitions.find_property("testprop").unwrap();

        // assert_some!(prop.clone().matches(&CssValue::List(vec![
        //     CssValue::String("left".into()),
        // ])));
        // assert_some!(prop.clone().matches(&CssValue::List(vec![
        //     CssValue::String("left".into()),
        //     CssValue::String("top".into()),
        // ])));
        // assert_some!(prop.clone().matches(&CssValue::List(vec![
        //     CssValue::String("center".into()),
        //     CssValue::String("top".into()),
        // ])));
        assert_some!(prop.clone().matches(&CssValue::List(vec![
            CssValue::String("center".into()),
            CssValue::String("center".into()),
        ])));
        assert_some!(prop.clone().matches(&CssValue::List(vec![
            CssValue::Percentage(10.0),
            CssValue::Percentage(20.0),
        ])));
        assert_some!(prop.clone().matches(&CssValue::List(vec![
            CssValue::String("left".into()),
            CssValue::Percentage(20.0),
        ])));

        assert_some!(prop.clone().matches(&CssValue::List(vec![
            CssValue::String("center".into()),
            CssValue::Percentage(10.0),
            CssValue::String("top".into()),
            CssValue::Percentage(20.0),
        ])));

        assert_some!(prop.clone().matches(&CssValue::List(vec![
            CssValue::String("top".into()),
            CssValue::Percentage(10.0),
            CssValue::String("center".into()),
            CssValue::Percentage(20.0),
        ])));

        assert_some!(prop
            .clone()
            .matches(&CssValue::List(vec![CssValue::String("right".into()),])));

        assert_none!(prop
            .clone()
            .matches(&CssValue::List(vec![CssValue::String("top".into()),])));
    }

    #[test]
    fn test_matcher_3() {
        let mut definitions = parse_definition_files();
        definitions.add_property(
            "testprop",
            PropertyDefinition {
                name: "testprop".to_string(),
                computed: vec![],
                syntax: CssSyntax::new("foo | [ foo [ foo | bar ] ]")
                    .compile()
                    .unwrap(),
                inherited: false,
                initial_value: None,
                resolved: false,
            },
        );
        definitions.resolve();

        let prop = definitions.find_property("testprop").unwrap();

        // assert_some!(prop.clone().matches(&CssValue::List(vec![
        //     CssValue::String("foo".into()),
        // ])));

        assert_some!(prop.clone().matches(&CssValue::List(vec![
            CssValue::String("foo".into()),
            CssValue::String("foo".into()),
        ])));

        assert_some!(prop.clone().matches(&CssValue::List(vec![
            CssValue::String("foo".into()),
            CssValue::String("foo".into()),
            CssValue::String("foo".into()),
        ])));

        // assert_some!(prop.clone().matches(&CssValue::List(vec![
        //     CssValue::String("foo".into()),
        //     CssValue::String("bar".into()),
        // ])));
        //
        // assert_none!(prop.clone().matches(&CssValue::List(vec![
        //     CssValue::String("bar".into()),
        // ])));
        // assert_none!(prop.clone().matches(&CssValue::List(vec![
        //     CssValue::String("bar".into()),
        //     CssValue::String("foo".into()),
        // ])));
    }
}
