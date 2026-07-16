use crate::colors::{is_named_color, is_system_color};
use crate::matcher::shorthands::{copy_resolver, ShorthandResolver};
use crate::matcher::syntax::{GroupCombinators, SyntaxComponent, SyntaxComponentMultiplier};
use crate::stylesheet::CssValue;

/// Structure to return from a matching function.
#[derive(Debug, Clone)]
pub struct MatchResult<'a> {
    /// The remainder of the values that are not matched.
    pub remainder: &'a [CssValue],
    /// True when this matched did some matching (todo: we might remove this and check for `matched_values.is_empty`)
    pub matched: bool,
    /// List of the matched values
    pub matched_values: Vec<CssValue>,
}

const LENGTH_UNITS: [&str; 31] = [
    "cap", "ch", "em", "ex", "ic", "lh", "rcap", "rch", "rem", "rex", "ric", "rlh", "vh", "vw", "vmax", "vmin", "vb",
    "vi", "cqw", "cqh", "cqi", "cqb", "cqmin", "cqmax", "px", "cm", "mm", "Q", "in", "pc", "pt",
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
    pub fn matches(&self, input: &[CssValue]) -> bool {
        if self.components.is_empty() {
            return false;
        }

        // The CSS-wide keywords are valid as the sole value of every property, yet they
        // appear in no property grammar, so accept them here at the top level. A value that
        // contains a substitution function (var()/env()) is likewise deferred: its grammar
        // cannot be checked until substitution, so it is valid at parse time for any
        // property. A lone vendor-prefixed keyword is accepted by policy (see
        // is_vendor_prefixed_keyword).
        if is_css_wide_keyword(input) || contains_substitution(input) || is_vendor_prefixed_keyword(input) {
            return true;
        }

        assert!(
            (self.components.len() == 1),
            "Syntax tree must have exactly one root component"
        );

        let res = match_component(input, &self.components[0], None);
        res.matched && res.remainder.is_empty()
    }

    pub fn matches_and_shorthands(&self, input: &[CssValue], resolver: ShorthandResolver) -> bool {
        if self.components.is_empty() {
            return false;
        }

        if is_css_wide_keyword(input) || contains_substitution(input) || is_vendor_prefixed_keyword(input) {
            return true;
        }

        assert!(
            (self.components.len() == 1),
            "Syntax tree must have exactly one root component"
        );

        let res = match_component(input, &self.components[0], Some(resolver));
        res.matched && res.remainder.is_empty()
    }
}

/// Returns true when `input` is exactly one CSS-wide keyword (`inherit`, `initial`,
/// `unset`, `revert`, `revert-layer`). These are valid for any property but must stand
/// alone - `margin: inherit inherit` is invalid - so a single value is required. The real
/// parser lowers them to `CssValue::String`, but the dedicated `Inherit`/`Initial`
/// variants are also accepted for callers that build values directly.
fn is_css_wide_keyword(input: &[CssValue]) -> bool {
    let [value] = input else {
        return false;
    };
    match value {
        CssValue::Inherit | CssValue::Initial => true,
        CssValue::String(s) => ["inherit", "initial", "unset", "revert", "revert-layer"]
            .iter()
            .any(|kw| s.eq_ignore_ascii_case(kw)),
        _ => false,
    }
}

/// POLICY: a lone vendor-prefixed keyword (`display: -webkit-box`, `position:
/// -webkit-sticky`, `cursor: -moz-grab`, …) is accepted for every property. Real-world
/// CSS ships these as cascade fallbacks before the standard value; each browser accepts
/// its own vendor's values, and rejecting them all would drop widely-deployed
/// declarations. Accepting keeps cascade behavior identical (a later standard value
/// still wins) without maintaining a per-value alias table. Only a single bare
/// identifier is covered - vendor keywords inside larger values stay strict, and
/// unprefixed legacy values (`display: box`) stay rejected, as in real browsers.
const VENDOR_PREFIXES: [&str; 6] = ["-webkit-", "-moz-", "-ms-", "-o-", "-khtml-", "-apple-"];

/// Returns the remainder of `s` after a known vendor prefix, or None.
fn strip_vendor_prefix(s: &str) -> Option<&str> {
    VENDOR_PREFIXES.iter().find_map(|p| {
        (s.len() > p.len() && s.get(..p.len()).is_some_and(|head| head.eq_ignore_ascii_case(p))).then(|| &s[p.len()..])
    })
}

fn is_vendor_prefixed_keyword(input: &[CssValue]) -> bool {
    let [CssValue::String(s)] = input else {
        return false;
    };
    strip_vendor_prefix(s).is_some()
}

/// Returns true when any value in the tree is a substitution function (`var()` or
/// `env()`), searching inside nested function arguments and lists. Such a value is
/// "guaranteed-invalid" to grammar-check until the substitution happens (CSS Variables
/// L1 §3), so a declaration containing one is valid at parse time for any property,
/// wherever the function appears (e.g. `1px solid var(--c)`, `rgb(var(--r), 0, 0)`).
fn contains_substitution(values: &[CssValue]) -> bool {
    values.iter().any(|value| match value {
        CssValue::Function(name, args) => {
            name.eq_ignore_ascii_case("var") || name.eq_ignore_ascii_case("env") || contains_substitution(args)
        }
        CssValue::List(items) => contains_substitution(items),
        _ => false,
    })
}

fn match_component_inner<'a>(
    raw_input: &'a [CssValue],
    component: &SyntaxComponent,
    mut shorthand_resolver: Option<ShorthandResolver>,
) -> MatchResult<'a> {
    let mut input = raw_input;
    let mut matched_values = vec![];

    // Loop through the input values and try to match them against the component. It's possible
    // that we need to loop multiple times in case we have a multiplier that allows this. ie: 'foo*' or 'foo{1,3}'
    let mut multiplier_count = 0;
    loop {
        if input.is_empty() {
            // We don't have anything in the input stream. We do need to check if this component
            // allows for optional values. If so, the component matches.
            let mff = multiplier_fulfilled(component, 0);
            if mff == Fulfillment::Fulfilled || mff == Fulfillment::FulfilledButMoreAllowed {
                return MatchResult {
                    remainder: &[],
                    matched: true,
                    matched_values: vec![],
                };
            }

            // Seems this component needs at least one value. We don't have any, so it's no match
            return no_match(raw_input);
        }

        // Check either single or group component
        let res = if component.is_group() {
            match_component_group(input, component, copy_resolver(&mut shorthand_resolver))
        } else {
            match_component_single(input, component)
        };

        if res.matched {
            // The element matched, so we keep track on how many times it did (in case of multipliers)
            multiplier_count += 1;

            let remainder = res.remainder;
            matched_values.append(&mut res.matched_values.clone());

            // Check if we fulfilled the multiplier for this component
            let mff = multiplier_fulfilled(component, multiplier_count);
            match mff {
                Fulfillment::NotYetFulfilled => {
                    // The multiplier is not yet fulfilled. Probably a range multiplier, so we need more
                    // values. Loop to the next value.
                    input = remainder;
                    continue;
                }
                Fulfillment::FulfilledButMoreAllowed => {
                    // More elements are allowed. Let's check if we have one
                    input = remainder;

                    // No more input to check, so we can just return this match
                    if input.is_empty() {
                        return res;
                    }
                }
                Fulfillment::Fulfilled => {
                    // no more values are allowed.
                    return res;
                }
                Fulfillment::NotFulfilled => {
                    // The multiplier is not fulfilled.
                    return no_match(raw_input);
                }
            }
        } else {
            let mff = multiplier_fulfilled(component, multiplier_count);
            return match mff {
                Fulfillment::NotYetFulfilled => {
                    // Don't know about this case
                    res
                }
                Fulfillment::Fulfilled => res,
                Fulfillment::FulfilledButMoreAllowed => MatchResult {
                    remainder: input,
                    matched: true,
                    matched_values,
                },
                Fulfillment::NotFulfilled => no_match(raw_input),
            };
        }
    }
}

/// Matches a component against the input values. After the match, there might be remaining
/// elements in the input. This is passed back in the `MatchResult` structure.
fn match_component<'a>(
    raw_input: &'a [CssValue],
    component: &SyntaxComponent,
    mut shorthand_resolver: Option<ShorthandResolver>,
) -> MatchResult<'a> {
    let mut input = raw_input;

    // Set some additional values when we are dealing with a comma separated lists (the # multiplier)
    let mut comma_separated = false;
    let mut csv_cnt = 0;
    let mut csv_min = 0;
    let mut csv_max = 0;
    let mut matched_values = vec![];
    for multiplier in component.get_multipliers() {
        if let SyntaxComponentMultiplier::CommaSeparatedRepeat(min, max) = multiplier {
            comma_separated = true;
            csv_min = min;
            csv_max = max;
        }
    }

    // CSV loop
    loop {
        let inner_result = match_component_inner(input, component, copy_resolver(&mut shorthand_resolver));
        if !comma_separated {
            // We don't need to check for comma separated values, so just return this result
            return inner_result;
        }

        if !inner_result.matched {
            // Not matched, so break the loop
            break;
        }

        csv_cnt += 1;
        matched_values.append(&mut inner_result.matched_values.clone());

        input = inner_result.remainder;

        // End of input.
        if input.is_empty() {
            break;
        }

        // If the next value is not a comma, the comma-separated list ends here; the
        // remaining input belongs to whatever component follows this one (e.g. the
        // `<box-shadow-spread>` after `<box-shadow-blur>#`). Stop consuming and let the
        // count check below decide whether enough items matched.
        if input.first() != Some(&CssValue::Comma) {
            break;
        }

        // Remove the comma, and continue matching
        input.clone_from(&&input[1..input.len()]);

        if input.is_empty() {
            // We have a comma at the end of the input. This is not allowed.
            return no_match(raw_input);
        }
    }

    // If we are in a comma separated list, we need to check if we have the correct amount of values
    if comma_separated && csv_cnt >= csv_min && csv_cnt <= csv_max {
        return MatchResult {
            remainder: input,
            matched: true,
            matched_values,
        };
    }

    no_match(raw_input)
}

/// Matches a component group
fn match_component_group<'a>(
    input: &'a [CssValue],
    component: &SyntaxComponent,
    shorthand_resolver: Option<ShorthandResolver>,
) -> MatchResult<'a> {
    match &component {
        SyntaxComponent::Group {
            components, combinator, ..
        } => match combinator {
            GroupCombinators::Juxtaposition => match_group_juxtaposition(input, components, shorthand_resolver),
            GroupCombinators::AllAnyOrder => match_group_all_any_order(input, components, shorthand_resolver),
            GroupCombinators::AtLeastOneAnyOrder => {
                match_group_at_least_one_any_order(input, components, shorthand_resolver)
            }
            GroupCombinators::ExactlyOne => match_group_exactly_one(input, components, shorthand_resolver),
        },
        _ => no_match(input),
    }
}

/// Matches a single component value
fn match_component_single<'a>(input: &'a [CssValue], component: &SyntaxComponent) -> MatchResult<'a> {
    // Get the first value from the input which we will use for matching
    let Some(value) = input.first() else {
        return no_match(input);
    };

    match &component {
        SyntaxComponent::GenericKeyword { keyword, .. } => match value {
            CssValue::None if keyword.eq_ignore_ascii_case("none") => {
                return first_match(input);
            }
            CssValue::String(v) if v.eq_ignore_ascii_case(keyword) => {
                return first_match(input);
            }
            _ => {}
        },
        SyntaxComponent::Definition { .. } => {
            return no_match(input);
        }
        SyntaxComponent::Builtin { datatype, range, .. } => {
            // A math function may be used wherever a numeric datatype is allowed (CSS
            // Values & Units §10), e.g. `width: calc(100% - 20px)`. The expression is
            // accepted opaquely - calc bodies are stored as raw text for the layout
            // engine to evaluate, so neither its type nor a `[min,max]` range can be
            // checked here.
            if let CssValue::Function(name, _) = value {
                if is_math_function(name) && NUMERIC_DATATYPES.contains(&datatype.as_str()) {
                    return first_match(input);
                }
            }
            match datatype.as_str() {
                // For the numeric datatypes, an optional `[min,max]` range written on the
                // reference (e.g. `<length [0,∞]>`) is enforced here: the magnitude must fall
                // within it. An empty range accepts every value, so unranged uses are
                // unaffected.
                "percentage" => {
                    if let CssValue::Percentage(n) = value {
                        if range.contains(*n) {
                            return first_match(input);
                        }
                    }
                }
                "angle" => match value {
                    CssValue::Zero if range.contains(0.0) => return first_match(input),
                    CssValue::Unit(n, u)
                        if range.contains(*n)
                            && (u.eq_ignore_ascii_case("deg")
                                || u.eq_ignore_ascii_case("grad")
                                || u.eq_ignore_ascii_case("rad")
                                || u.eq_ignore_ascii_case("turn")) =>
                    {
                        return first_match(input)
                    }
                    _ => {}
                },
                "length" => match value {
                    CssValue::Zero if range.contains(0.0) => return first_match(input),
                    CssValue::Unit(n, u) if LENGTH_UNITS.contains(&u.as_str()) && range.contains(*n) => {
                        return first_match(input)
                    }
                    _ => {}
                },
                "time" => match value {
                    CssValue::Zero if range.contains(0.0) => return first_match(input),
                    CssValue::Unit(n, u)
                        if (u.eq_ignore_ascii_case("s") || u.eq_ignore_ascii_case("ms")) && range.contains(*n) =>
                    {
                        return first_match(input)
                    }
                    _ => {}
                },
                // A flexible length is a `<number>` followed by the `fr` unit (grid track sizing).
                "flex" => match value {
                    CssValue::Zero if range.contains(0.0) => return first_match(input),
                    CssValue::Unit(n, u) if u.eq_ignore_ascii_case("fr") && range.contains(*n) => {
                        return first_match(input)
                    }
                    _ => {}
                },
                "number" => match value {
                    CssValue::Zero if range.contains(0.0) => return first_match(input),
                    CssValue::Number(n) if range.contains(*n) => return first_match(input),
                    _ => {}
                },
                "integer" => match value {
                    CssValue::Zero if range.contains(0.0) => return first_match(input),
                    CssValue::Number(n) if n.fract() == 0.0 && range.contains(*n) => return first_match(input),
                    _ => {}
                },
                "system-color" => {
                    if let CssValue::String(v) = value {
                        if is_system_color(v) {
                            return first_match(input);
                        }
                    }
                }
                "named-color" => {
                    if let CssValue::String(v) = value {
                        if is_named_color(v) {
                            return first_match(input);
                        }
                    }
                }
                "hex-color" => match value {
                    CssValue::Color(_) => return first_match(input),
                    CssValue::String(v) if v.starts_with('#') => return first_match(input),
                    _ => {}
                },
                // `<alpha()>` (css-color-hdr) is an alternative of `<color-function>`, so it
                // denotes a FUNCTION named `alpha`, not a bare numeric. It must not fall
                // through to the permissive catch-all below (any string would match <color>),
                // and it must not match bare numerics either - that made `color: 0` valid and
                // let a leading `0` offset in box-shadow claim the shadow-color slot. No data
                // source carries its argument grammar, so arguments are accepted opaquely.
                "alpha()" => match value {
                    CssValue::Function(name, _) if name.eq_ignore_ascii_case("alpha") => return first_match(input),
                    _ => {}
                },
                // Identifiers are ident-like tokens only: the parser lowers them to String.
                // Matching them via the permissive catch-all let `<custom-ident>` swallow
                // units and numbers, e.g. `transition: 0.2s ease left` had `0.2s` claimed as
                // the transition-property name. Slashes are separators, not idents.
                "custom-ident" | "ident" => match value {
                    CssValue::String(s) if s != "/" => return first_match(input),
                    _ => {}
                },
                "dashed-ident" => match value {
                    CssValue::String(s) if s.starts_with("--") => return first_match(input),
                    _ => {}
                },
                // Commas and slashes are structural separators (list items, function
                // arguments, `<grid-line> / <grid-line>`, font-size/line-height), never leaf
                // datatype values. Without this guard the permissive catch-all would let a
                // built-in such as `<time>` consume the separator and leave the following
                // part unmatched (e.g. `transition: opacity 0.3s, transform 0.5s`,
                // `grid-column: 1 / span 2`). Grammar-level separators still match through
                // the Literal arm.
                _ if matches!(value, CssValue::Comma) => {}
                _ if matches!(value, CssValue::String(s) if s == "/") => {}
                _ => {
                    return first_match(input);
                } // _ => panic!("Unknown built-in datatype: {:?}", datatype),
            }
        }
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
                // A bare `0` is a valid value for any unit-typed component (e.g. `<length>`):
                // it parses to the dedicated `Zero` variant, and `Number(0)` is the same case.
                CssValue::Zero => return first_match(input),
                CssValue::Number(n) if *n == 0.0 => return first_match(input),
                CssValue::Unit(n, u)
                    if unit.contains(u) && *n >= from.unwrap_or(f32min) && *n <= to.unwrap_or(f32max) =>
                {
                    return first_match(input);
                }
                _ => {}
            }
        }
        SyntaxComponent::Literal { literal, .. } => match value {
            CssValue::String(v) if v.eq(literal) => return first_match(input),
            CssValue::String(v) if v.eq_ignore_ascii_case(literal) => {
                log::warn!("Case insensitive literal matched");
                return first_match(input);
            }
            // A comma token is parsed to its own value variant, so match it against a
            // `,` literal (e.g. the argument separators in `cubic-bezier(a, b, c, d)`).
            CssValue::Comma if literal == "," => return first_match(input),
            _ => {}
        },
        SyntaxComponent::Function { name, arguments, .. } => {
            let CssValue::Function(c_name, c_args) = value else {
                return no_match(input);
            };

            if !name.eq_ignore_ascii_case(c_name) {
                return no_match(input);
            }

            match arguments {
                // No argument grammar was declared for this function, so match on the
                // function name alone (we have nothing to validate the arguments against).
                None => return first_match(input),
                Some(arg_syntax) => {
                    // Match the function's actual arguments against its argument grammar.
                    // An empty argument list is allowed only if the grammar is satisfiable
                    // by no input (i.e. every argument is optional).
                    let res = match_component(c_args, arg_syntax, None);
                    if res.matched && res.remainder.is_empty() {
                        return first_match(input);
                    }
                    return no_match(input);
                }
            }
        }
        SyntaxComponent::Value { value: css_value, .. } => {
            if value == css_value {
                return first_match(input);
            }
        }
        e => {
            #[allow(clippy::panic)]
            // PANIC-SAFE: components come from the compiled-in definitions, which the test suite matches exhaustively
            {
                panic!("Unknown syntax component: {e:?}");
            }
        }
    }

    no_match(input)
}

/// Returns element if exactly one element matches in the group
fn match_group_exactly_one<'a>(
    raw_input: &'a [CssValue],
    components: &[SyntaxComponent],
    mut shorthand_resolver: Option<ShorthandResolver>,
) -> MatchResult<'a> {
    let input = raw_input;
    let mut matched_values = vec![];
    let mut components_matched = vec![];

    let mut c_idx = 0;
    while c_idx < components.len() {
        if input.is_empty() {
            break;
        }

        if let Some(mut resolver) = copy_resolver(&mut shorthand_resolver) {
            let step = resolver.step(c_idx);

            let mut complete = None;
            let mut resolver = None;

            match step {
                Ok(Some(r)) => resolver = Some(r),
                Ok(None) => {}
                Err(c) => complete = Some(c),
            }

            let component = &components[c_idx];

            let res = match_component(input, component, resolver);
            if res.matched {
                matched_values.append(&mut res.matched_values.clone());

                // input = res.remainder.clone();

                components_matched.push((c_idx, res.matched_values.clone(), res.remainder));

                if let Some(complete) = complete {
                    complete.complete(res.matched_values);
                }
            } else {
                // No match. That's all right.
            }
        } else {
            let component = &components[c_idx];

            let res = match_component(input, component, None);
            if res.matched {
                matched_values.append(&mut res.matched_values.clone());

                // input = res.remainder.clone();

                components_matched.push((c_idx, res.matched_values, res.remainder));
            } else {
                // No match. That's all right.
            }
        }
        c_idx += 1;
    }

    if components_matched.is_empty() {
        return no_match(input);
    }

    if components_matched.len() > 1 {
        let mut shortest_remainder_idx = 0;
        let mut shortest_remainder_len = usize::MAX;

        for (idx, (_, _, remainder)) in components_matched.iter().enumerate() {
            if remainder.len() < shortest_remainder_len {
                shortest_remainder_len = remainder.len();
                shortest_remainder_idx = idx;
            }
        }

        return MatchResult {
            remainder: components_matched[shortest_remainder_idx].2,
            matched: true,
            matched_values: components_matched[shortest_remainder_idx].1.clone(),
        };
    }

    MatchResult {
        remainder: components_matched[0].2,
        matched: true,
        matched_values: components_matched[0].1.clone(),
    }
}

/// Returns element, when at least one of the elements in the group matches
fn match_group_at_least_one_any_order<'a>(
    raw_input: &'a [CssValue],
    components: &[SyntaxComponent],
    mut shorthand_resolver: Option<ShorthandResolver>,
) -> MatchResult<'a> {
    // Same rotation strategy as match_group_all_any_order: a single greedy pass can
    // hand a value to the wrong operand (`transition: ease all 300ms` - the
    // <custom-ident> transition-property grabs `ease` before the easing operand gets a
    // chance). The shorthand-resolver path keeps the single pass (side effects).
    if shorthand_resolver.is_none() {
        return best_any_order_attempt(raw_input, components.len(), |order| {
            at_least_one_any_order_pass(raw_input, components, order)
        });
    }

    let mut input = raw_input;
    let mut matched_values = vec![];
    let mut components_matched = vec![];

    let mut c_idx = 0;
    while c_idx < components.len() {
        if input.is_empty() {
            break;
        }

        if let Some(mut resolver) = copy_resolver(&mut shorthand_resolver) {
            let step = resolver.step(c_idx);

            let mut complete = None;
            let mut resolver = None;

            match step {
                Ok(Some(r)) => resolver = Some(r),
                Ok(None) => {}
                Err(c) => complete = Some(c),
            }

            let component = &components[c_idx];

            let res = match_component(input, component, resolver);
            if res.matched {
                matched_values.append(&mut res.matched_values.clone());
                components_matched.push(c_idx);

                input = res.remainder;

                // Found a match, so loop around for new matches
                c_idx = 0;
                while components_matched.contains(&c_idx) {
                    c_idx += 1;
                }

                if let Some(complete) = complete {
                    complete.complete(res.matched_values);
                }
            } else {
                // Element didn't match. That might be alright, and we continue with the next unmatched component
                c_idx += 1;
                while components_matched.contains(&c_idx) {
                    c_idx += 1;
                }
            }
        } else {
            unreachable!("resolver-less matching is handled by at_least_one_any_order_pass");
        }
    }

    if components_matched.is_empty() {
        return no_match(input);
    }

    MatchResult {
        remainder: input,
        matched: true,
        matched_values,
    }
}

/// One greedy `||` pass trying operands in `order` priority (see all_any_order_pass).
fn at_least_one_any_order_pass<'a>(
    raw_input: &'a [CssValue],
    components: &[SyntaxComponent],
    order: &[usize],
) -> MatchResult<'a> {
    let mut input = raw_input;
    let mut matched_values = vec![];
    let mut components_matched: Vec<usize> = vec![];

    let mut pos = 0;
    while pos < order.len() {
        if input.is_empty() {
            break;
        }
        let c_idx = order[pos];
        if components_matched.contains(&c_idx) {
            pos += 1;
            continue;
        }

        let res = match_component(input, &components[c_idx], None);
        if res.matched {
            matched_values.append(&mut res.matched_values.clone());
            components_matched.push(c_idx);
            input = res.remainder;
            pos = 0;
        } else {
            pos += 1;
        }
    }

    if components_matched.is_empty() {
        return no_match(input);
    }

    MatchResult {
        remainder: input,
        matched: true,
        matched_values,
    }
}

fn match_group_all_any_order<'a>(
    raw_input: &'a [CssValue],
    components: &[SyntaxComponent],
    mut shorthand_resolver: Option<ShorthandResolver>,
) -> MatchResult<'a> {
    // A single greedy pass can assign a value to the wrong operand: in
    // `[ center | [left|right] <lp>? ] && [ center | [top|bottom] <lp>? ]` matching
    // `center left`, the first operand grabs `center` and `left` has no home, even
    // though the assignment left/center works. There is no full backtracking here, but
    // trying every rotation of the operand priority order covers the practical
    // ambiguities. The shorthand-resolver path keeps the single greedy pass: its
    // completions have side effects that must not run once per attempt.
    if shorthand_resolver.is_none() {
        return best_any_order_attempt(raw_input, components.len(), |order| {
            all_any_order_pass(raw_input, components, order)
        });
    }

    let mut input = raw_input;
    let mut matched_values = vec![];
    let mut components_matched = vec![];

    let mut c_idx = 0;
    while c_idx < components.len() {
        if input.is_empty() {
            break;
        }

        if let Some(mut resolver) = copy_resolver(&mut shorthand_resolver) {
            let step = resolver.step(c_idx);

            let mut complete = None;
            let mut resolver = None;

            match step {
                Ok(Some(r)) => resolver = Some(r),
                Ok(None) => {}
                Err(c) => complete = Some(c),
            }
            let component = &components[c_idx];

            let res = match_component(input, component, resolver);
            // Only claim this slot when the component actually consumed input, or when it
            // is required. An *optional* component that matched emptily must not claim its
            // slot: the real value it should match may appear later, once other operands
            // consume the values in between (e.g. the trailing `<color>` in
            // `box-shadow: 2px 2px 4px red`). Absent optionals are accepted by the
            // end-of-function check instead.
            let consumed = res.matched && res.remainder.len() < input.len();
            let optional = matches!(
                multiplier_fulfilled(component, 0),
                Fulfillment::Fulfilled | Fulfillment::FulfilledButMoreAllowed
            );
            if res.matched && (consumed || !optional) {
                matched_values.append(&mut res.matched_values.clone());
                components_matched.push(c_idx);

                input = res.remainder;

                // Found a match, so loop around for new matches
                c_idx = 0;
                while components_matched.contains(&c_idx) {
                    c_idx += 1;
                }

                if let Some(complete) = complete {
                    complete.complete(res.matched_values);
                }
            } else {
                // Element didn't match. That might be alright, and we continue with the next unmatched component
                c_idx += 1;
                while components_matched.contains(&c_idx) {
                    c_idx += 1;
                }
            }
        } else {
            unreachable!("resolver-less matching is handled by all_any_order_pass");
        }
    }

    // Every component must be accounted for. A component that never matched is only
    // acceptable if it is optional (its multiplier is satisfied by zero occurrences,
    // e.g. `a?` or `a*`). This matters when the input runs out before a trailing
    // optional operand gets its turn, e.g. `<color>? && [ … ] && <position>?`.
    for (idx, component) in components.iter().enumerate() {
        if components_matched.contains(&idx) {
            continue;
        }
        match multiplier_fulfilled(component, 0) {
            Fulfillment::Fulfilled | Fulfillment::FulfilledButMoreAllowed => {}
            _ => return no_match(raw_input),
        }
    }

    MatchResult {
        remainder: input,
        matched: true,
        matched_values,
    }
}

/// Runs `attempt` once per rotation of the operand priority order and returns the best
/// result: the first attempt that consumes all input wins outright, otherwise the
/// matched attempt with the shortest remainder.
fn best_any_order_attempt<'a>(
    raw_input: &'a [CssValue],
    component_count: usize,
    attempt: impl Fn(&[usize]) -> MatchResult<'a>,
) -> MatchResult<'a> {
    let mut best: Option<MatchResult> = None;
    for offset in 0..component_count.max(1) {
        let order: Vec<usize> = (0..component_count)
            .map(|i| (i + offset) % component_count.max(1))
            .collect();
        let res = attempt(&order);
        if !res.matched {
            continue;
        }
        if res.remainder.is_empty() {
            return res;
        }
        if best.as_ref().is_none_or(|b| res.remainder.len() < b.remainder.len()) {
            best = Some(res);
        }
    }
    best.unwrap_or_else(|| no_match(raw_input))
}

/// One greedy `&&` pass trying operands in `order` priority: after every claim the scan
/// restarts at the front of `order`; a failed operand moves the scan to the next one.
fn all_any_order_pass<'a>(
    raw_input: &'a [CssValue],
    components: &[SyntaxComponent],
    order: &[usize],
) -> MatchResult<'a> {
    let mut input = raw_input;
    let mut matched_values = vec![];
    let mut components_matched: Vec<usize> = vec![];

    let mut pos = 0;
    while pos < order.len() {
        if input.is_empty() {
            break;
        }
        let c_idx = order[pos];
        if components_matched.contains(&c_idx) {
            pos += 1;
            continue;
        }

        let component = &components[c_idx];
        let res = match_component(input, component, None);
        // See match_group_all_any_order: only claim a slot on real consumption or for
        // required operands; absent optionals are handled by the final check.
        let consumed = res.matched && res.remainder.len() < input.len();
        let optional = matches!(
            multiplier_fulfilled(component, 0),
            Fulfillment::Fulfilled | Fulfillment::FulfilledButMoreAllowed
        );
        if res.matched && (consumed || !optional) {
            matched_values.append(&mut res.matched_values.clone());
            components_matched.push(c_idx);
            input = res.remainder;
            pos = 0;
        } else {
            pos += 1;
        }
    }

    // Every unmatched component must be omissible.
    for (idx, component) in components.iter().enumerate() {
        if components_matched.contains(&idx) {
            continue;
        }
        match multiplier_fulfilled(component, 0) {
            Fulfillment::Fulfilled | Fulfillment::FulfilledButMoreAllowed => {}
            _ => return no_match(raw_input),
        }
    }

    MatchResult {
        remainder: input,
        matched: true,
        matched_values,
    }
}

fn match_group_juxtaposition<'a>(
    raw_input: &'a [CssValue],
    components: &[SyntaxComponent],
    mut shorthand_resolver: Option<ShorthandResolver>,
) -> MatchResult<'a> {
    let mut input = raw_input;
    let mut matched_values = vec![];
    // Whether the previously matched component consumed input. Grammar commas next to an
    // OMITTED optional component are elided (CSS Values & Units §2.2), so a comma right
    // after an omitted component may be skipped. The group start counts as "consumed".
    let mut prev_consumed = true;

    let mut c_idx = 0;
    while c_idx < components.len() {
        let component = &components[c_idx];

        let res = if let Some(mut resolver) = copy_resolver(&mut shorthand_resolver) {
            let step = resolver.step(c_idx);

            let mut complete = None;
            let mut resolver = None;

            match step {
                Ok(Some(r)) => resolver = Some(r),
                Ok(None) => {}
                Err(c) => complete = Some(c),
            }

            let res = match_component(input, component, resolver);
            if res.matched {
                if let Some(complete) = complete {
                    complete.complete(res.matched_values.clone());
                }
            }
            res
        } else {
            match_component(input, component, None)
        };

        if res.matched {
            let consumed = res.remainder.len() < input.len();
            matched_values.append(&mut res.matched_values.clone());
            input = res.remainder;
            prev_consumed = consumed;
        } else {
            if is_comma_literal(component) {
                // Elide a comma whose preceding optional component was omitted
                // (`a? , b` matching just `b`), and keep matching after it.
                if !prev_consumed {
                    c_idx += 1;
                    continue;
                }
                // Elide a comma when everything after it is omitted (`a , b?` matching
                // just `a`): the group ends here, leaving the rest of the input untouched.
                // There must BE an omitted component: a comma that is the group's last
                // component separates against something outside the group (e.g. the
                // repeat group in `[ <bg-layer> , ]* <final-bg-layer>`) and stays mandatory.
                let rest = &components[c_idx + 1..];
                if !rest.is_empty() && rest.iter().all(is_omissible) {
                    return MatchResult {
                        remainder: input,
                        matched: true,
                        matched_values,
                    };
                }
            }
            break;
        }

        c_idx += 1;
    }

    if c_idx != components.len() {
        return no_match(input);
    }

    MatchResult {
        remainder: input,
        matched: true,
        matched_values,
    }
}

/// Returns true when the component is the literal comma separator.
/// Numeric datatypes a math function may substitute for (CSS Values & Units §10).
const NUMERIC_DATATYPES: [&str; 9] = [
    "length",
    "percentage",
    "number",
    "integer",
    "time",
    "angle",
    "flex",
    "frequency",
    "resolution",
];

/// Returns true when `name` is a CSS math function (CSS Values & Units §10). Vendor
/// prefixed forms (`-webkit-calc()`, `-moz-calc()`) predate the unprefixed ones and are
/// still common in shipped CSS, so a vendor prefix is stripped first.
fn is_math_function(name: &str) -> bool {
    let name = strip_vendor_prefix(name).unwrap_or(name);
    [
        "calc",
        "calc-size",
        "min",
        "max",
        "clamp",
        "round",
        "mod",
        "rem",
        "abs",
        "sign",
        "pow",
        "sqrt",
        "hypot",
        "log",
        "exp",
        "sin",
        "cos",
        "tan",
        "asin",
        "acos",
        "atan",
        "atan2",
    ]
    .iter()
    .any(|f| name.eq_ignore_ascii_case(f))
}

fn is_comma_literal(component: &SyntaxComponent) -> bool {
    matches!(component, SyntaxComponent::Literal { literal, .. } if literal == ",")
}

/// Returns true when the component may match zero occurrences (`?`, `*`, `{0,n}`).
fn is_omissible(component: &SyntaxComponent) -> bool {
    matches!(
        multiplier_fulfilled(component, 0),
        Fulfillment::Fulfilled | Fulfillment::FulfilledButMoreAllowed
    )
}

/// Fulfillment is a result returned by the `multiplier_fulfilled` function. This is used to determine
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
    // Filter out the multipliers that are not relevant for this check
    let binding = component.get_multipliers();
    let filtered_multipliers: Vec<_> = binding
        .iter()
        .filter(|m| {
            !matches!(
                m,
                SyntaxComponentMultiplier::AtLeastOneValue | SyntaxComponentMultiplier::CommaSeparatedRepeat(_, _)
            )
        })
        .collect();

    // Make sure that whenever we do not find a (primary) multiplier, we use the default "Once".
    match filtered_multipliers
        .first()
        .unwrap_or(&&SyntaxComponentMultiplier::Once)
    {
        SyntaxComponentMultiplier::Once => match cnt {
            0 => Fulfillment::NotYetFulfilled,
            1 => Fulfillment::Fulfilled,
            _ => Fulfillment::NotFulfilled,
        },
        SyntaxComponentMultiplier::ZeroOrMore => Fulfillment::FulfilledButMoreAllowed,
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
            _ if cnt < *from => Fulfillment::NotYetFulfilled,
            // At the maximum, the component is satisfied and must NOT consume more,
            // otherwise `<length>{2}` would greedily grab a following value (e.g. the
            // blur length after the two offset lengths in `box-shadow`).
            _ if cnt == *to => Fulfillment::Fulfilled,
            _ if cnt >= *from && cnt < *to => Fulfillment::FulfilledButMoreAllowed,
            _ => Fulfillment::NotFulfilled,
        },
        _ => Fulfillment::NotFulfilled,
    }
}

/// Helper function to return no matches
fn no_match(input: &[CssValue]) -> MatchResult<'_> {
    MatchResult {
        remainder: input,
        matched: false,
        matched_values: vec![],
    }
}

/// Helper function to return the first element from input in a match result, as we need this a lot
fn first_match(input: &[CssValue]) -> MatchResult<'_> {
    MatchResult {
        remainder: input.get(1..).unwrap_or(&[]),
        matched: true,
        matched_values: input.first().cloned().into_iter().collect(),
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::matcher::property_definitions::{get_css_definitions, PropertyDefinition};
    use crate::matcher::syntax::CssSyntax;

    macro_rules! str {
        ($s:expr) => {
            CssValue::String($s.to_string())
        };
    }

    macro_rules! assert_match {
        ($e:expr) => {
            println!("\n\n-------- ASSERT MATCH --------");
            let res = $e.clone();
            assert_eq!(true, res.matched);
            println!("------------------------------\n\n");
        };
    }

    macro_rules! assert_not_match {
        ($e:expr) => {
            println!("\n\n------- ASSERT NOT MATCH ------");
            let res = $e;
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
        assert_true!(tree.matches(&[str!("auto")]));
        assert_true!(tree.matches(&[CssValue::None]));
        assert_true!(tree.matches(&[str!("block")]));
        assert_false!(tree.matches(&[str!("inline")]));
        assert_false!(tree.matches(&[str!("")]));
        assert_false!(tree.matches(&[str!("foobar")]));
        assert_false!(tree.matches(&[str!("foo"), CssValue::None]));
        assert_false!(tree.matches(&[CssValue::None, str!("foo")]));
        assert_false!(tree.matches(&[str!("auto"), CssValue::None]));
        assert_false!(tree.matches(&[str!("auto"), CssValue::Comma, str!("none"),]));
        assert_false!(tree.matches(&[
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
        assert_false!(tree.matches(&[str!("auto")]));
        assert_false!(tree.matches(&[CssValue::None]));
        assert_false!(tree.matches(&[str!("block")]));
        assert_true!(tree.matches(&[str!("auto"), CssValue::None, str!("block"),]));
        assert_false!(tree.matches(&[str!("block"), CssValue::None, str!("block"),]));
        assert_false!(tree.matches(&[str!("auto"), CssValue::None, str!("auto"),]));
    }

    #[test]
    fn test_match_group3() {
        // all any order
        let tree = CssSyntax::new("auto && none && block").compile().unwrap();
        assert_false!(tree.matches(&[str!("auto")]));
        assert_false!(tree.matches(&[CssValue::None]));
        assert_false!(tree.matches(&[str!("block")]));
        assert_false!(tree.matches(&[str!("inline")]));
        assert_false!(tree.matches(&[str!("")]));
        assert_false!(tree.matches(&[str!("foobar")]));
        assert_false!(tree.matches(&[str!("foo"), CssValue::None]));
        assert_false!(tree.matches(&[CssValue::None, str!("foo")]));
        assert_false!(tree.matches(&[str!("auto"), CssValue::None]));
        assert_false!(tree.matches(&[str!("auto"), CssValue::Comma, str!("none")]));
        assert_false!(tree.matches(&[
            str!("auto"),
            CssValue::Comma,
            CssValue::None,
            CssValue::Comma,
            str!("block")
        ]));
        assert_true!(tree.matches(&[str!("block"), str!("auto"), CssValue::None]));
        assert_true!(tree.matches(&[str!("auto"), str!("block"), CssValue::None]));
        assert_true!(tree.matches(&[str!("block"), CssValue::None, str!("auto")]));
        assert_true!(tree.matches(&[CssValue::None, str!("auto"), str!("block")]));
        assert_false!(tree.matches(&[str!("auto"), str!("block")]));
        assert_false!(tree.matches(&[CssValue::None, str!("block")]));
        assert_false!(tree.matches(&[str!("block"), str!("block"), CssValue::None, CssValue::None]));
    }

    #[test]
    fn test_match_group4() {
        // At least one in any order
        let tree = CssSyntax::new("auto || none || block").compile().unwrap();
        assert_true!(tree.matches(&[str!("auto")]));
        assert_true!(tree.matches(&[CssValue::None]));
        assert_true!(tree.matches(&[str!("block")]));
        assert_true!(tree.matches(&[str!("auto"), CssValue::None]));
        assert_true!(tree.matches(&[str!("block"), str!("auto"), CssValue::None,]));

        assert_false!(tree.matches(&[str!("inline")]));
        assert_false!(tree.matches(&[str!("")]));
        assert_false!(tree.matches(&[str!("foo"), CssValue::None]));
        assert_false!(tree.matches(&[CssValue::None, str!("foo")]));
        assert_false!(tree.matches(&[CssValue::None, CssValue::None,]));
        assert_false!(tree.matches(&[str!("auto"), CssValue::Comma, str!("none"),]));
        assert_false!(tree.matches(&[
            str!("auto"),
            CssValue::Comma,
            CssValue::None,
            CssValue::Comma,
            str!("block"),
        ]));
        assert_false!(tree.matches(&[str!("block"), str!("block"), CssValue::None, CssValue::None,]));
    }

    #[test]
    fn test_match_group_juxtaposition() {
        let tree = CssSyntax::new("auto none block").compile().unwrap();
        if let SyntaxComponent::Group { components, .. } = &tree.components[0] {
            let input = [str!("auto")];
            let res = match_group_juxtaposition(&input, components, None);
            assert_not_match!(res);

            let input = [str!("auto"), str!("none")];
            let res = match_group_juxtaposition(&input, components, None);
            assert_not_match!(res);

            let input = [str!("auto"), str!("none"), str!("block")];
            let res = match_group_juxtaposition(&input, components, None);
            assert_match!(res);

            let input = [str!("none"), str!("block"), str!("auto")];
            let res = match_group_juxtaposition(&input, components, None);
            assert_not_match!(res);

            let input = [str!("none"), str!("block"), str!("block"), str!("auto"), str!("none")];
            let res = match_group_juxtaposition(&input, components, None);
            assert_not_match!(res);

            let input = [str!("none"), str!("banana"), str!("car"), str!("block")];

            let res = match_group_juxtaposition(&input, components, None);
            assert_not_match!(res);
        }
    }

    #[test]
    fn test_match_group_juxtaposition_with_groups() {
        // Test if groups are working icw juxtaposition
        let tree = CssSyntax::new("[top | bottom] [ up | down ] [ charm | strange] ")
            .compile()
            .unwrap();
        if let SyntaxComponent::Group { components, .. } = &tree.components[0] {
            let input = [str!("top"), str!("up"), str!("strange")];
            let res = match_group_juxtaposition(&input, components, None);
            assert_match!(res);

            let input = [str!("bottom"), str!("up"), str!("strange")];
            let res = match_group_juxtaposition(&input, components, None);
            assert_match!(res);

            let input = [str!("bottom"), str!("down"), str!("charm")];
            let res = match_group_juxtaposition(&input, components, None);
            assert_match!(res);
        }
    }

    #[test]
    fn test_match_group_all_any_order() {
        let tree = CssSyntax::new("auto none block").compile().unwrap();
        if let SyntaxComponent::Group { components, .. } = &tree.components[0] {
            let input = [str!("auto")];

            let res = match_group_all_any_order(&input, components, None);
            assert_not_match!(res);

            let input = [str!("auto"), str!("none")];
            let res = match_group_all_any_order(&input, components, None);
            assert_not_match!(res);

            let input = [str!("auto"), str!("none"), str!("block")];
            let res = match_group_all_any_order(&input, components, None);
            assert_match!(res);

            let input = [str!("none"), str!("block"), str!("auto")];

            let res = match_group_all_any_order(&input, components, None);
            assert_match!(res);

            let input = [str!("none"), str!("block"), str!("block"), str!("auto"), str!("none")];

            let res = match_group_all_any_order(&input, components, None);
            assert_not_match!(res);

            let input = [str!("none"), str!("banana"), str!("car"), str!("block")];

            let res = match_group_all_any_order(&input, components, None);
            assert_not_match!(res);
        }
    }

    #[test]
    fn test_match_group_at_least_one_any_order() {
        let tree = CssSyntax::new("auto none block").compile().unwrap();
        if let SyntaxComponent::Group { components, .. } = &tree.components[0] {
            let input = [str!("auto")];
            let res = match_group_at_least_one_any_order(&input, components, None);
            assert_match!(res);

            let input = [str!("auto"), str!("none")];
            let res = match_group_at_least_one_any_order(&input, components, None);
            assert_match!(res);

            let input = [str!("auto"), str!("none"), str!("block")];
            let res = match_group_at_least_one_any_order(&input, components, None);
            assert_match!(res);

            let input = [str!("none"), str!("block"), str!("auto")];
            let res = match_group_at_least_one_any_order(&input, components, None);
            assert_match!(res);

            let input = [str!("none"), str!("block"), str!("auto")];
            let res = match_group_at_least_one_any_order(&input, components, None);
            assert_match!(res);

            let input = [
                str!("none"),
                str!("block"),
                str!("none"),
                str!("block"),
                str!("auto"),
                str!("none"),
            ];

            let res = match_group_at_least_one_any_order(&input, components, None);
            assert_match!(res);
            assert_eq!(vec![str!("none"), str!("block")], res.matched_values);

            let input = [str!("none"), str!("block"), str!("banana"), str!("auto")];
            let res = match_group_at_least_one_any_order(&input, components, None);
            assert_match!(res);
            assert_eq!(vec![str!("none"), str!("block")], res.matched_values);
            assert_eq!(vec![str!("banana"), str!("auto")], res.remainder);

            let res = match_group_at_least_one_any_order(&[], components, None);
            assert_not_match!(res);
        }
    }

    #[test]
    fn test_multipliers_optional() {
        let tree = CssSyntax::new("foo bar baz").compile().unwrap();
        assert_false!(tree.clone().matches(&[str!("foo")]));
        assert_false!(tree.clone().matches(&[str!("foo")]));
        assert_true!(tree.clone().matches(&[str!("foo"), str!("bar"), str!("baz"),]));
        assert_false!(tree.clone().matches(&[str!("foo"), str!("baz"),]));

        let tree = CssSyntax::new("foo bar?").compile().unwrap();
        assert_true!(tree.clone().matches(&[str!("foo")]));
        assert_true!(tree.clone().matches(&[str!("foo"), str!("bar"),]));
        assert_false!(tree.clone().matches(&[str!("foo"), str!("bar"), str!("bar"),]));
        assert_false!(tree.clone().matches(&[str!("bar"), str!("foo"),]));

        let tree = CssSyntax::new("foo bar? baz").compile().unwrap();
        assert_false!(tree.clone().matches(&[str!("foo")]));
        assert_true!(tree.clone().matches(&[str!("foo"), str!("baz"),]));
        assert_true!(tree.clone().matches(&[str!("foo"), str!("bar"), str!("baz"),]));

        assert_false!(tree
            .clone()
            .matches(&[str!("foo"), str!("bar"), str!("bar"), str!("baz"),]));

        assert_false!(tree
            .clone()
            .matches(&[str!("foo"), str!("bar"), str!("baz"), str!("baz"),]));
    }

    #[test]
    fn test_multipliers_zero_or_more() {
        let tree = CssSyntax::new("foo bar* baz").compile().unwrap();
        assert_false!(tree.clone().matches(&[str!("foo")]));
        assert_false!(tree.clone().matches(&[str!("foo")]));
        assert_true!(tree.clone().matches(&[str!("foo"), str!("bar"), str!("baz"),]));
        assert_true!(tree.clone().matches(&[str!("foo"), str!("baz"),]));
        assert_true!(tree.clone().matches(&[
            str!("foo"),
            str!("bar"),
            str!("bar"),
            str!("bar"),
            str!("bar"),
            str!("baz"),
        ]));
        assert_false!(tree.clone().matches(&[
            str!("foo"),
            str!("bar"),
            str!("bar"),
            str!("bar"),
            str!("baz"),
            str!("bar"),
        ]));

        let tree = CssSyntax::new("foo bar*").compile().unwrap();
        assert_true!(tree.clone().matches(&[str!("foo")]));
        assert_true!(tree.clone().matches(&[str!("foo")]));
        assert_true!(tree.clone().matches(&[str!("foo"), str!("bar"),]));
        assert_true!(tree.clone().matches(&[str!("foo"), str!("bar"), str!("bar"),]));
        assert_false!(tree.clone().matches(&[str!("bar"), str!("foo"),]));
    }

    #[test]
    fn test_multipliers_one_or_more() {
        let tree = CssSyntax::new("foo bar+ baz").compile().unwrap();
        assert_false!(tree.clone().matches(&[str!("foo")]));
        assert_false!(tree.clone().matches(&[str!("foo")]));
        assert_true!(tree.clone().matches(&[str!("foo"), str!("bar"), str!("baz"),]));
        assert_false!(tree.clone().matches(&[str!("foo"), str!("baz"),]));
        assert_true!(tree.clone().matches(&[
            str!("foo"),
            str!("bar"),
            str!("bar"),
            str!("bar"),
            str!("bar"),
            str!("baz"),
        ]));
        assert_false!(tree.clone().matches(&[
            str!("foo"),
            str!("bar"),
            str!("bar"),
            str!("bar"),
            str!("baz"),
            str!("bar"),
        ]));

        let tree = CssSyntax::new("foo bar+").compile().unwrap();
        assert_false!(tree.clone().matches(&[str!("foo")]));
        assert_false!(tree.clone().matches(&[str!("bar")]));
        assert_true!(tree.clone().matches(&[str!("foo"), str!("bar"),]));
        assert_true!(tree.clone().matches(&[str!("foo"), str!("bar"), str!("bar"),]));
        assert_false!(tree.clone().matches(&[str!("bar"), str!("foo"),]));

        let tree = CssSyntax::new("foo+ bar+").compile().unwrap();
        assert_false!(tree.clone().matches(&[str!("foo")]));
        assert_false!(tree.clone().matches(&[str!("bar")]));
        assert_true!(tree.clone().matches(&[str!("foo"), str!("bar"),]));
        assert_true!(tree.clone().matches(&[str!("foo"), str!("bar"), str!("bar"),]));
        assert_true!(tree
            .clone()
            .matches(&[str!("foo"), str!("foo"), str!("bar"), str!("bar"),]));

        assert_false!(tree.clone().matches(&[str!("bar"), str!("foo"),]));
    }

    #[test]
    fn test_multipliers_between() {
        let tree = CssSyntax::new("foo bar{1,3} baz").compile().unwrap();
        assert_false!(tree.clone().matches(&[str!("foo")]));
        assert_false!(tree.clone().matches(&[str!("foo")]));
        assert_true!(tree.clone().matches(&[str!("foo"), str!("bar"), str!("baz"),]));
        assert_false!(tree.clone().matches(&[str!("foo"), str!("baz"),]));
        assert_true!(tree
            .clone()
            .matches(&[str!("foo"), str!("bar"), str!("bar"), str!("baz"),]));
        assert_true!(tree
            .clone()
            .matches(&[str!("foo"), str!("bar"), str!("bar"), str!("bar"), str!("baz"),]));
        assert_false!(tree.clone().matches(&[
            str!("foo"),
            str!("bar"),
            str!("bar"),
            str!("bar"),
            str!("bar"),
            str!("baz"),
        ]));
        assert_false!(tree.clone().matches(&[
            str!("foo"),
            str!("bar"),
            str!("bar"),
            str!("baz"),
            str!("bar"),
            str!("bar"),
        ]));

        let tree = CssSyntax::new("foo bar{0,3}").compile().unwrap();
        assert_true!(tree.clone().matches(&[str!("foo")]));
        assert_true!(tree.clone().matches(&[str!("foo")]));
        assert_true!(tree.clone().matches(&[str!("foo"), str!("bar"),]));
        assert_true!(tree.clone().matches(&[str!("foo"), str!("bar"), str!("bar"),]));
        assert_false!(tree
            .clone()
            .matches(&[str!("foo"), str!("bar"), str!("bar"), str!("bar"), str!("bar"),]));
        assert_false!(tree.clone().matches(&[str!("bar"), str!("foo"),]));
    }

    #[test]
    fn test_matcher() {
        let mut definitions = get_css_definitions().clone();
        definitions.add_property(
            "testprop",
            PropertyDefinition {
                name: "testprop".to_string(),
                computed: vec![],
                syntax: CssSyntax::new("[ left | right ] <length>? | [ top | bottom ] <length> | [ top | bottom ]")
                    .compile()
                    .unwrap(),
                inherited: false,
                initial_value: None,
                resolved: false,
                shorthands: None,
            },
        );
        definitions.resolve();

        let prop = definitions.find_property("testprop").unwrap();

        assert_true!(prop.clone().matches(&[str!("left"), CssValue::Unit(5.0, "px".into()),]));
        assert_true!(prop.clone().matches(&[str!("top"), CssValue::Unit(5.0, "px".into()),]));
        assert_true!(prop
            .clone()
            .matches(&[str!("bottom"), CssValue::Unit(5.0, "px".into()),]));
        assert_true!(prop
            .clone()
            .matches(&[str!("right"), CssValue::Unit(5.0, "px".into()),]));
        assert_true!(prop.clone().matches(&[str!("left")]));
        assert_true!(prop.clone().matches(&[str!("top")]));
        assert_true!(prop.clone().matches(&[str!("bottom")]));
        assert_true!(prop.clone().matches(&[str!("right")]));

        assert_false!(prop
            .clone()
            .matches(&[CssValue::Unit(5.0, "px".into()), str!("right"),]));
        assert_false!(prop.clone().matches(&[
            CssValue::Unit(5.0, "px".into()),
            CssValue::Unit(10.0, "px".into()),
            str!("right"),
        ]));
    }

    #[test]
    fn test_matcher_2() {
        let mut definitions = get_css_definitions().clone();
        definitions.add_property(
            "testprop",
            PropertyDefinition {
                name: "testprop".to_string(),
                computed: vec![],
                syntax: CssSyntax::new("[ [ left | center | right | top | bottom | <length-percentage> ] | [ left | center | right | <length-percentage> ] [ top | center | bottom | <length-percentage> ] ]").compile().unwrap(),
                inherited: false,
                initial_value: None,
                resolved: false,
                shorthands: None,
            },
        );
        definitions.resolve();

        let prop = definitions.find_property("testprop").unwrap();

        assert_true!(prop.clone().matches(&[str!("left"),]));
        assert_true!(prop.clone().matches(&[str!("left"), str!("top"),]));
        assert_true!(prop.clone().matches(&[str!("center"), str!("top"),]));
        assert_false!(prop.clone().matches(&[str!("top"), str!("top"),]));
        assert_false!(prop.clone().matches(&[str!("top"), str!("center"),]));
        assert_true!(prop.clone().matches(&[str!("center"), str!("top"),]));
        assert_true!(prop.clone().matches(&[str!("center"), str!("center"),]));
        assert_true!(prop
            .clone()
            .matches(&[CssValue::Percentage(10.0), CssValue::Percentage(20.0),]));
        assert_true!(prop
            .clone()
            .matches(&[CssValue::Unit(10.0, "px".into()), CssValue::Percentage(20.0),]));
        assert_true!(prop.clone().matches(&[str!("left"), CssValue::Percentage(20.0),]));

        assert_true!(prop
            .clone()
            .matches(&[CssValue::Unit(10.0, "px".into()), str!("center"),]));

        assert_true!(prop.clone().matches(&[CssValue::Percentage(10.0), str!("top"),]));

        assert_true!(prop.clone().matches(&[str!("right")]));

        assert_true!(prop.clone().matches(&[str!("top")]));
    }

    #[test]
    fn test_matcher_3() {
        let mut definitions = get_css_definitions().clone();
        definitions.add_property(
            "testprop",
            PropertyDefinition {
                name: "testprop".to_string(),
                computed: vec![],
                syntax: CssSyntax::new("foo | [ foo [ foo | bar ] ]").compile().unwrap(),
                inherited: false,
                initial_value: None,
                resolved: false,
                shorthands: None,
            },
        );
        definitions.resolve();

        let prop = definitions.find_property("testprop").unwrap();

        assert_true!(prop.clone().matches(&[str!("foo"),]));
        assert_true!(prop.clone().matches(&[str!("foo"), str!("foo"),]));
        assert_true!(prop.clone().matches(&[str!("foo"), str!("bar"),]));

        assert_false!(prop.clone().matches(&[str!("bar"),]));
        assert_false!(prop.clone().matches(&[str!("bar"), str!("foo"),]));
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
                0,
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
                1,
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
                2,
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
                0,
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
                1,
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
                2,
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
                0,
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
                1,
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
                2,
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
                0,
            ),
            Fulfillment::FulfilledButMoreAllowed
        );
    }

    #[test]
    fn test_match_with_subgroups() {
        let tree = CssSyntax::new("[a b ] | [a c]").compile().unwrap();
        assert_true!(tree.matches(&[str!("a"), str!("b"),]));
        assert_true!(tree.matches(&[str!("a"), str!("c"),]));
        assert_false!(tree.matches(&[str!("b"), str!("b"),]));
    }

    #[test]
    fn test_matcher_4() {
        let mut definitions = get_css_definitions().clone();
        definitions.add_property(
            "testprop",
            PropertyDefinition {
                name: "testprop".to_string(),
                computed: vec![],
                syntax: CssSyntax::new(
                    "[ left | right ] <length>? | [ top | bottom ] <length> | [ top | bottom ]", // "left <length>? | top <length> | top"
                )
                .compile()
                .unwrap(),
                inherited: false,
                initial_value: None,
                resolved: false,
                shorthands: None,
            },
        );
        definitions.resolve();

        let prop = definitions.find_property("testprop").unwrap();

        assert_true!(prop
            .clone()
            .matches(&[str!("left"), CssValue::Unit(10.0, "px".into()),]));
        assert_true!(prop
            .clone()
            .matches(&[str!("right"), CssValue::Unit(10.0, "px".into()),]));
        assert_true!(prop.clone().matches(&[str!("left"),]));
        assert_true!(prop.clone().matches(&[str!("right"),]));

        assert_true!(prop.clone().matches(&[str!("top"), CssValue::Unit(10.0, "px".into()),]));
        assert_true!(prop
            .clone()
            .matches(&[str!("bottom"), CssValue::Unit(10.0, "px".into()),]));

        assert_true!(prop.clone().matches(&[str!("top"),]));
        assert_true!(prop.clone().matches(&[str!("bottom"),]));
    }

    #[test]
    fn test_comma_separated() {
        let tree = CssSyntax::new("[foo | bar | baz]#").compile().unwrap();
        assert_true!(tree.matches(&[str!("foo")]));
        assert_true!(tree.matches(&[str!("foo"), CssValue::Comma, str!("foo")]));
        assert_true!(tree.matches(&[str!("foo"), CssValue::Comma, str!("foo"), CssValue::Comma, str!("foo")]));
        assert_true!(tree.matches(&[str!("foo"), CssValue::Comma, str!("bar")]));
        assert_true!(tree.matches(&[str!("foo"), CssValue::Comma, str!("baz")]));
        assert_true!(tree.matches(&[str!("foo"), CssValue::Comma, str!("bar"), CssValue::Comma, str!("baz")]));

        assert_false!(tree.matches(&[str!("foo"), CssValue::Comma]));
        assert_false!(tree.matches(&[str!("foo"), CssValue::Comma, str!("bar"), CssValue::Comma]));
        assert_false!(tree.matches(&[str!("foo"), CssValue::Comma, CssValue::Comma, str!("bar")]));
    }
}
