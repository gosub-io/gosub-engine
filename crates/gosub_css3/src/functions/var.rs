use crate::stylesheet::CssValue;
use std::collections::HashMap;

/// How deep a chain of custom properties referencing other custom properties is followed
/// (`--a: var(--b); --b: var(--c); …`). Cyclic references are already caught by the
/// visited-name stack; this only bounds pathological but acyclic chains.
const MAX_VAR_DEPTH: usize = 32;

/// Resolves a single `var(--name[, <fallback>])` against the custom properties in scope.
///
/// Returns the substitution tokens, which the caller splices into the surrounding value.
/// An empty vector means the reference is invalid at computed-value time (the property is
/// undefined and no fallback was given, or the fallback itself is unresolvable) - the caller
/// drops the declaration, which is what CSS requires.
pub fn resolve_var(values: &[CssValue], custom_props: &HashMap<String, CssValue>) -> Vec<CssValue> {
    resolve_var_inner(values, custom_props, &mut Vec::new())
}

fn resolve_var_inner(
    values: &[CssValue],
    custom_props: &HashMap<String, CssValue>,
    seen: &mut Vec<String>,
) -> Vec<CssValue> {
    let Some(name) = values.first().map(ToString::to_string) else {
        return vec![];
    };

    // `var(--a, <fallback>)`: the arguments arrive as a flat token list with the separator kept
    // as its own `Comma`, so the fallback is everything after the *first* comma. It can be more
    // than one token (`var(--rule, 1px solid red)`) and may contain commas of its own
    // (`var(--font, "A", sans-serif)`), which stay part of the fallback.
    let fallback = values
        .iter()
        .position(|v| matches!(v, CssValue::Comma))
        .map(|comma| &values[comma + 1..]);

    // A `var()` inside the fallback (or inside the substituted value below) is resolved with the
    // same scope, so `var(--a, var(--b, red))` keeps working.
    let resolve_fallback = |seen: &mut Vec<String>| match fallback {
        Some(tokens) if !tokens.is_empty() => substitute(tokens, custom_props, seen),
        _ => vec![],
    };

    // A cycle (`--a: var(--b); --b: var(--a)`) makes every property in it invalid at
    // computed-value time. Note this deliberately does not fall through to the fallback:
    // the reference itself is what is cyclic.
    if seen.iter().any(|n| n == &name) || seen.len() >= MAX_VAR_DEPTH {
        return vec![];
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
    seen.pop();

    if resolved.is_empty() {
        // The custom property is defined but computes to the guaranteed-invalid value, which
        // css-variables-1 treats the same as undefined: use the fallback.
        return resolve_fallback(seen);
    }
    resolved
}

/// Replaces every `var()` in `values` with its substitution, splicing multi-token results into
/// the surrounding list. Returns an empty vector when any `var()` is unresolvable, so the
/// invalidity propagates to the declaration.
fn substitute(values: &[CssValue], custom_props: &HashMap<String, CssValue>, seen: &mut Vec<String>) -> Vec<CssValue> {
    let mut out = Vec::with_capacity(values.len());
    for value in values {
        match value {
            CssValue::Function(name, args) if name.eq_ignore_ascii_case("var") => {
                let resolved = resolve_var_inner(args, custom_props, seen);
                if resolved.is_empty() {
                    return vec![];
                }
                out.extend(resolved);
            }
            CssValue::List(list) => {
                let resolved = substitute(list, custom_props, seen);
                if resolved.is_empty() && !list.is_empty() {
                    return vec![];
                }
                out.extend(resolved);
            }
            // Any other function may hold a `var()` among its arguments -
            // `linear-gradient(var(--from), white)` - so it is rebuilt around its substituted
            // arguments rather than cloned whole.
            CssValue::Function(name, args) => {
                let resolved = substitute(args, custom_props, seen);
                if resolved.is_empty() && !args.is_empty() {
                    return vec![];
                }
                out.push(CssValue::Function(name.clone(), resolved));
            }
            other => out.push(other.clone()),
        }
    }
    out
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
        assert_eq!(resolve_var(&args, &props), vec![s("red")]);
    }

    #[test]
    fn falls_back_when_undefined() {
        let props = HashMap::new();
        let args = vec![s("--missing"), CssValue::Comma, s("blue")];
        assert_eq!(resolve_var(&args, &props), vec![s("blue")]);
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
            vec![CssValue::Unit(1.0, "px".to_string()), s("solid"), s("red")]
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
            vec![s("Helvetica"), CssValue::Comma, s("serif")]
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
        assert_eq!(resolve_var(&args, &props), vec![s("green")]);
    }

    #[test]
    fn resolves_variable_referencing_another_variable() {
        let props = props(&[
            ("--base", s("red")),
            ("--accent", CssValue::Function("var".to_string(), vec![s("--base")])),
        ]);

        let args = vec![s("--accent")];
        assert_eq!(resolve_var(&args, &props), vec![s("red")]);
    }

    #[test]
    fn variable_referencing_undefined_variable_uses_outer_fallback() {
        let props = props(&[("--accent", CssValue::Function("var".to_string(), vec![s("--nope")]))]);

        let args = vec![s("--accent"), CssValue::Comma, s("blue")];
        assert_eq!(resolve_var(&args, &props), vec![s("blue")]);
    }

    #[test]
    fn cyclic_references_resolve_to_nothing() {
        let props = props(&[
            ("--a", CssValue::Function("var".to_string(), vec![s("--b")])),
            ("--b", CssValue::Function("var".to_string(), vec![s("--a")])),
        ]);

        assert_eq!(resolve_var(&[s("--a")], &props), vec![]);
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
            vec![CssValue::Unit(1.0, "px".to_string()), s("solid"), s("red")]
        );
    }

    #[test]
    fn returns_empty_when_undefined_and_no_fallback() {
        let props = HashMap::new();
        let args = vec![s("--missing")];
        assert_eq!(resolve_var(&args, &props), vec![]);
    }

    #[test]
    fn initial_is_the_guaranteed_invalid_value() {
        // css-variables-1: a custom property's initial value *is* the guaranteed-invalid value,
        // so `--x: initial` is not a value the reference can use.
        let props = props(&[("--x", CssValue::Initial)]);

        assert_eq!(
            resolve_var(&[s("--x"), CssValue::Comma, s("blue")], &props),
            vec![s("blue")]
        );
        assert_eq!(resolve_var(&[s("--x")], &props), vec![]);
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
            vec![CssValue::Function(
                "linear-gradient".to_string(),
                vec![s("red"), CssValue::Comma, s("white")]
            )]
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
            vec![s("red")]
        );
    }

    #[test]
    fn empty_fallback_is_not_a_value() {
        let props = HashMap::new();
        let args = vec![s("--missing"), CssValue::Comma];
        assert_eq!(resolve_var(&args, &props), vec![]);
    }
}
