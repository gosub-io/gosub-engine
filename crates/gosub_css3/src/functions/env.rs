use crate::stylesheet::CssValue;

/// Resolves a single `env(<name> <integer>*, <fallback>?)` (css-env-1 §3).
///
/// Returns the substitution tokens, which the caller splices into the surrounding value, the
/// same way a `var()` is spliced. An empty vector means the reference is invalid at
/// computed-value time: the name is not an environment variable this engine knows and no
/// fallback was given.
///
/// A name followed by indices (`env(viewport-segment-width 0 0)`) names one entry of a
/// multi-dimensional variable. The engine has none of those, so an indexed reference always
/// takes its fallback.
pub fn resolve_env(args: &[CssValue]) -> Vec<CssValue> {
    // As with `var()`, the separator arrives as its own token and the fallback is everything
    // after the first one - which may hold commas of its own.
    let comma = args.iter().position(|v| matches!(v, CssValue::Comma));
    let (reference, fallback) = match comma {
        Some(comma) => (&args[..comma], &args[comma + 1..]),
        None => (args, &[][..]),
    };

    let known = match reference {
        [CssValue::String(name)] => environment_variable(name),
        _ => None,
    };
    known.unwrap_or_else(|| fallback.to_vec())
}

/// The value of a one-dimensional environment variable, or `None` for a name this engine does
/// not define.
///
/// A desktop window has no display cutout and no on-screen keyboard, so the insets for them are
/// zero - which is what desktop browsers report too, and what lets a page write
/// `padding-top: max(1rem, env(safe-area-inset-top))`. The title-bar area only exists in a
/// window-controls-overlay window; outside one it is undefined and the fallback applies. Names
/// are case-sensitive: they are custom identifiers, not keywords.
fn environment_variable(name: &str) -> Option<Vec<CssValue>> {
    const ZERO_INSETS: [&str; 14] = [
        "safe-area-inset-top",
        "safe-area-inset-right",
        "safe-area-inset-bottom",
        "safe-area-inset-left",
        "safe-area-max-inset-top",
        "safe-area-max-inset-right",
        "safe-area-max-inset-bottom",
        "safe-area-max-inset-left",
        "keyboard-inset-top",
        "keyboard-inset-right",
        "keyboard-inset-bottom",
        "keyboard-inset-left",
        "keyboard-inset-width",
        "keyboard-inset-height",
    ];
    ZERO_INSETS
        .contains(&name)
        .then(|| vec![CssValue::Unit(0.0, "px".to_string())])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(text: &str) -> CssValue {
        CssValue::String(text.to_string())
    }

    fn px(value: f64) -> CssValue {
        CssValue::Unit(value, "px".to_string())
    }

    #[test]
    fn a_known_inset_is_zero_on_a_desktop_window() {
        assert_eq!(resolve_env(&[s("safe-area-inset-top")]), vec![px(0.0)]);
        // The fallback is only for a name that is not known.
        assert_eq!(
            resolve_env(&[s("keyboard-inset-height"), CssValue::Comma, px(20.0)]),
            vec![px(0.0)]
        );
    }

    #[test]
    fn an_unknown_name_takes_its_fallback_or_nothing() {
        assert_eq!(
            resolve_env(&[s("titlebar-area-height"), CssValue::Comma, px(33.0)]),
            vec![px(33.0)]
        );
        // A fallback may hold several tokens and commas of its own.
        assert_eq!(
            resolve_env(&[s("nope"), CssValue::Comma, px(1.0), CssValue::Comma, px(2.0)]),
            vec![px(1.0), CssValue::Comma, px(2.0)]
        );
        assert!(resolve_env(&[s("nope")]).is_empty());
        // Names are case-sensitive.
        assert!(resolve_env(&[s("SAFE-AREA-INSET-TOP")]).is_empty());
    }

    #[test]
    fn an_indexed_reference_takes_its_fallback() {
        let indexed = [
            s("viewport-segment-width"),
            CssValue::Number(0.0, crate::tokenizer::NumberKind::Integer),
            CssValue::Number(0.0, crate::tokenizer::NumberKind::Integer),
            CssValue::Comma,
            px(100.0),
        ];
        assert_eq!(resolve_env(&indexed), vec![px(100.0)]);
    }
}
