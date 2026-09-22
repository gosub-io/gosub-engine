use crate::colors::{is_named_color, is_system_color, ColorSyntax};
use crate::functions::calc;
use crate::matcher::shorthands::{copy_resolver, ShorthandResolver};
use crate::matcher::syntax::{GroupCombinators, SyntaxComponent, SyntaxComponentMultiplier};
use crate::stylesheet::CssValue;
use crate::tokenizer::NumberKind;
use cow_utils::CowUtils;

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

/// Whether `unit` is a length, asked of the one table that knows.
///
/// This used to be a list of its own, and the two drifted: it spelled `Q` in capitals and was
/// compared exactly, so once units were folded to lowercase at parse time `1Q` stopped being a
/// length at all; and of the twenty-four viewport units it named three.
fn is_length_unit(unit: &str) -> bool {
    calc::unit_datatype(unit) == Some("length")
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
    pub fn matches(&self, input: &[CssValue]) -> bool {
        if self.components.is_empty() {
            return false;
        }

        // The CSS-wide keywords are valid as the sole value of every property, yet they
        // appear in no property grammar, so accept them here at the top level. A value that
        // contains a substitution function (var()/env()) is likewise deferred: its grammar
        // cannot be checked until substitution, so it is valid at parse time for any
        // property.
        if is_css_wide_keyword(input) || contains_substitution(input) {
            return true;
        }

        assert!(
            (self.components.len() == 1),
            "Syntax tree must have exactly one root component"
        );

        let res = match_component(input, &self.components[0], None);
        res.matched && res.remainder.is_empty()
    }

    /// The values `input` matched as, in the grammar's canonical form, or `None` when it does
    /// not match. This is what a specified value serializes as (CSSOM §6.7.2): keywords in the
    /// grammar's spelling, a bare zero with the unit it matched as, `||` operands in grammar
    /// order, box repetitions in their shortest form. A CSS-wide keyword, or a value still
    /// holding a `var()`, is returned as it came: there is no grammar to read it against yet.
    pub fn canonical(&self, input: &[CssValue]) -> Option<Vec<CssValue>> {
        if self.components.is_empty() {
            return None;
        }
        if is_css_wide_keyword(input) || contains_substitution(input) {
            return Some(input.to_vec());
        }
        assert!(
            (self.components.len() == 1),
            "Syntax tree must have exactly one root component"
        );
        let res = match_component(input, &self.components[0], None);
        (res.matched && res.remainder.is_empty()).then_some(res.matched_values)
    }

    pub fn matches_and_shorthands(&self, input: &[CssValue], resolver: ShorthandResolver) -> bool {
        if self.components.is_empty() {
            return false;
        }

        if is_css_wide_keyword(input) || contains_substitution(input) {
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

/// The vendor prefixes a keyword can carry, used to recognise a prefixed *math function*
/// (`-webkit-calc(...)`), which is the same function under another name.
///
/// Prefixed keyword *values* get no such treatment: a value is matched against the property's
/// grammar like any other, so one is accepted only where a grammar lists it. This engine
/// implements almost none of them, and a value a UA does not support makes the declaration
/// invalid (css-syntax-3 §9) - which is what lets the cascade fall back to the standard
/// declaration beside it, the reason pages write the prefixed form at all. The exception is the
/// handful of `display` keywords the Compatibility Standard requires, resolved to the value
/// they alias when the stylesheet is built.
const VENDOR_PREFIXES: [&str; 6] = ["-webkit-", "-moz-", "-ms-", "-o-", "-khtml-", "-apple-"];

/// Returns the remainder of `s` after a known vendor prefix, or None.
fn strip_vendor_prefix(s: &str) -> Option<&str> {
    VENDOR_PREFIXES.iter().find_map(|p| {
        (s.len() > p.len() && s.get(..p.len()).is_some_and(|head| head.eq_ignore_ascii_case(p))).then(|| &s[p.len()..])
    })
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
    let mut repetitions: Vec<Vec<CssValue>> = vec![];

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
            // Every repetition's values are kept. This used to return the last repetition's
            // result alone once the multiplier was satisfied, so `margin: 1px 2px 3px 4px`
            // reported `4px` as what it matched.
            repetitions.push(res.matched_values);

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
                        return MatchResult {
                            remainder,
                            matched: true,
                            matched_values: collapse_repetitions(component, repetitions),
                        };
                    }
                }
                Fulfillment::Fulfilled => {
                    // no more values are allowed.
                    return MatchResult {
                        remainder,
                        matched: true,
                        matched_values: collapse_repetitions(component, repetitions),
                    };
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
                    matched_values: collapse_repetitions(component, repetitions),
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
            // Not matched, so break the loop. A comma was consumed ahead of this item; it is
            // not part of what matched.
            if matches!(matched_values.last(), Some(CssValue::Comma)) {
                matched_values.pop();
            }
            break;
        }

        csv_cnt += 1;
        // The separator is part of the matched value: `20s, 10s` serializes with its comma.
        if csv_cnt > 1 {
            matched_values.push(CssValue::Comma);
        }
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
        if let Some(resolver) = shorthand_resolver.as_mut() {
            resolver.layer_separator();
        }

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
                // Keywords are ASCII case-insensitive and serialize in lowercase (CSSOM
                // §6.7.2); the grammar itself spells a few in mixed case (`currentColor`).
                return matched_as(input, CssValue::String(keyword.cow_to_ascii_lowercase().into_owned()));
            }
            _ => {}
        },
        SyntaxComponent::Definition { .. } => {
            return no_match(input);
        }
        SyntaxComponent::Builtin { datatype, range, .. } => {
            // A math function may be used wherever a numeric datatype is allowed (CSS
            // Values & Units §10), e.g. `width: calc(100% - 20px)`.
            //
            // Its arguments are type-checked rather than taken on trust. This used to accept any
            // math function outright, on the name alone, because a `calc()` body was opaque text
            // nobody evaluated - so `width: min(red, 50px)` was valid, and so was `min(1px 2px)`.
            // Now the expression is evaluated, its type is knowable, and an expression that does
            // not parse is known not to.
            if let CssValue::Function(name, args) = value {
                if is_math_function(name) && NUMERIC_DATATYPES.contains(&datatype.as_str()) {
                    return match calc::math_function_type(name, args, &calc::Units::none()) {
                        // A length mixed with a percentage is a legal `<length-percentage>`, and
                        // that expands to `[ <length> | <percentage> ]` before it reaches here -
                        // so by the time one arm is being matched there is no way to tell a
                        // length-only property from one that takes both. Accepting the mix
                        // over-accepts `border-width: min(1px, 1%)`; rejecting it would throw out
                        // `width: min(1px, 1%)`, which is valid and which real pages write.
                        calc::MathType::Resolved(kinds)
                            if kinds.iter().any(|kind| datatype_accepts(datatype, kind))
                                && kinds
                                    .iter()
                                    .all(|kind| datatype_accepts(datatype, kind) || *kind == "percentage") =>
                        {
                            first_match(input)
                        }
                        calc::MathType::Resolved(_) | calc::MathType::Invalid => no_match(input),
                        // A `var()` yet to be substituted, or a function this cannot evaluate.
                        // Not knowing is not the same as knowing it is wrong.
                        calc::MathType::Unknown => first_match(input),
                    };
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
                    CssValue::Zero if range.contains(0.0) => {
                        return matched_as(input, CssValue::Unit(0.0, "deg".to_string()))
                    }
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
                    CssValue::Zero if range.contains(0.0) => {
                        return matched_as(input, CssValue::Unit(0.0, "px".to_string()))
                    }
                    CssValue::Unit(n, u) if is_length_unit(u) && range.contains(*n) => return first_match(input),
                    _ => {}
                },
                "time" => match value {
                    CssValue::Zero if range.contains(0.0) => {
                        return matched_as(input, CssValue::Unit(0.0, "s".to_string()))
                    }
                    CssValue::Unit(n, u)
                        if (u.eq_ignore_ascii_case("s") || u.eq_ignore_ascii_case("ms")) && range.contains(*n) =>
                    {
                        return first_match(input)
                    }
                    _ => {}
                },
                // A flexible length is a `<number>` followed by the `fr` unit (grid track sizing).
                "flex" => match value {
                    CssValue::Zero if range.contains(0.0) => {
                        return matched_as(input, CssValue::Unit(0.0, "fr".to_string()))
                    }
                    CssValue::Unit(n, u) if u.eq_ignore_ascii_case("fr") && range.contains(*n) => {
                        return first_match(input)
                    }
                    _ => {}
                },
                "number" => match value {
                    CssValue::Zero if range.contains(0.0) => return first_match(input),
                    CssValue::Number(n, _) if range.contains(*n) => return first_match(input),
                    _ => {}
                },
                // An `<integer>` is spelled as digits with an optional sign: no decimal point
                // and no exponent. `1e1` and `10` are the same number and only one of them is an
                // integer, so the css-syntax type flag decides this and not the value - asking
                // whether the value happens to be whole says yes to `z-index: 1e1`.
                //
                // A math function is not held to the spelling: `calc(1e1)` and `calc(10.1)` are
                // both valid here, because css-values rounds a math function's result to the
                // nearest integer in an `<integer>` context. Those arrive as a function and are
                // answered by the `is_math_function` branch above, never here.
                "integer" => match value {
                    CssValue::Zero if range.contains(0.0) => return first_match(input),
                    CssValue::Number(n, NumberKind::Integer) if n.fract() == 0.0 && range.contains(*n) => {
                        return first_match(input)
                    }
                    _ => {}
                },
                // `<zero>` is a literal zero and nothing else. css-values allows it alongside
                // `<angle>` so that `rotate(0)` works - a bare `0` carries no unit, so it is not
                // an angle - and alongside `<length>` for the same reason.
                //
                // Without an arm here it fell to the permissive catch-all at the bottom, which
                // accepts any value at all. `<zero>` sits in the grammar of every transform
                // function, so `transform: rotate(banana)` was accepted through this.
                "zero" => match value {
                    CssValue::Zero => return first_match(input),
                    CssValue::Number(n, _) if *n == 0.0 => return first_match(input),
                    _ => {}
                },
                // A `<url>` is the `url()` function (or `src()`); the parser folds both the
                // `url(x)` token form and `url("x")` into a function. `<uri>`, `<url-token>` and
                // `<url-set>` are the older spellings of the same thing.
                //
                // These are in `BUILTIN_DATA_TYPES`, which makes `parse_syntax_file` skip the
                // grammar the definitions file carries for them - so without an arm here they
                // fell to the permissive catch-all, and `background-image: banana` was valid.
                "url" | "uri" | "url-token" | "url-set" => match value {
                    CssValue::Function(name, _)
                        if name.eq_ignore_ascii_case("url")
                            || name.eq_ignore_ascii_case("src")
                            || name.eq_ignore_ascii_case("image-set")
                            || name.eq_ignore_ascii_case("-webkit-image-set") =>
                    {
                        return first_match(input)
                    }
                    _ => {}
                },
                "resolution" => match value {
                    CssValue::Unit(n, u)
                        if range.contains(*n)
                            && (u.eq_ignore_ascii_case("dpi")
                                || u.eq_ignore_ascii_case("dpcm")
                                || u.eq_ignore_ascii_case("dppx")
                                || u.eq_ignore_ascii_case("x")) =>
                    {
                        return first_match(input)
                    }
                    _ => {}
                },
                "frequency" => match value {
                    CssValue::Zero if range.contains(0.0) => return first_match(input),
                    CssValue::Unit(n, u)
                        if range.contains(*n) && (u.eq_ignore_ascii_case("hz") || u.eq_ignore_ascii_case("khz")) =>
                    {
                        return first_match(input)
                    }
                    _ => {}
                },
                // The aural/speech types. Rare, but as cheap to answer as to leave open.
                "decibel" => match value {
                    CssValue::Zero if range.contains(0.0) => return first_match(input),
                    CssValue::Unit(n, u) if range.contains(*n) && u.eq_ignore_ascii_case("db") => {
                        return first_match(input)
                    }
                    _ => {}
                },
                "semitones" => match value {
                    CssValue::Zero if range.contains(0.0) => return first_match(input),
                    CssValue::Unit(n, u) if range.contains(*n) && u.eq_ignore_ascii_case("st") => {
                        return first_match(input)
                    }
                    _ => {}
                },
                // A `<dimension>` is any number with a unit - the token, not a particular type.
                "dimension" => match value {
                    CssValue::Zero => return first_match(input),
                    CssValue::Unit(n, _) if range.contains(*n) => return first_match(input),
                    _ => {}
                },
                "number-token" => match value {
                    CssValue::Zero if range.contains(0.0) => return first_match(input),
                    CssValue::Number(n, _) if range.contains(*n) => return first_match(input),
                    _ => {}
                },
                "hash-token" => match value {
                    CssValue::Color(_) => return first_match(input),
                    CssValue::String(v) if v.starts_with('#') => return first_match(input),
                    _ => {}
                },
                "age" => match value {
                    CssValue::String(v) if ["child", "young", "old"].iter().any(|kw| v.eq_ignore_ascii_case(kw)) => {
                        return first_match(input)
                    }
                    _ => {}
                },
                "gender" => match value {
                    CssValue::String(v)
                        if ["male", "female", "neutral"]
                            .iter()
                            .any(|kw| v.eq_ignore_ascii_case(kw)) =>
                    {
                        return first_match(input)
                    }
                    _ => {}
                },
                // `<declaration-value>` really is "any sequence of tokens" (css-syntax §9), so
                // the permissive answer is the correct one. Saying so explicitly separates it
                // from the datatypes that reach the catch-all only because nobody wrote an arm.
                "declaration-value" => return first_match(input),
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
                    // The brackets of a line-name list are structure, not identifiers, or the
                    // `<custom-ident>*` inside `[ ... ]` would swallow its own closing bracket.
                    CssValue::String(s) if s != "/" && s != "[" && s != "]" => return first_match(input),
                    _ => {}
                },
                // A quoted string and a bare identifier both arrive as `CssValue::String`, so
                // `<string>` cannot yet tell `"a"` from `a`; it can at least refuse the
                // structural tokens, or `grid-template: [a] 10px` matched its `<string>` piece
                // against the `[`.
                "string" => match value {
                    CssValue::String(s) if s == "/" || s == "[" || s == "]" => return no_match(input),
                    // An identifier is accepted: a quoted string and a bare identifier both
                    // arrive as `CssValue::String`, so `<string>` cannot tell them apart yet.
                    CssValue::String(_) => return first_match(input),
                    // A function this engine does not implement (`random-item()`) is accepted
                    // as it always was, so an unsupported feature reads as "cannot tell" rather
                    // than "invalid". A function known to be something else is not a string.
                    CssValue::Function(name, _)
                        if !is_math_function(name)
                            && !["repeat", "minmax", "fit-content", "url", "src"]
                                .iter()
                                .any(|f| name.eq_ignore_ascii_case(f)) =>
                    {
                        return first_match(input)
                    }
                    // A number, a length, a colour: not a string.
                    _ => return no_match(input),
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
            let min_bound = f64::MIN;
            let max_bound = f64::MAX;

            match value {
                // A bare `0` is a valid value for any unit-typed component (e.g. `<length>`):
                // it parses to the dedicated `Zero` variant, and `Number(0)` is the same case.
                CssValue::Zero => return first_match(input),
                CssValue::Number(n, _) if *n == 0.0 => return first_match(input),
                CssValue::Unit(n, u)
                    if unit.contains(u)
                        && *n >= from.map_or(min_bound, f64::from)
                        && *n <= to.map_or(max_bound, f64::from) =>
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
                        // The arguments are reported as the grammar orders them, not as they
                        // were written: a function's insides are canonicalized like anything
                        // else. `linear-gradient(in lab 30deg, ...)` serializes as
                        // `linear-gradient(30deg in lab, ...)`, because the `||` that holds the
                        // angle and the interpolation method lists the angle first.
                        return MatchResult {
                            remainder: input.get(1..).unwrap_or(&[]),
                            matched: true,
                            matched_values: vec![CssValue::Function(
                                c_name.clone(),
                                canonical_gradient(c_name, res.matched_values),
                            )],
                        };
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
        // A group never reaches here - `match_component` sends it to `match_component_group`
        // instead - but naming it keeps this match exhaustive, which is the point: a component
        // variant added later is now a compile error rather than a panic in front of a user.
        // It used to be a catch-all `panic!`, justified on the grounds that the test suite
        // covers every variant. That is a promise about test coverage; this is a promise the
        // compiler keeps.
        SyntaxComponent::Group { .. } => {}
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
    let mut components_matched = vec![];

    // Pass 1: find the winning alternative WITHOUT resolver side effects. `|` picks
    // exactly one alternative, but a value can syntactically match several (e.g.
    // `green` matches more than one arm of `<color>`); firing a shorthand-complete per
    // match would over-advance the {1,4} value->side counter and scramble box
    // shorthands like `border-color`.
    let mut c_idx = 0;
    while c_idx < components.len() {
        if input.is_empty() {
            break;
        }
        let res = match_component(input, &components[c_idx], None);
        if res.matched {
            components_matched.push((c_idx, res.matched_values, res.remainder));
        }
        c_idx += 1;
    }

    if components_matched.is_empty() {
        return no_match(input);
    }

    // The alternative that consumes the most wins. Between alternatives that consume the same,
    // the one that reports the value as written wins over one that rewrote it: a bare `0`
    // against `<length> | <number>` matches both, and it is a number, which serializes as `0`,
    // not the length's `0px`. A change of case alone is not a rewrite: `NONE` against
    // `none | <custom-ident>` is the keyword, spelled `none`, and not an ident called `NONE`.
    let mut winner = 0;
    let mut shortest_remainder_len = usize::MAX;
    let mut winner_as_written = false;
    for (idx, (_, values, remainder)) in components_matched.iter().enumerate() {
        let consumed = input.len() - remainder.len();
        let as_written = same_ignoring_case(values, &input[..consumed.min(input.len())]);
        let better = remainder.len() < shortest_remainder_len
            || (remainder.len() == shortest_remainder_len && as_written && !winner_as_written);
        if better {
            shortest_remainder_len = remainder.len();
            winner = idx;
            winner_as_written = as_written;
        }
    }
    let (winner_c_idx, winner_values, winner_remainder) = &components_matched[winner];

    // Pass 2: replay ONLY the winner against the resolver - completing it here, or
    // descending with the stepped resolver when the shorthand path continues deeper.
    if let Some(mut resolver) = copy_resolver(&mut shorthand_resolver) {
        match resolver.step(*winner_c_idx) {
            Ok(Some(sub)) => {
                let res = match_component(input, &components[*winner_c_idx], Some(sub));
                if res.matched {
                    return res;
                }
            }
            Ok(None) => {}
            Err(complete) => complete.complete(winner_values.clone()),
        }
    }

    MatchResult {
        remainder: winner_remainder,
        matched: true,
        matched_values: winner_values.clone(),
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

    // The order that matches without the resolver is the order to replay with it: the
    // completions have side effects, so they run for the winning assignment only.
    let order = best_any_order(components.len(), |order| {
        at_least_one_any_order_pass(raw_input, components, order)
    });

    let mut input = raw_input;
    // Collected per operand and flattened in grammar order at the end: the canonical
    // serialization of `a || b` lists a before b however the author ordered them.
    let mut per_component: Vec<Vec<CssValue>> = vec![Vec::new(); components.len()];
    let mut components_matched = vec![];

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
                per_component[c_idx] = res.matched_values.clone();
                components_matched.push(c_idx);

                input = res.remainder;

                // Found a match, so loop around for new matches
                pos = 0;

                if let Some(complete) = complete {
                    complete.complete(res.matched_values);
                }
            } else {
                // Element didn't match. That might be alright, and we continue with the next unmatched component
                pos += 1;
            }
        } else {
            // No resolver: `at_least_one_any_order_pass` handles that case and this function is
            // never called without one. Stopping leaves `components_matched` as it stands, and
            // the check below decides the match on that - an answer rather than an abort.
            break;
        }
    }

    if components_matched.is_empty() {
        return no_match(input);
    }

    MatchResult {
        remainder: input,
        matched: true,
        matched_values: per_component.into_iter().flatten().collect(),
    }
}

/// One greedy `||` pass trying operands in `order` priority (see all_any_order_pass).
fn at_least_one_any_order_pass<'a>(
    raw_input: &'a [CssValue],
    components: &[SyntaxComponent],
    order: &[usize],
) -> MatchResult<'a> {
    let mut input = raw_input;
    // Collected per operand and flattened in grammar order at the end: the canonical
    // serialization of `a || b` lists a before b however the author ordered them.
    let mut per_component: Vec<Vec<CssValue>> = vec![Vec::new(); components.len()];
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
            per_component[c_idx] = res.matched_values.clone();
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
        matched_values: per_component.into_iter().flatten().collect(),
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

    // The order that matches without the resolver is the order to replay with it: the
    // completions have side effects, so they run for the winning assignment only.
    let order = best_any_order(components.len(), |order| {
        all_any_order_pass(raw_input, components, order)
    });

    let mut input = raw_input;
    // Collected per operand and flattened in grammar order at the end: the canonical
    // serialization of `a || b` lists a before b however the author ordered them.
    let mut per_component: Vec<Vec<CssValue>> = vec![Vec::new(); components.len()];
    let mut components_matched = vec![];

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
                per_component[c_idx] = res.matched_values.clone();
                components_matched.push(c_idx);

                input = res.remainder;

                // Found a match, so loop around for new matches
                pos = 0;

                if let Some(complete) = complete {
                    complete.complete(res.matched_values);
                }
            } else {
                // Element didn't match. That might be alright, and we continue with the next unmatched component
                pos += 1;
            }
        } else {
            // No resolver: `all_any_order_pass` handles that case and this function is never
            // called without one. Stopping leaves `components_matched` as it stands, and the
            // check below decides the match on that - an answer rather than an abort.
            break;
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
        matched_values: per_component.into_iter().flatten().collect(),
    }
}

/// The operand order whose attempt does best, by the same measure as
/// [`best_any_order_attempt`]; the plain order when nothing matches.
fn best_any_order<'a>(component_count: usize, attempt: impl Fn(&[usize]) -> MatchResult<'a>) -> Vec<usize> {
    let mut best: Option<(Vec<usize>, usize)> = None;
    for offset in 0..component_count.max(1) {
        let order: Vec<usize> = (0..component_count)
            .map(|i| (i + offset) % component_count.max(1))
            .collect();
        let res = attempt(&order);
        if !res.matched {
            continue;
        }
        if res.remainder.is_empty() {
            return order;
        }
        if best.as_ref().is_none_or(|(_, len)| res.remainder.len() < *len) {
            best = Some((order, res.remainder.len()));
        }
    }
    best.map_or_else(|| (0..component_count).collect(), |(order, _)| order)
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
    // Collected per operand and flattened in grammar order at the end: the canonical
    // serialization of `a || b` lists a before b however the author ordered them.
    let mut per_component: Vec<Vec<CssValue>> = vec![Vec::new(); components.len()];
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
            per_component[c_idx] = res.matched_values.clone();
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
        matched_values: per_component.into_iter().flatten().collect(),
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
pub(crate) const NUMERIC_DATATYPES: [&str; 9] = [
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
/// Whether a component expecting `datatype` accepts an expression that came to `kind`.
///
/// The only place the two names differ is `<integer>`, which a math function reaches through
/// plain numbers - `z-index: calc(1 + 1)` is an integer-valued expression, and css-values-4 has
/// the result rounded to an integer rather than rejected for not already being one.
fn datatype_accepts(datatype: &str, kind: &str) -> bool {
    datatype == kind || (datatype == "integer" && kind == "number")
}

fn is_math_function(name: &str) -> bool {
    calc::is_math_function_name(strip_vendor_prefix(name).unwrap_or(name))
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

/// Drop a radial gradient's shape keyword when its size already says which shape it is
/// (css-images-3 §4.2): two size values can only describe an ellipse, one only a circle.
fn drop_implied_radial_shape(name: &str, args: Vec<CssValue>, head_end: usize) -> Vec<CssValue> {
    if !name.cow_to_ascii_lowercase().contains("radial") {
        return args;
    }
    let shape = match args.first() {
        Some(CssValue::String(word)) if word.eq_ignore_ascii_case("ellipse") => 2,
        Some(CssValue::String(word)) if word.eq_ignore_ascii_case("circle") => 1,
        _ => return args,
    };
    // The size is what sits between the shape and whatever comes after it: the `at` that starts
    // a position, or the `in` that starts an interpolation method.
    let sizes = args[1..head_end]
        .iter()
        .take_while(|value| {
            !matches!(value, CssValue::String(word) if word.eq_ignore_ascii_case("at") || word.eq_ignore_ascii_case("in"))
        })
        .count();
    if sizes != shape {
        return args;
    }
    args[1..].to_vec()
}

/// Tidy the interpolation method of a gradient, which has three rules of its own
/// (css-images-4 §3.1 and css-color-4 §12.4).
///
/// `xyz` is a synonym that serializes under its full name; `shorter hue` is what a polar space
/// does anyway; and a method that names the space the stops would have been interpolated in
/// regardless is left off entirely. That last one depends on the stops: the default is `oklab`,
/// except for a gradient whose every stop is a legacy sRGB colour, where it stays `srgb`.
fn canonical_gradient(name: &str, args: Vec<CssValue>) -> Vec<CssValue> {
    if !name.cow_to_ascii_lowercase().contains("gradient") {
        return args;
    }
    let head_end = |args: &[CssValue]| {
        args.iter()
            .position(|value| matches!(value, CssValue::Comma))
            .unwrap_or(args.len())
    };
    // Dropping the shape shortens the part before the stops, so the rest is measured again.
    let first_head_end = head_end(&args);
    let args = drop_implied_radial_shape(name, args, first_head_end);
    let head_end = head_end(&args);
    let Some(at) = args[..head_end]
        .iter()
        .position(|value| matches!(value, CssValue::String(word) if word.eq_ignore_ascii_case("in")))
    else {
        return args;
    };

    let mut method: Vec<String> = args[at + 1..head_end]
        .iter()
        .map(|value| value.to_string().cow_to_ascii_lowercase().into_owned())
        .collect();
    if method.first().is_some_and(|space| space == "xyz") {
        method[0] = "xyz-d65".to_string();
    }
    if method.len() >= 3 && method[method.len() - 2] == "shorter" && method[method.len() - 1] == "hue" {
        method.truncate(method.len() - 2);
    }

    // A stop that is not a legacy sRGB colour moves the default to oklab for the whole gradient.
    let all_legacy = args[head_end..].iter().all(|value| match value {
        CssValue::Color(color) => !matches!(
            color.syntax,
            ColorSyntax::Predefined(_) | ColorSyntax::Lab | ColorSyntax::Lch | ColorSyntax::Oklab | ColorSyntax::Oklch
        ),
        _ => true,
    });
    let default = if all_legacy { "srgb" } else { "oklab" };

    let mut out: Vec<CssValue> = args[..at].to_vec();
    if method.as_slice() != [default.to_string()] {
        out.push(CssValue::String("in".to_string()));
        out.extend(method.into_iter().map(CssValue::String));
    }
    // Dropping the method can empty the part before the stops, and then the comma that
    // separated them has nothing left to separate.
    let tail = if out.is_empty() && matches!(args.get(head_end), Some(CssValue::Comma)) {
        &args[head_end + 1..]
    } else {
        &args[head_end..]
    };
    out.extend_from_slice(tail);
    out
}

/// Whether two value lists are the same apart from the ASCII case of their keywords.
fn same_ignoring_case(a: &[CssValue], b: &[CssValue]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| match (x, y) {
            (CssValue::String(x), CssValue::String(y)) => x.eq_ignore_ascii_case(y),
            _ => x == y,
        })
}

/// The first element matched, reported as `value` rather than as written: the canonical form
/// the grammar leaf knows for it (a keyword in the grammar's spelling, a bare `0` matched as a
/// length as `0px`). What the CSSOM serializes is these matched values, not the author's text.
fn matched_as(input: &[CssValue], value: CssValue) -> MatchResult<'_> {
    MatchResult {
        remainder: input.get(1..).unwrap_or(&[]),
        matched: true,
        matched_values: vec![value],
    }
}

/// The matched values of a component that repeats `{1,2}` or `{1,4}`, in the shortest form
/// that means the same (CSSOM "serialize a CSS value" for the box shorthands): `1px 1px` is
/// `1px`, `1px 2px 1px 2px` is `1px 2px`, `1px 2px 3px 2px` is `1px 2px 3px`. Only when every
/// repetition matched exactly one value; anything else is returned as it was.
fn collapse_repetitions(component: &SyntaxComponent, reps: Vec<Vec<CssValue>>) -> Vec<CssValue> {
    let box_like = component.get_multipliers().iter().any(|m| {
        matches!(
            m,
            SyntaxComponentMultiplier::Between(1, 2) | SyntaxComponentMultiplier::Between(1, 4)
        )
    });
    if !box_like || reps.is_empty() || reps.iter().any(|r| r.len() != 1) {
        return reps.into_iter().flatten().collect();
    }
    let mut values: Vec<CssValue> = reps.into_iter().map(|mut r| r.remove(0)).collect();
    if values.len() == 4 && values[3] == values[1] {
        values.pop();
    }
    if values.len() == 3 && values[2] == values[0] {
        values.pop();
    }
    if values.len() == 2 && values[1] == values[0] {
        values.pop();
    }
    values
}

#[cfg(test)]
mod tests {

    use super::*;

    /// The canonical form of a specified value (CSSOM §6.7.2), as the grammar leaves report it.
    #[test]
    fn canonical_values_follow_the_grammar() {
        use crate::matcher::property_definitions::get_css_definitions;
        let canonical = |property: &str, css: &str| -> String {
            let value = crate::stylesheet::CssValue::parse_str(css)
                .ok()
                .map(|v| v.into_vec())
                .unwrap_or_default();
            let values = match crate::Css3::parse_str(
                &format!("x {{ {property}: {css} }}"),
                gosub_shared::config::ParserConfig::default(),
                gosub_interface::css3::CssOrigin::Author,
                "t",
            ) {
                Ok(sheet) => sheet.rules[0].declarations()[0].value.to_slice().to_vec(),
                Err(_) => value,
            };
            let def = get_css_definitions().find_property(property).expect("defined");
            def.canonical(&values)
                .map(CssValue::from_vec)
                .expect("valid")
                .to_string()
        };
        // Keywords in the grammar's spelling, lowercase.
        assert_eq!(canonical("animation-name", "NONE"), "none");
        assert_eq!(canonical("color", "currentColor"), "currentcolor");
        // A bare zero takes the unit it matched as - and stays `0` where a number would do.
        assert_eq!(canonical("column-gap", "0"), "0px");
        assert_eq!(canonical("border-image-width", "0"), "0");
        // `||` operands in grammar order, box repetitions in their shortest form.
        assert_eq!(
            canonical("text-decoration-line", "overline underline"),
            "underline overline"
        );
        assert_eq!(canonical("margin", "1px 1px"), "1px");
        assert_eq!(canonical("margin", "1px 2px 1px 2px"), "1px 2px");
        // Comma lists keep their commas.
        assert_eq!(canonical("animation-delay", "20s, 10s"), "20s, 10s");
    }
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
                    components: vec![].into(),
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
                    components: vec![].into(),
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
                    components: vec![].into(),
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
                    components: vec![].into(),
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
                    components: vec![].into(),
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
                    components: vec![].into(),
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
                    components: vec![].into(),
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
                    components: vec![].into(),
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
                    components: vec![].into(),
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
                    components: vec![].into(),
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
