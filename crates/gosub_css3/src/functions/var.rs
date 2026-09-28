use crate::stylesheet::CssValue;
use std::collections::HashMap;

/// How deep a chain of custom properties referencing other custom properties is followed
/// (`--a: var(--b); --b: var(--c); …`). Cyclic references are already caught by the
/// visited-name stack; this only bounds pathological but acyclic chains.
pub(crate) const MAX_VAR_DEPTH: usize = 32;

/// Resolves a single `var(--name[, <fallback>])` against the custom properties in scope.
///
/// Returns the substitution tokens, which the caller splices into the surrounding value, or
/// `None` when the reference is invalid at computed-value time (the property is undefined and
/// no fallback was given, the fallback itself is unresolvable, or the reference is cyclic) - the
/// caller drops the declaration, which is what CSS requires.
///
/// No tokens at all is a substitution like any other: an empty fallback (`var(--nope,)`) and an
/// empty custom property (`--x:;`) both splice in nothing and leave the rest of the declaration.
pub fn resolve_var(values: &[CssValue], custom_props: &HashMap<String, CssValue>) -> Option<Vec<CssValue>> {
    resolve_var_inner(values, custom_props, &mut Vec::new()).ok()
}

/// Why a reference did not substitute.
#[derive(Debug)]
enum Unresolved {
    /// Undefined with no usable fallback, or too deep: the reference's own fallback may still
    /// apply, and an enclosing reference's fallback does.
    Invalid,
    /// The custom properties from the named one down to here reference each other. Every one of
    /// them is invalid at computed-value time (css-variables-1 §2.3), whatever fallbacks their
    /// own values hold, so this passes through the frames of the cycle without trying one - and
    /// becomes [`Unresolved::Invalid`] at the property it started from, whose reference is what
    /// the fallback outside the cycle is for.
    Cycle(String),
}

fn resolve_var_inner(
    values: &[CssValue],
    custom_props: &HashMap<String, CssValue>,
    seen: &mut Vec<String>,
) -> Result<Vec<CssValue>, Unresolved> {
    let name = values.first().map(ToString::to_string).ok_or(Unresolved::Invalid)?;

    // `var(--a, <fallback>)`: the arguments arrive as a flat token list with the separator kept
    // as its own `Comma`, so the fallback is everything after the *first* comma. It can be more
    // than one token (`var(--rule, 1px solid red)`) and may contain commas of its own
    // (`var(--font, "A", sans-serif)`), which stay part of the fallback.
    let fallback = values
        .iter()
        .position(|v| matches!(v, CssValue::Comma))
        .map(|comma| &values[comma + 1..]);

    // A `var()` inside the fallback (or inside the substituted value below) is resolved with the
    // same scope, so `var(--a, var(--b, red))` keeps working. An empty fallback is still one.
    let resolve_fallback = |seen: &mut Vec<String>| match fallback {
        Some(tokens) => substitute(tokens, custom_props, seen),
        None => Err(Unresolved::Invalid),
    };

    // A reference back into the chain being resolved closes a cycle. The reference itself is part
    // of it, so its own fallback does not apply either.
    if seen.iter().any(|n| n == &name) {
        return Err(Unresolved::Cycle(name));
    }
    if seen.len() >= MAX_VAR_DEPTH {
        return Err(Unresolved::Invalid);
    }

    let Some(value) = custom_props.get(&name) else {
        // Variable not defined - use the fallback if provided.
        return resolve_fallback(seen);
    };

    // `--x: initial` sets the property to its initial value, and css-variables-1 defines a custom
    // property's initial value as the guaranteed-invalid value. Substituting it would splice the
    // literal token `initial` into the declaration instead of taking the fallback.
    if matches!(value, CssValue::Initial) {
        return resolve_fallback(seen);
    }

    // Custom properties are stored with their `var()` references intact (they are substituted
    // lazily, at use), so the substituted value has to be resolved in turn.
    seen.push(name);
    let resolved = substitute(std::slice::from_ref(value), custom_props, seen);
    let name = seen.pop().unwrap_or_default();

    match resolved {
        Ok(tokens) => Ok(tokens),
        // This property is inside a cycle that started further out: it is invalid, and so is
        // this reference to it, fallback or not.
        Err(Unresolved::Cycle(start)) if start != name => Err(Unresolved::Cycle(start)),
        // The custom property computes to the guaranteed-invalid value - through a cycle that
        // started here, or otherwise - which css-variables-1 treats the same as undefined: use
        // the fallback.
        Err(_) => resolve_fallback(seen),
    }
}

/// Replaces every `var()` in `values` with its substitution, splicing multi-token results into
/// the surrounding list. Fails when any `var()` is unresolvable, so the invalidity propagates to
/// the declaration.
fn substitute(
    values: &[CssValue],
    custom_props: &HashMap<String, CssValue>,
    seen: &mut Vec<String>,
) -> Result<Vec<CssValue>, Unresolved> {
    let mut out = Vec::with_capacity(values.len());
    for value in values {
        match value {
            CssValue::Function(name, args) if name.eq_ignore_ascii_case("var") => {
                out.extend(resolve_var_inner(args, custom_props, seen)?);
            }
            CssValue::List(list) => out.extend(substitute(list, custom_props, seen)?),
            // Any other function may hold a `var()` among its arguments -
            // `linear-gradient(var(--from), white)` - so it is rebuilt around its substituted
            // arguments rather than cloned whole.
            CssValue::Function(name, args) => {
                out.push(CssValue::Function(name.clone(), substitute(args, custom_props, seen)?));
            }
            other => out.push(other.clone()),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn props(pairs: &[(&str, CssValue)]) -> HashMap<String, CssValue> {
        pairs.iter().map(|(k, v)| ((*k).to_string(), v.clone())).collect()
    }

    fn s(v: &str) -> CssValue {
        CssValue::String(v.to_string())
    }

    #[test]
    fn resolves_defined_variable() {
        let props = props(&[("--color", s("red"))]);

        let args = vec![s("--color")];
        assert_eq!(resolve_var(&args, &props), Some(vec![s("red")]));
    }

    #[test]
    fn falls_back_when_undefined() {
        let props = HashMap::new();
        let args = vec![s("--missing"), CssValue::Comma, s("blue")];
        assert_eq!(resolve_var(&args, &props), Some(vec![s("blue")]));
    }

    #[test]
    fn fallback_keeps_all_its_tokens() {
        let props = HashMap::new();
        let args = vec![
            s("--rule"),
            CssValue::Comma,
            CssValue::Unit(1.0, "px".to_string()),
            s("solid"),
            s("red"),
        ];
        assert_eq!(
            resolve_var(&args, &props),
            Some(vec![CssValue::Unit(1.0, "px".to_string()), s("solid"), s("red")])
        );
    }

    #[test]
    fn fallback_may_contain_commas() {
        let props = HashMap::new();
        let args = vec![
            s("--font"),
            CssValue::Comma,
            s("Helvetica"),
            CssValue::Comma,
            s("serif"),
        ];
        assert_eq!(
            resolve_var(&args, &props),
            Some(vec![s("Helvetica"), CssValue::Comma, s("serif")])
        );
    }

    #[test]
    fn nested_fallback_resolves() {
        let props = props(&[("--b", s("green"))]);
        let args = vec![
            s("--a"),
            CssValue::Comma,
            CssValue::Function("var".to_string(), vec![s("--b")]),
        ];
        assert_eq!(resolve_var(&args, &props), Some(vec![s("green")]));
    }

    #[test]
    fn resolves_variable_referencing_another_variable() {
        let props = props(&[
            ("--base", s("red")),
            ("--accent", CssValue::Function("var".to_string(), vec![s("--base")])),
        ]);

        let args = vec![s("--accent")];
        assert_eq!(resolve_var(&args, &props), Some(vec![s("red")]));
    }

    #[test]
    fn variable_referencing_undefined_variable_uses_outer_fallback() {
        let props = props(&[("--accent", CssValue::Function("var".to_string(), vec![s("--nope")]))]);

        let args = vec![s("--accent"), CssValue::Comma, s("blue")];
        assert_eq!(resolve_var(&args, &props), Some(vec![s("blue")]));
    }

    #[test]
    fn cyclic_references_resolve_to_nothing() {
        let props = props(&[
            ("--a", CssValue::Function("var".to_string(), vec![s("--b")])),
            ("--b", CssValue::Function("var".to_string(), vec![s("--a")])),
        ]);

        assert_eq!(resolve_var(&[s("--a")], &props), None);
    }

    #[test]
    fn substitutes_into_a_multi_token_value() {
        let props = props(&[(
            "--rule",
            CssValue::List(vec![
                CssValue::Unit(1.0, "px".to_string()),
                s("solid"),
                CssValue::Function("var".to_string(), vec![s("--c")]),
            ]),
        )]);
        let props = {
            let mut p = props;
            p.insert("--c".to_string(), s("red"));
            p
        };

        assert_eq!(
            resolve_var(&[s("--rule")], &props),
            Some(vec![CssValue::Unit(1.0, "px".to_string()), s("solid"), s("red")])
        );
    }

    #[test]
    fn returns_empty_when_undefined_and_no_fallback() {
        let props = HashMap::new();
        let args = vec![s("--missing")];
        assert_eq!(resolve_var(&args, &props), None);
    }

    #[test]
    fn initial_is_the_guaranteed_invalid_value() {
        // css-variables-1: a custom property's initial value *is* the guaranteed-invalid value,
        // so `--x: initial` is not a value the reference can use.
        let props = props(&[("--x", CssValue::Initial)]);

        assert_eq!(
            resolve_var(&[s("--x"), CssValue::Comma, s("blue")], &props),
            Some(vec![s("blue")])
        );
        assert_eq!(resolve_var(&[s("--x")], &props), None);
    }

    #[test]
    fn var_inside_another_function_is_substituted() {
        // A custom property holding `linear-gradient(var(--from), white)` used to come back with
        // the inner `var()` untouched, because only a bare `var()` and a list were descended into.
        let gradient = CssValue::Function(
            "linear-gradient".to_string(),
            vec![
                CssValue::Function("var".to_string(), vec![s("--from")]),
                CssValue::Comma,
                s("white"),
            ],
        );
        let props = props(&[("--from", s("red")), ("--g", gradient)]);

        assert_eq!(
            resolve_var(&[s("--g")], &props),
            Some(vec![CssValue::Function(
                "linear-gradient".to_string(),
                vec![s("red"), CssValue::Comma, s("white")]
            )])
        );
    }

    #[test]
    fn an_unresolvable_var_invalidates_the_function_around_it() {
        let props = props(&[(
            "--g",
            CssValue::Function(
                "linear-gradient".to_string(),
                vec![CssValue::Function("var".to_string(), vec![s("--nope")])],
            ),
        )]);

        assert_eq!(
            resolve_var(&[s("--g"), CssValue::Comma, s("red")], &props),
            Some(vec![s("red")])
        );
    }

    /// Custom properties that reference each other are all invalid at computed-value time,
    /// whatever fallbacks their own values hold (css-variables-1 §2.3); only a fallback outside
    /// the cycle applies. An empty fallback inside it must not make the cycle look resolved.
    #[test]
    fn a_fallback_inside_a_cycle_does_not_rescue_it() {
        let var = |args: Vec<CssValue>| CssValue::Function("var".to_string(), args);
        let px = |v: f64| CssValue::Unit(v, "px".to_string());
        let outer = [s("--a"), CssValue::Comma, px(20.0)];
        for inner in [
            var(vec![s("--b")]),
            var(vec![s("--b"), CssValue::Comma]),
            var(vec![s("--b"), CssValue::Comma, px(5.0)]),
        ] {
            let cycle = props(&[("--a", inner.clone()), ("--b", var(vec![s("--a")]))]);
            assert_eq!(resolve_var(&outer, &cycle), Some(vec![px(20.0)]), "--a: {inner:?}");
            assert_eq!(
                resolve_var(&outer[..1], &cycle),
                None,
                "--a: {inner:?} without a fallback"
            );
        }

        // A property that references itself through its own fallback is a cycle too.
        let own = props(&[("--a", var(vec![s("--nope"), CssValue::Comma, var(vec![s("--a")])]))]);
        assert_eq!(resolve_var(&outer, &own), Some(vec![px(20.0)]));

        // Outside the cycle, a property that only *uses* a cycle member is invalid, and its
        // reference takes its own fallback as usual.
        let user = props(&[
            ("--a", var(vec![s("--b")])),
            ("--b", var(vec![s("--a")])),
            ("--c", var(vec![s("--a")])),
        ]);
        assert_eq!(
            resolve_var(&[s("--c"), CssValue::Comma, px(7.0)], &user),
            Some(vec![px(7.0)])
        );
    }

    #[test]
    fn an_empty_fallback_substitutes_to_nothing() {
        let props = HashMap::new();
        let args = vec![s("--missing"), CssValue::Comma];
        assert_eq!(resolve_var(&args, &props), Some(vec![]));
    }

    #[test]
    fn an_empty_custom_property_substitutes_to_nothing_and_skips_the_fallback() {
        let props = props(&[("--empty", CssValue::List(vec![]))]);
        assert_eq!(resolve_var(&[s("--empty")], &props), Some(vec![]));
        assert_eq!(
            resolve_var(&[s("--empty"), CssValue::Comma, s("blue")], &props),
            Some(vec![])
        );
    }
}
