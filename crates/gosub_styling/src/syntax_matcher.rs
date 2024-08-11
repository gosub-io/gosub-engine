use gosub_css3::colors::{is_named_color, is_system_color};
use gosub_css3::stylesheet::CssValue;
use crate::syntax::{GroupCombinators, SyntaxComponent, SyntaxComponentMultiplier};

/// Structure to return from a matching function.
#[derive(Debug, Clone)]
pub struct MatchResult {
    /// The remainder of the values that are not matched.
    pub remainder: Vec<CssValue>,
    /// True when this matched did some matching (todo: we might remove this and check for matched_values.is_empty)
    pub matched: bool,
    /// List of the matched values
    pub matched_values: Vec<CssValue>,
}

#[allow(dead_code)]
const LENGTH_UNITS: [&str; 31] = [
    "cap", "ch", "em", "ex", "ic", "lh", "rcap", "rch", "rem", "rex", "ric", "rlh", "vh", "vw",
    "vmax", "vmin", "vb", "vi", "cqw", "cqh", "cqi", "cqb", "cqmin", "cqmax", "px", "cm", "mm",
    "Q", "in", "pc", "pt",
];


macro_rules! debug_print_juxta {
    ($($x:tt)*) => { println!($($x)*) }
    // ($($x:tt)*) => {{}};
}
macro_rules! debug_print_exactly {
    ($($x:tt)*) => { println!($($x)*) }
    // ($($x:tt)*) => {{}};
}

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
    pub fn matches(&self, input: Vec<CssValue>) -> bool {
        if self.components.len() != 1 {
            panic!("Syntax tree must have exactly one root component");
        }

        let res = match_component(&input, &self.components[0]);
        res.matched && res.remainder.is_empty()
    }
}

/// Matches a single component against the input values. After the match, there might be remaining
/// elements in the input. This is passed back in the MatchResult structure.
fn match_component(input: &Vec<CssValue>, component: &SyntaxComponent) -> MatchResult {
    // dbg!(&input);
    // dbg!(&component);

    if input.is_empty() {
        // println!("Input is empty. So we don't match anything");
        return no_match(input);
    }

    // Get the first value from the input which we will use for matching
    let value = input.get(0).unwrap();
    // println!("\n\nmatch_component: {:?} against {:?}", value, component);

    match &component {
        SyntaxComponent::GenericKeyword { keyword, .. } => match value {
            CssValue::None if keyword.eq_ignore_ascii_case("none") => {
                return first_match(input);
            }
            CssValue::String(v) if v.eq_ignore_ascii_case(keyword) => {
                // println!("keyword {:?} match!", v);
                return first_match(input);
            }
            _ => {}
        },
        SyntaxComponent::Definition { .. } => {
            //dbg!(&component);
            todo!("Definition not implemented yet");
        }
        SyntaxComponent::Builtin { datatype, .. } => match datatype.as_str() {
            "percentage" => match value {
                CssValue::Percentage(_) => return first_match(input),
                _ => {}
            },
            "angle" => match value {
                CssValue::Zero => return first_match(input),
                CssValue::Unit(_, u) if u.eq_ignore_ascii_case("deg") => return first_match(input),
                CssValue::Unit(_, u) if u.eq_ignore_ascii_case("grad") => {
                    return first_match(input)
                }
                CssValue::Unit(_, u) if u.eq_ignore_ascii_case("rad") => return first_match(input),
                CssValue::Unit(_, u) if u.eq_ignore_ascii_case("turn") => {
                    return first_match(input)
                }
                _ => {}
            },
            "length" => match value {
                CssValue::Zero => return first_match(input),
                CssValue::Unit(_, u) if LENGTH_UNITS.contains(&u.as_str()) => {
                    return first_match(input)
                }
                _ => {}
            },
            "system-color" => match value {
                CssValue::String(v) => {
                    if is_system_color(v) {
                        return first_match(input);
                    }
                }
                _ => {}
            },
            "named-color" => match value {
                CssValue::String(v) => {
                    if is_named_color(v) {
                        return first_match(input);
                    }
                }
                _ => {}
            },
            "color()" => match value {
                // @TODO: fix this according to what the spec says
                CssValue::Color(_) => return first_match(input),
                CssValue::String(v) if v.starts_with('#') => return first_match(input),
                _ => {}
            },
            "hex-color" => match value {
                CssValue::Color(_) => return first_match(input),
                CssValue::String(v) if v.starts_with('#') => return first_match(input),
                _ => {}
            },
            _ => panic!("Unknown built-in datatype: {:?}", datatype),
        },
        SyntaxComponent::Inherit { .. } => match value {
            CssValue::Inherit => return first_match(input),
            CssValue::String(v) if v.eq_ignore_ascii_case("inherit") => return first_match(input),
            _ => {}
        },
        SyntaxComponent::Initial { .. } => match value {
            CssValue::Initial => return first_match(input),
            CssValue::String(v) if v.eq_ignore_ascii_case("initial") => return first_match(input),
            _ => {}
        },
        SyntaxComponent::Unset { .. } => match value {
            CssValue::String(v) if v.eq_ignore_ascii_case("unset") => return first_match(input),
            _ => {}
        },
        SyntaxComponent::Unit { from, to, unit, .. } => {
            let f32min = f32::MIN;
            let f32max = f32::MAX;

            match value {
                CssValue::Number(n) if *n == 0.0 => return first_match(input),
                CssValue::Unit(n, u) => {
                    if unit.contains(u)
                        && *n >= from.unwrap_or(f32min)
                        && *n <= to.unwrap_or(f32max)
                    {
                        return first_match(input);
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
            // println!("We need to do a group match on {:?}, our value is: {:?}", combinator, input);

            let result = match combinator {
                GroupCombinators::Juxtaposition => match_group_juxtaposition(input, components),
                GroupCombinators::AllAnyOrder => match_group_all_any_order(input, components),
                GroupCombinators::AtLeastOneAnyOrder => {
                    match_group_at_least_one_any_order(input, components)
                }
                GroupCombinators::ExactlyOne => match_group_exactly_one(input, components),
            };

            // println!("Did a group match. This is the result for {:?}: ", combinator);
            // dbg!(&result);

            return result;
        }
        SyntaxComponent::Literal { literal, .. } => match value {
            CssValue::String(v) if v.eq(literal) => return first_match(input),
            CssValue::String(v) if v.eq_ignore_ascii_case(literal) => {
                log::warn!("Case insensitive literal matched");
                return first_match(input);
            }
            _ => {}
        },
        SyntaxComponent::Function { name, .. } => {
            let CssValue::Function(c_name, c_args) = value else {
                return no_match(input);
            };

            if !name.eq_ignore_ascii_case(c_name) {
                return no_match(input);
            }

            if c_args.is_empty() {
                return first_match(input);
            }

            todo!("Function not implemented yet. We must match the arguments");
            // let list = CssValue::List(c_args.clone());
            // return match_internal(&list, arguments);
        }
        SyntaxComponent::Value {
            value: css_value, ..
        } => {
            if value == css_value {
                return first_match(input);
            }
        }
        e => {
            panic!("Unknown syntax component: {:?}", e);
        }
    }

    no_match(input)
}

/// Returns element if exactly one element matches in the group
fn match_group_exactly_one(
    raw_input: &Vec<CssValue>,
    components: &Vec<SyntaxComponent>,
) -> MatchResult {
    let gid = rand::random::<u8>();

    let mut input = raw_input.to_vec();
    // Component index we are currently matching against
    let mut c_idx = 0;
    // The values that were matched
    let mut matched_values = vec![];
    // List of components that are already matched against previous values
    let mut components_matched = vec![];

    debug_print_exactly!("[{}] *** Matching Group Exactly One", gid);
    let mut multiplier_count = 0;
    loop {
        let component = &components[c_idx];
        debug_print_exactly!("[{}] *** Input '{:?}' against '{:?}': ", gid, input, component);

        let res = match_component(&input, component);
        // dbg!(&res);
        // if res.matched && res.remainder.is_empty() {
        if res.matched {
            debug_print_exactly!("[{}] *** matches: ", gid);
            multiplier_count += 1;

            matched_values.append(&mut res.matched_values.clone());

            let mff = multiplier_fulfilled(component, multiplier_count);
            debug_print_exactly!("[{}] *** multiplier {} fulfilled: {:?}", gid, multiplier_count, mff);

            match mff {
                Fulfillment::NotYetFulfilled => {
                    debug_print_exactly!("[{}] *** and not yet fulfilled", gid);
                    // The multiplier is not yet fulfilled. We need more values so check the next
                    input = res.remainder.clone();
                }
                Fulfillment::FulfilledButMoreAllowed => {
                    debug_print_exactly!("[{}] *** fulfilled but more allowed", gid);
                    // More elements are allowed. Let's check if we have one
                    input = res.remainder.clone();
                }
                Fulfillment::Fulfilled => {
                    debug_print_exactly!("[{}] *** and fulfilled", gid);
                    // no more values are allowed. Continue with the next value and element

                    components_matched.push((c_idx, res.matched_values, res.remainder));
                    c_idx += 1;
                    multiplier_count = 0;
                    input = raw_input.to_vec();
                }
                Fulfillment::NotFulfilled => {
                    debug_print_exactly!("[{}] *** and not fulfilled.", gid);

                    // The multiplier is not fulfilled. Just continue with the next element
                    c_idx += 1;
                    multiplier_count = 0;
                    input = raw_input.to_vec();
                }
            }
        } else {
            // Element didn't match. That might be allright depending on the multiplier
            debug_print_exactly!("[{}] *** not matched", gid);

            match multiplier_fulfilled(component, multiplier_count) {
                Fulfillment::NotYetFulfilled => {
                    debug_print_exactly!("[{}] *** not yet fulfilled. That's ok. Just check the next element", gid);
                    c_idx += 1;
                    multiplier_count = 0;
                    input = raw_input.to_vec();
                }
                Fulfillment::Fulfilled => {
                    // matched_values.append(&mut res.matched_values.clone()); //TODO: i don't think, we should push this to matched_values?
                    // input = res.remainder.clone();

                    c_idx += 1;
                    multiplier_count = 0;
                    input = raw_input.to_vec();
                }
                Fulfillment::FulfilledButMoreAllowed => {
                    debug_print_exactly!("[{}] *** multiplier fulfilled, more values allowed, but this wasn't one of them.", gid);

                    c_idx += 1;
                    multiplier_count = 0;
                    input = raw_input.to_vec();
                }
                Fulfillment::NotFulfilled => {
                    debug_print_exactly!("[{}] *** needed a match and found none. That's ok, just check the next element", gid);
                    c_idx += 1;
                    multiplier_count = 0;
                    input = raw_input.to_vec();
                }
            }
        }

        // End of input, so break
        if input.is_empty() {
            debug_print_exactly!("[{}] *** input is empty. We don't know if we need to break here or not", gid);
            break; //TODO: I don't think break is correct here, it probably should just say that the current component doesn't match and go to the next one.
        }

        // Reached the end of either components or values
        if c_idx >= components.len() {
            debug_print_exactly!("[{}] *** At the end of components. Breaking", gid);
            break;
        }

        debug_print_exactly!("[{}] *** relooping with the next component", gid);
    }

    // let _dbg = format!("{:#?}", &components_matched);
    // dbg!(&components_matched);

    // let component = components_matched
    //     .into_iter()
    //     .filter(|c| c.1.is_empty())
    //     .collect::<Vec<_>>();
    //
    // dbg!(&component);
    if components_matched.len() != 1 {
        debug_print_exactly!("[{}] *** Matched components is not 1.", gid);
        dbg!(&components_matched);
        return no_match(&input);
    }

    debug_print_exactly!("[{}] *** Matched exactly one value", gid);

    let res = MatchResult {
        remainder: components_matched[0].2.clone(),
        matched: true,
        matched_values: components_matched[0].1.clone(),
    };
    // dbg!(&res);
    res
}

/// Returns element, when at least one of the elements in the group matches
fn match_group_at_least_one_any_order(
    input: &Vec<CssValue>,
    components: &Vec<SyntaxComponent>,
) -> MatchResult {
    let mut input = input.to_vec();

    // Component index we are currently matching against
    let mut c_idx = 0;
    // List of components that are already matched against previous values
    let mut components_matched = vec![];
    let mut matched_values = vec![];

    let mut multiplier_count = 0;
    loop {
        let component = &components[c_idx];
        // println!("value '{:?}' against '{:?}': ", input, component);

        let res = match_component(&input, component);
        if res.matched {
            // println!("matches: ");
            multiplier_count += 1;

            let mff = multiplier_fulfilled(component, multiplier_count);
            // println!("multiplier {} fulfilled: {:?}", multiplier_count, mff);

            match mff {
                Fulfillment::NotYetFulfilled => {
                    // The multiplier is not yet fulfilled. We need more values so check the next
                    matched_values.append(&mut res.matched_values.clone());
                    input = res.remainder.clone();
                }
                Fulfillment::FulfilledButMoreAllowed => {
                    matched_values.append(&mut res.matched_values.clone());
                    input = res.remainder.clone();
                }
                Fulfillment::Fulfilled => {
                    // no more values are allowed. Continue with the next value and element
                    matched_values.append(&mut res.matched_values.clone());
                    input = res.remainder.clone();

                    // Loop around
                    // println!("-- loop around");
                    components_matched.push(c_idx);
                    c_idx = 0;
                    while components_matched.contains(&c_idx) {
                        // println!("component {} has already been matched", c_idx);
                        c_idx += 1;
                    }

                    multiplier_count = 0;
                }
                Fulfillment::NotFulfilled => {
                    // The multiplier is not fulfilled. This is a failure
                    return no_match(&input);
                }
            }
        } else {
            // Element didn't match. That might be allright depending on the multiplier
            // println!("no match");

            c_idx += 1;
            while components_matched.contains(&c_idx) {
                // println!("component {} has already been matched", c_idx);
                c_idx += 1;
            }
        }

        // Reached the end of components
        if c_idx >= components.len() {
            break;
        }
    }

    if components_matched.is_empty() {
        // println!(" - No components have been matched");
        return no_match(&input);
    }
    // dbg!(&matched_values);

    // println!("Match at least one any order is valid.");

    return MatchResult {
        remainder: input.clone(),
        matched: true,
        matched_values,
    };
}

fn match_group_all_any_order(
    input: &Vec<CssValue>,
    components: &Vec<SyntaxComponent>,
) -> MatchResult {
    let mut input = input.to_vec();
    // Component index we are currently matching against
    let mut c_idx = 0;
    // List of components that are already matched against previous values
    let mut components_matched = vec![];
    let mut matched_values = vec![];

    let mut multiplier_count = 0;
    loop {
        let component = &components[c_idx];
        // println!("value '{:?}' against '{:?}': ", input, component);

        let res = match_component(&input, component);
        if res.matched {
            // println!("matches: ");
            multiplier_count += 1;

            let mff = multiplier_fulfilled(component, multiplier_count);
            // println!("multiplier {} fulfilled: {:?}", multiplier_count, mff);

            match mff {
                Fulfillment::NotYetFulfilled => {
                    // The multiplier is not yet fulfilled. We need more values so check the next
                    matched_values.append(&mut res.matched_values.clone());
                    input = res.remainder.clone();
                }
                Fulfillment::FulfilledButMoreAllowed => {
                    // More elements are allowed. Let's check if we have one
                    matched_values.append(&mut res.matched_values.clone());
                    input = res.remainder.clone();
                }
                Fulfillment::Fulfilled => {
                    // no more values are allowed. Continue with the next value and element
                    matched_values.append(&mut res.matched_values.clone());
                    input = res.remainder.clone();

                    // Loop around
                    // println!("-- loop around");
                    components_matched.push(c_idx);
                    c_idx = 0;
                    while components_matched.contains(&c_idx) {
                        // println!("component {} has already been matched", c_idx);
                        c_idx += 1;
                    }

                    multiplier_count = 0;
                }
                Fulfillment::NotFulfilled => {
                    // The multiplier is not fulfilled. This is a failure
                    break;
                }
            }
        } else {
            // Element didn't match. That might be allright depending on the multiplier
            // println!("no match");

            c_idx += 1;
            while components_matched.contains(&c_idx) {
                // println!("component {} has already been matched", c_idx);
                c_idx += 1;
            }
        }

        // Reached the end of components
        if c_idx >= components.len() {
            break;
        }
    }

    // println!("Group checks follow (cidx: {} vidx: {})", c_idx, v_idx);

    while c_idx < components.len() {
        // println!("Not all components have been checked");
        let component = &components[c_idx];
        match multiplier_fulfilled(component, multiplier_count) {
            Fulfillment::NotYetFulfilled => {
                // println!(" - Multiplier not yet fulfilled");
                return no_match(&input);
            }
            Fulfillment::Fulfilled => {
                // println!(" - Multiplier fulfilled");
            }
            Fulfillment::FulfilledButMoreAllowed => {
                // println!(" - Multiplier fulfilled, but more values allowed");
            }
            Fulfillment::NotFulfilled => {
                // println!(" - Multiplier not fulfilled");
                return no_match(&input);
            }
        }

        c_idx += 1;
        multiplier_count = 0;
    }

    if components.len() != components_matched.len() {
        // println!(" - Not all components have been matched");
        return no_match(&input);
    }

    // println!("Match all_any_order is valid.");
    MatchResult {
        remainder: input.clone(),
        matched: true,
        matched_values,
    }
}

fn match_group_juxtaposition(
    input: &Vec<CssValue>,
    components: &Vec<SyntaxComponent>,
) -> MatchResult {
    let gid = rand::random::<u8>();
    debug_print_juxta!("[{}]+++ Entering Match group juxtaposition", gid);

    let mut input = input.to_vec();
    let mut c_idx = 0;
    let mut matched_values = vec![];

    let mut multiplier_count = 0;
    loop {
        if input.is_empty() {
            debug_print_juxta!("[{}]+++ input is empty. Done with matching", gid);
            break;
        }

        let component = &components[c_idx];
        debug_print_juxta!("[{}]+++ value '{:?}' against component", gid, input);

        let res = match_component(&input, component);
        if res.matched {
            debug_print_juxta!("[{}]+++ matches: ", gid);
            multiplier_count += 1;

            dbg!(&res);

            // Add matched elements to matched_values
            matched_values.append(&mut res.matched_values.clone());
            input = res.remainder.clone();

            let mff = multiplier_fulfilled(component, multiplier_count);
            debug_print_juxta!("[{}]+++ multiplier {} fulfilled: {:?}", gid, multiplier_count, mff);

            match mff {
                Fulfillment::NotYetFulfilled => {
                    // The multiplier is not yet fulfilled. We need more values so check the next
                }
                Fulfillment::FulfilledButMoreAllowed => {
                    // More elements are allowed. Let's check if we have one
                }
                Fulfillment::Fulfilled => {
                    // no more values are allowed. Continue with the next value and element
                    c_idx += 1;
                    multiplier_count = 0;
                }
                Fulfillment::NotFulfilled => {
                    // The multiplier is not fulfilled. This is a failure
                    break;
                }
            }
        } else {
            // Element didn't match. That might be allright depending on the multiplier
            debug_print_juxta!("[{}]+++ no match", gid);

            match multiplier_fulfilled(component, multiplier_count) {
                Fulfillment::NotYetFulfilled => {
                    debug_print_juxta!("[{}]+++ needed a match and found none (notyetfulfilled)", gid);
                    break;
                }
                Fulfillment::Fulfilled => {
                    debug_print_juxta!("[{}]+++ multiplier fulfilled", gid);

                    // Add matched elements to matched_values
                    matched_values.append(&mut res.matched_values.clone());
                    input = res.remainder.clone();

                    c_idx += 1;
                    multiplier_count = 0;
                }
                Fulfillment::FulfilledButMoreAllowed => {
                    debug_print_juxta!("[{}]+++ multiplier fulfilled, more values allowed, but this wasn't one of them.", gid);
                    c_idx += 1;
                    multiplier_count = 0;
                }
                Fulfillment::NotFulfilled => {
                    debug_print_juxta!("[{}]+++ needed a match and found none (notfulfilled)", gid);
                    break;
                }
            }
        }

        // Reached the end of either components to check
        if c_idx >= components.len() {
            break;
        }
    }

    // If we have broken out of the loop early, we need to check if all remaining components can
    // be fulfilled. These fulfillments should be optional as there are.
    while c_idx < components.len() {
        debug_print_juxta!("[{}]+++ Not all components have been checked", gid);
        let component = &components[c_idx];
        match multiplier_fulfilled(component, multiplier_count) {
            Fulfillment::NotYetFulfilled => {
                debug_print_juxta!("[{}]+++ - Multiplier not yet fulfilled. Juxta didn't had enough values", gid);
                return no_match(&input);
            }
            Fulfillment::Fulfilled => {
                debug_print_juxta!("[{}]+++ - Multiplier fulfilled. This is good", gid);
            }
            Fulfillment::FulfilledButMoreAllowed => {
                debug_print_juxta!("[{}]+++ - Multiplier fulfilled, but more values allowed", gid);
            }
            Fulfillment::NotFulfilled => {
                debug_print_juxta!("[{}]+++ - Multiplier not fulfilled. Juxta not met", gid);
                return no_match(&input);
            }
        }

        // Note that multiplier_count is zero at the SECOND iteration of the loop. The first iteration
        // is to check the last value of the input against the last component.
        c_idx += 1;
        multiplier_count = 0;
    }

    debug_print_juxta!("[{}]+++ Match juxtaposition is valid. Return value is : {:?}", gid, matched_values);
    return MatchResult {
        remainder: input.clone(),
        matched: true,
        matched_values,
    };
}

/// Fulfillment is a result returned by the multiplier_fulfilled function. This is used to determine
/// if a multiplier is fulfilled or not and how.
#[derive(Debug, PartialEq)]
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

/// Returns fulfillment enum given the cnt and the actual multiplier of the component
fn multiplier_fulfilled(component: &SyntaxComponent, cnt: usize) -> Fulfillment {
    for m in component.get_multipliers() {
        return match m {
            SyntaxComponentMultiplier::Once => match cnt {
                0 => Fulfillment::NotYetFulfilled,
                1 => Fulfillment::Fulfilled,
                _ => Fulfillment::NotFulfilled,
            },
            SyntaxComponentMultiplier::ZeroOrMore => match cnt {
                _ => Fulfillment::FulfilledButMoreAllowed,
            },
            SyntaxComponentMultiplier::OneOrMore => match cnt {
                0 => Fulfillment::NotYetFulfilled,
                _ => Fulfillment::FulfilledButMoreAllowed,
            },
            SyntaxComponentMultiplier::Optional => match cnt {
                0 => Fulfillment::FulfilledButMoreAllowed,
                1 => Fulfillment::Fulfilled,
                _ => Fulfillment::NotFulfilled,
            },
            SyntaxComponentMultiplier::Between(from, to) => match cnt {
                _ if cnt < from => Fulfillment::NotYetFulfilled,
                _ if cnt >= from && cnt <= to => Fulfillment::FulfilledButMoreAllowed,
                _ => Fulfillment::NotFulfilled,
            },
            SyntaxComponentMultiplier::AtLeastOneValue => match cnt {
                0 => Fulfillment::NotYetFulfilled,
                _ => Fulfillment::FulfilledButMoreAllowed,
            },
            SyntaxComponentMultiplier::CommaSeparatedRepeat(from, to) => match cnt {
                _ if cnt <= from => Fulfillment::NotYetFulfilled,
                _ if cnt >= from && cnt <= to => Fulfillment::FulfilledButMoreAllowed,
                _ => Fulfillment::NotFulfilled,
            },
        };
    }

    Fulfillment::NotFulfilled
}

/// Helper function to return no matches
fn no_match(input: &Vec<CssValue>) -> MatchResult {
    return MatchResult {
        remainder: input.clone(),
        matched: false,
        matched_values: vec![],
    };
}

/// Helper function to return the first element from input in a match result, as we need this a lot
fn first_match(input: &Vec<CssValue>) -> MatchResult {
    return MatchResult {
        remainder: input.into_iter().skip(1).cloned().collect(),
        matched: true,
        matched_values: vec![input.get(0).unwrap().clone()],
    };
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

    macro_rules! assert_match {
        ($e:expr) => {
            println!("\n\n-------- ASSERT MATCH --------");
            let res = $e.clone();
            dbg!(&res);
            assert_eq!(true, res.matched);
            println!("------------------------------\n\n");
        };
    }

    macro_rules! assert_not_match {
        ($e:expr) => {
            println!("\n\n------- ASSERT NOT MATCH ------");
            let res = $e;
            dbg!(&res);
            assert_eq!(false, res.matched);
            println!("------------------------------\n\n");
        };
    }

    macro_rules! assert_true {
        ($e:expr) => {
            assert_eq!(true, $e);
        };
    }

    macro_rules! assert_false {
        ($e:expr) => {
            assert_eq!(false, $e);
        };
    }

    #[test]
    fn test_match_group1() {
        // Exactly one
        let tree = CssSyntax::new("auto | none | block").compile().unwrap();
        assert_true!(tree.matches(vec![str!("auto")]));
        assert_true!(tree.matches(vec![CssValue::None]));
        assert_true!(tree.matches(vec![str!("block")]));
        assert_false!(tree.matches(vec![str!("inline")]));
        assert_false!(tree.matches(vec![str!("")]));
        assert_false!(tree.matches(vec![str!("foobar")]));
        assert_false!(tree.matches(vec![str!("foo"), CssValue::None]));
        assert_false!(tree.matches(vec![CssValue::None, str!("foo")]));
        assert_false!(tree.matches(vec![str!("auto"), CssValue::None]));
        assert_false!(tree.matches(vec![str!("auto"), CssValue::Comma, str!("none"),]));
        assert_false!(tree.matches(vec![
            str!("auto"),
            CssValue::Comma,
            CssValue::None,
            CssValue::Comma,
            str!("block"),
        ]));
    }

    #[test]
    fn test_match_group2() {
        // juxtaposition
        let tree = CssSyntax::new("auto none block").compile().unwrap();
        assert_false!(tree.matches(vec![str!("auto")]));
        assert_false!(tree.matches(vec![CssValue::None]));
        assert_false!(tree.matches(vec![str!("block")]));
        assert_true!(tree.matches(vec![str!("auto"), CssValue::None, str!("block"),]));
        assert_false!(tree.matches(vec![str!("block"), CssValue::None, str!("block"),]));
        assert_false!(tree.matches(vec![str!("auto"), CssValue::None, str!("auto"),]));
    }

    #[test]
    fn test_match_group3() {
        // all any order
        let tree = CssSyntax::new("auto && none && block").compile().unwrap();
        assert_false!(tree.matches(vec![str!("auto")]));
        assert_false!(tree.matches(vec![CssValue::None]));
        assert_false!(tree.matches(vec![str!("block")]));
        assert_false!(tree.matches(vec![str!("inline")]));
        assert_false!(tree.matches(vec![str!("")]));
        assert_false!(tree.matches(vec![str!("foobar")]));
        assert_false!(tree.matches(vec![str!("foo"), CssValue::None]));
        assert_false!(tree.matches(vec![CssValue::None, str!("foo")]));
        assert_false!(tree.matches(vec![str!("auto"), CssValue::None]));
        assert_false!(tree.matches(vec![str!("auto"), CssValue::Comma, str!("none")]));
        assert_false!(tree.matches(vec![
            str!("auto"),
            CssValue::Comma,
            CssValue::None,
            CssValue::Comma,
            str!("block")
        ]));
        assert_true!(tree.matches(vec![str!("block"), str!("auto"), CssValue::None]));
        assert_true!(tree.matches(vec![str!("auto"), str!("block"), CssValue::None]));
        assert_true!(tree.matches(vec![str!("block"), CssValue::None, str!("auto")]));
        assert_true!(tree.matches(vec![CssValue::None, str!("auto"), str!("block")]));
        assert_false!(tree.matches(vec![str!("auto"), str!("block")]));
        assert_false!(tree.matches(vec![CssValue::None, str!("block")]));
        assert_false!(tree.matches(vec![
            str!("block"),
            str!("block"),
            CssValue::None,
            CssValue::None
        ]));
    }

    #[test]
    fn test_match_group4() {
        // At least one in any order
        let tree = CssSyntax::new("auto || none || block").compile().unwrap();
        assert_true!(tree.matches(vec![str!("auto")]));
        assert_true!(tree.matches(vec![CssValue::None]));
        assert_true!(tree.matches(vec![str!("block")]));
        assert_true!(tree.matches(vec![str!("auto"), CssValue::None]));
        assert_true!(tree.matches(vec![str!("block"), str!("auto"), CssValue::None,]));

        assert_false!(tree.matches(vec![str!("inline")]));
        assert_false!(tree.matches(vec![str!("")]));
        assert_false!(tree.matches(vec![str!("foo"), CssValue::None]));
        assert_false!(tree.matches(vec![CssValue::None, str!("foo")]));
        assert_false!(tree.matches(vec![CssValue::None, CssValue::None,]));
        assert_false!(tree.matches(vec![str!("auto"), CssValue::Comma, str!("none"),]));
        assert_false!(tree.matches(vec![
            str!("auto"),
            CssValue::Comma,
            CssValue::None,
            CssValue::Comma,
            str!("block"),
        ]));
        assert_false!(tree.matches(vec![
            str!("block"),
            str!("block"),
            CssValue::None,
            CssValue::None,
        ]));
    }

    #[test]
    fn test_match_group_juxtaposition() {
        let tree = CssSyntax::new("auto none block").compile().unwrap();
        if let SyntaxComponent::Group { components, .. } = &tree.components[0] {
            let res = match_group_juxtaposition(&vec![str!("auto")], components);
            assert_not_match!(res);

            let res = match_group_juxtaposition(&vec![str!("auto"), str!("none")], components);
            assert_not_match!(res);

            let res = match_group_juxtaposition(
                &vec![str!("auto"), str!("none"), str!("block")],
                components,
            );
            assert_match!(res);

            let res = match_group_juxtaposition(
                &vec![str!("none"), str!("block"), str!("auto")],
                components,
            );
            assert_not_match!(res);

            let res = match_group_juxtaposition(
                &vec![
                    str!("none"),
                    str!("block"),
                    str!("block"),
                    str!("auto"),
                    str!("none"),
                ],
                components,
            );
            assert_not_match!(res);

            let res = match_group_juxtaposition(
                &vec![str!("none"), str!("banana"), str!("car"), str!("block")],
                components,
            );
            assert_not_match!(res);
        }
    }

    #[test]
    fn test_match_group_juxtaposition_with_groups() {
        // Test if groups are working icw juxtaposition
        let tree = CssSyntax::new("[top | bottom] [ up | down ] [ charm | strange] ").compile().unwrap();
        if let SyntaxComponent::Group { components, .. } = &tree.components[0] {
            let res = match_group_juxtaposition(&vec![
                str!("top"),
                str!("up"),
                str!("strange")
            ], components);
            assert_match!(res);

            let res = match_group_juxtaposition(&vec![
                str!("bottom"),
                str!("up"),
                str!("strange")
            ], components);
            assert_match!(res);

            let res = match_group_juxtaposition(&vec![
                str!("bottom"),
                str!("down"),
                str!("charm")
            ], components);
            assert_match!(res);
        }
    }

    #[test]
    fn test_match_group_all_any_order() {
        let tree = CssSyntax::new("auto none block").compile().unwrap();
        if let SyntaxComponent::Group { components, .. } = &tree.components[0] {
            let res = match_group_all_any_order(&vec![str!("auto")], components);
            assert_not_match!(res);

            let res = match_group_all_any_order(&vec![str!("auto"), str!("none")], components);
            assert_not_match!(res);

            let res = match_group_all_any_order(
                &vec![str!("auto"), str!("none"), str!("block")],
                components,
            );
            assert_match!(res);

            let res = match_group_all_any_order(
                &vec![str!("none"), str!("block"), str!("auto")],
                components,
            );
            assert_match!(res);

            let res = match_group_all_any_order(
                &vec![
                    str!("none"),
                    str!("block"),
                    str!("block"),
                    str!("auto"),
                    str!("none"),
                ],
                components,
            );
            assert_not_match!(res);

            let res = match_group_all_any_order(
                &vec![str!("none"), str!("banana"), str!("car"), str!("block")],
                components,
            );
            assert_not_match!(res);
        }
    }

    #[test]
    fn test_match_group_at_least_one_any_order() {
        let tree = CssSyntax::new("auto none block").compile().unwrap();
        if let SyntaxComponent::Group { components, .. } = &tree.components[0] {
            let res = match_group_at_least_one_any_order(&vec![str!("auto")], components);
            assert_match!(res);

            let res =
                match_group_at_least_one_any_order(&vec![str!("auto"), str!("none")], components);
            assert_match!(res);

            let res = match_group_at_least_one_any_order(
                &vec![str!("auto"), str!("none"), str!("block")],
                components,
            );
            assert_match!(res);

            let res = match_group_at_least_one_any_order(
                &vec![str!("none"), str!("block"), str!("auto")],
                components,
            );
            assert_match!(res);

            let res = match_group_at_least_one_any_order(
                &vec![str!("none"), str!("block"), str!("auto")],
                components,
            );
            assert_match!(res);

            let res = match_group_at_least_one_any_order(
                &vec![
                    str!("none"),
                    str!("block"),
                    str!("none"),
                    str!("block"),
                    str!("auto"),
                    str!("none"),
                ],
                components,
            );
            assert_match!(res);
            assert_eq!(vec![str!("none"), str!("block")], res.matched_values);

            let res = match_group_at_least_one_any_order(
                &vec![str!("none"), str!("block"), str!("banana"), str!("auto")],
                components,
            );
            assert_match!(res);
            assert_eq!(vec![str!("none"), str!("block")], res.matched_values);
            assert_eq!(vec![str!("banana"), str!("auto")], res.remainder);

            let res = match_group_at_least_one_any_order(&vec![], components);
            assert_not_match!(res);
        }
    }

    #[test]
    fn test_multipliers_optional() {
        let tree = CssSyntax::new("foo bar baz").compile().unwrap();
        assert_false!(tree.clone().matches(vec![str!("foo")]));
        assert_false!(tree.clone().matches(vec![str!("foo")]));
        assert_true!(tree
            .clone()
            .matches(vec![str!("foo"), str!("bar"), str!("baz"),]));
        assert_false!(tree.clone().matches(vec![str!("foo"), str!("baz"),]));

        let tree = CssSyntax::new("foo bar?").compile().unwrap();
        assert_true!(tree.clone().matches(vec![str!("foo")]));
        assert_true!(tree.clone().matches(vec![str!("foo")]));
        assert_true!(tree.clone().matches(vec![str!("foo"), str!("bar"),]));
        assert_false!(tree
            .clone()
            .matches(vec![str!("foo"), str!("bar"), str!("bar"),]));
        assert_false!(tree.clone().matches(vec![str!("bar"), str!("foo"),]));

        let tree = CssSyntax::new("foo bar? baz").compile().unwrap();
        assert_false!(tree.clone().matches(vec![str!("foo")]));
        assert_true!(tree.clone().matches(vec![str!("foo"), str!("baz"),]));
        assert_true!(tree
            .clone()
            .matches(vec![str!("foo"), str!("bar"), str!("baz"),]));

        assert_false!(tree.clone().matches(vec![
            str!("foo"),
            str!("bar"),
            str!("bar"),
            str!("baz"),
        ]));

        assert_false!(tree.clone().matches(vec![
            str!("foo"),
            str!("bar"),
            str!("baz"),
            str!("baz"),
        ]));
    }

    #[test]
    fn test_multipliers_zero_or_more() {
        let tree = CssSyntax::new("foo bar* baz").compile().unwrap();
        assert_false!(tree.clone().matches(vec![str!("foo")]));
        assert_false!(tree.clone().matches(vec![str!("foo")]));
        assert_true!(tree
            .clone()
            .matches(vec![str!("foo"), str!("bar"), str!("baz"),]));
        assert_true!(tree.clone().matches(vec![str!("foo"), str!("baz"),]));
        assert_true!(tree.clone().matches(vec![
            str!("foo"),
            str!("bar"),
            str!("bar"),
            str!("bar"),
            str!("bar"),
            str!("baz"),
        ]));
        assert_false!(tree.clone().matches(vec![
            str!("foo"),
            str!("bar"),
            str!("bar"),
            str!("bar"),
            str!("baz"),
            str!("bar"),
        ]));

        let tree = CssSyntax::new("foo bar*").compile().unwrap();
        assert_true!(tree.clone().matches(vec![str!("foo")]));
        assert_true!(tree.clone().matches(vec![str!("foo")]));
        assert_true!(tree.clone().matches(vec![str!("foo"), str!("bar"),]));
        assert_true!(tree
            .clone()
            .matches(vec![str!("foo"), str!("bar"), str!("bar"),]));
        assert_false!(tree.clone().matches(vec![str!("bar"), str!("foo"),]));
    }

    #[test]
    fn test_multipliers_one_or_more() {
        let tree = CssSyntax::new("foo bar+ baz").compile().unwrap();
        assert_false!(tree.clone().matches(vec![str!("foo")]));
        assert_false!(tree.clone().matches(vec![str!("foo")]));
        assert_true!(tree
            .clone()
            .matches(vec![str!("foo"), str!("bar"), str!("baz"),]));
        assert_false!(tree.clone().matches(vec![str!("foo"), str!("baz"),]));
        assert_true!(tree.clone().matches(vec![
            str!("foo"),
            str!("bar"),
            str!("bar"),
            str!("bar"),
            str!("bar"),
            str!("baz"),
        ]));
        assert_false!(tree.clone().matches(vec![
            str!("foo"),
            str!("bar"),
            str!("bar"),
            str!("bar"),
            str!("baz"),
            str!("bar"),
        ]));

        let tree = CssSyntax::new("foo bar+").compile().unwrap();
        assert_false!(tree.clone().matches(vec![str!("foo")]));
        assert_false!(tree.clone().matches(vec![str!("foo")]));
        assert_false!(tree.clone().matches(vec![str!("bar")]));
        assert_true!(tree.clone().matches(vec![str!("foo"), str!("bar"),]));
        assert_true!(tree
            .clone()
            .matches(vec![str!("foo"), str!("bar"), str!("bar"),]));
        assert_false!(tree.clone().matches(vec![str!("bar"), str!("foo"),]));
    }

    #[test]
    fn test_multipliers_between() {
        let tree = CssSyntax::new("foo bar{1,3} baz").compile().unwrap();
        assert_false!(tree.clone().matches(vec![str!("foo")]));
        assert_false!(tree.clone().matches(vec![str!("foo")]));
        assert_true!(tree
            .clone()
            .matches(vec![str!("foo"), str!("bar"), str!("baz"),]));
        assert_false!(tree.clone().matches(vec![str!("foo"), str!("baz"),]));
        assert_true!(tree.clone().matches(vec![
            str!("foo"),
            str!("bar"),
            str!("bar"),
            str!("baz"),
        ]));
        assert_true!(tree.clone().matches(vec![
            str!("foo"),
            str!("bar"),
            str!("bar"),
            str!("bar"),
            str!("baz"),
        ]));
        assert_false!(tree.clone().matches(vec![
            str!("foo"),
            str!("bar"),
            str!("bar"),
            str!("bar"),
            str!("bar"),
            str!("baz"),
        ]));
        assert_false!(tree.clone().matches(vec![
            str!("foo"),
            str!("bar"),
            str!("bar"),
            str!("baz"),
            str!("bar"),
            str!("bar"),
        ]));

        let tree = CssSyntax::new("foo bar{0,3}").compile().unwrap();
        assert_true!(tree.clone().matches(vec![str!("foo")]));
        assert_true!(tree.clone().matches(vec![str!("foo")]));
        assert_true!(tree.clone().matches(vec![str!("foo"), str!("bar"),]));
        assert_true!(tree
            .clone()
            .matches(vec![str!("foo"), str!("bar"), str!("bar"),]));
        assert_false!(tree.clone().matches(vec![
            str!("foo"),
            str!("bar"),
            str!("bar"),
            str!("bar"),
            str!("bar"),
        ]));
        assert_false!(tree.clone().matches(vec![str!("bar"), str!("foo"),]));
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
                    "[ left | right ] <length>? | [ top | bottom ] <length> | [ top | bottom ]",
                )
                .compile()
                .unwrap(),
                inherited: false,
                initial_value: None,
                resolved: false,
            },
        );
        definitions.resolve();

        let prop = definitions.find_property("testprop").unwrap();

        // assert_true!(prop
        //     .clone()
        //     .matches(vec![str!("left"), CssValue::Unit(5.0, "px".into()),]));
        assert_true!(prop
            .clone()
            .matches(vec![str!("top"), CssValue::Unit(5.0, "px".into()),]));
        // assert_true!(prop
        //     .clone()
        //     .matches(vec![str!("bottom"), CssValue::Unit(5.0, "px".into()),]));
        // assert_true!(prop
        //     .clone()
        //     .matches(vec![str!("right"), CssValue::Unit(5.0, "px".into()),]));
        // assert_true!(prop.clone().matches(vec![str!("left")]));
        // assert_true!(prop.clone().matches(vec![str!("top")]));
        // assert_true!(prop.clone().matches(vec![str!("bottom")]));
        // assert_true!(prop.clone().matches(vec![str!("right")]));
        //
        // assert_false!(prop
        //     .clone()
        //     .matches(vec![CssValue::Unit(5.0, "px".into()), str!("right"),]));
        // assert_false!(prop.clone().matches(vec![
        //     CssValue::Unit(5.0, "px".into()),
        //     CssValue::Unit(10.0, "px".into()),
        //     str!("right"),
        // ]));
    }

    #[test]
    fn test_matcher_2() {
        let mut definitions = parse_definition_files();
        definitions.add_property(
            "testprop",
            PropertyDefinition {
                name: "testprop".to_string(),
                computed: vec![],
                syntax: CssSyntax::new("[ [ left | center | right | top | bottom | <length-percentage> ] | [ left | center | right | <length-percentage> ] [ top | center | bottom | <length-percentage> ] ]").compile().unwrap(),
                // syntax: CssSyntax::new("[ [ left | center ] [ top | center ] ]").compile().unwrap(),
                // syntax: CssSyntax::new("[ top | top top ]").compile().unwrap(),
                // syntax: CssSyntax::new("top | top top").compile().unwrap(),
                inherited: false,
                initial_value: None,
                resolved: false,
            },
        );
        definitions.resolve();

        let prop = definitions.find_property("testprop").unwrap();

        assert_true!(prop.clone().matches(vec![
            str!("left"),
        ]));
        assert_true!(prop.clone().matches(vec![
            str!("left"),
            str!("top"),
        ]));
        assert_true!(prop.clone().matches(vec![
            str!("center"),
            str!("top"),
        ]));
        assert_true!(prop.clone().matches(vec![str!("top"), str!("top"),]));
        assert_true!(prop.clone().matches(vec![
            str!("top"),
            str!("center"),
        ]));
        assert_true!(prop.clone().matches(vec![
            str!("center"),
            str!("center"),
        ]));
        assert_true!(prop.clone().matches(vec![
            CssValue::Percentage(10.0),
            CssValue::Percentage(20.0),
        ]));
        assert_true!(prop.clone().matches(vec![
            CssValue::Unit(10.0, "px".into()),
            CssValue::Percentage(20.0),
        ]));
        assert_true!(prop.clone().matches(vec![
            str!("left"),
            CssValue::Percentage(20.0),
        ]));

        assert_true!(prop.clone().matches(vec![
            CssValue::Unit(10.0, "px".into()),
            str!("center"),
        ]));

        assert_true!(prop.clone().matches(vec![
            str!("top"),
            CssValue::Percentage(10.0),
        ]));

        assert_true!(prop
            .clone()
            .matches(vec![str!("right")]));

        assert_false!(prop
            .clone()
            .matches(vec![str!("top")]));
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

        assert_true!(prop.clone().matches(vec![str!("foo"),]));

        assert_true!(prop.clone().matches(vec![str!("foo"), str!("foo"),]));

        // assert_true!(prop.clone().matches(vec![
        //     str!("foo"),
        //     str!("foo"),
        //     str!("foo"),
        // ])); // I don't think this should match?

        assert_true!(prop.clone().matches(vec![str!("foo"), str!("bar"),]));

        assert_false!(prop.clone().matches(vec![str!("bar"),]));
        assert_false!(prop.clone().matches(vec![str!("bar"), str!("foo"),]));
    }

    #[test]
    fn test_fulfillment() {
        assert_eq!(
            multiplier_fulfilled(
                &SyntaxComponent::Group {
                    components: vec![],
                    combinator: GroupCombinators::Juxtaposition,
                    multipliers: vec![SyntaxComponentMultiplier::Once],
                },
                0
            ),
            Fulfillment::NotYetFulfilled
        );

        assert_eq!(
            multiplier_fulfilled(
                &SyntaxComponent::Group {
                    components: vec![],
                    combinator: GroupCombinators::Juxtaposition,
                    multipliers: vec![SyntaxComponentMultiplier::Once],
                },
                1
            ),
            Fulfillment::Fulfilled
        );

        assert_eq!(
            multiplier_fulfilled(
                &SyntaxComponent::Group {
                    components: vec![],
                    combinator: GroupCombinators::Juxtaposition,
                    multipliers: vec![SyntaxComponentMultiplier::Once],
                },
                2
            ),
            Fulfillment::NotFulfilled
        );

        assert_eq!(
            multiplier_fulfilled(
                &SyntaxComponent::Group {
                    components: vec![],
                    combinator: GroupCombinators::Juxtaposition,
                    multipliers: vec![SyntaxComponentMultiplier::ZeroOrMore],
                },
                0
            ),
            Fulfillment::FulfilledButMoreAllowed
        );

        assert_eq!(
            multiplier_fulfilled(
                &SyntaxComponent::Group {
                    components: vec![],
                    combinator: GroupCombinators::Juxtaposition,
                    multipliers: vec![SyntaxComponentMultiplier::ZeroOrMore],
                },
                1
            ),
            Fulfillment::FulfilledButMoreAllowed
        );

        assert_eq!(
            multiplier_fulfilled(
                &SyntaxComponent::Group {
                    components: vec![],
                    combinator: GroupCombinators::Juxtaposition,
                    multipliers: vec![SyntaxComponentMultiplier::ZeroOrMore],
                },
                2
            ),
            Fulfillment::FulfilledButMoreAllowed
        );

        assert_eq!(
            multiplier_fulfilled(
                &SyntaxComponent::Group {
                    components: vec![],
                    combinator: GroupCombinators::Juxtaposition,
                    multipliers: vec![SyntaxComponentMultiplier::OneOrMore],
                },
                0
            ),
            Fulfillment::NotYetFulfilled
        );

        assert_eq!(
            multiplier_fulfilled(
                &SyntaxComponent::Group {
                    components: vec![],
                    combinator: GroupCombinators::Juxtaposition,
                    multipliers: vec![SyntaxComponentMultiplier::OneOrMore],
                },
                1
            ),
            Fulfillment::FulfilledButMoreAllowed
        );

        assert_eq!(
            multiplier_fulfilled(
                &SyntaxComponent::Group {
                    components: vec![],
                    combinator: GroupCombinators::Juxtaposition,
                    multipliers: vec![SyntaxComponentMultiplier::OneOrMore],
                },
                2
            ),
            Fulfillment::FulfilledButMoreAllowed
        );

        assert_eq!(
            multiplier_fulfilled(
                &SyntaxComponent::Group {
                    components: vec![],
                    combinator: GroupCombinators::Juxtaposition,
                    multipliers: vec![SyntaxComponentMultiplier::Optional],
                },
                0
            ),
            Fulfillment::FulfilledButMoreAllowed
        );
    }

    #[test]
    fn test_match_with_subgroups() {
        let tree = CssSyntax::new("[a b ] | [a c]").compile().unwrap();
        assert_true!(tree.matches(vec![str!("a"), str!("b"),]));
        assert_true!(tree.matches(vec![str!("a"), str!("c"),]));
        assert_false!(tree.matches(vec![str!("b"), str!("b"),]));
    }

    #[test]
    fn test_matcher_4() {
        let mut definitions = parse_definition_files();
        definitions.add_property(
            "testprop",
            PropertyDefinition {
                name: "testprop".to_string(),
                computed: vec![],
                syntax: CssSyntax::new(
                    // "[ left | right ] <length>? | [ top | bottom ] <length> | [ top | bottom ]"
                    "left <length>? | top <length> | top"
                ).compile().unwrap(),
                inherited: false,
                initial_value: None,
                resolved: false,
            },
        );
        definitions.resolve();

        let prop = definitions.find_property("testprop").unwrap();

        // assert_true!(prop.clone().matches(vec![
        //     str!("left"),
        //     CssValue::Unit(10.0, "px".into()),
        // ]));
        // assert_true!(prop.clone().matches(vec![
        //     str!("right"),
        //     CssValue::Unit(10.0, "px".into()),
        // ]));
        // assert_true!(prop.clone().matches(vec![
        //     str!("left"),
        // ]));
        // assert_true!(prop.clone().matches(vec![
        //     str!("right"),
        // ]));

        assert_true!(prop.clone().matches(vec![
            str!("top"),
            CssValue::Unit(10.0, "px".into()),
        ]));
        // assert_true!(prop.clone().matches(vec![
        //     str!("bottom"),
        //     CssValue::Unit(10.0, "px".into()),
        // ]));

        // assert_true!(prop.clone().matches(vec![
        //     str!("top"),
        // ]));
        // assert_true!(prop.clone().matches(vec![
        //     str!("bottom"),
        // ]));

    }

}
