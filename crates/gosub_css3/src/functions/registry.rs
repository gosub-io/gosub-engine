//! Every CSS function the engine gives a meaning to, and what kind of function it is.
//!
//! The functions used to be known by name in several places at once: the grammar matcher's
//! bypass for the colour functions, the colour fold, the list of colour notations, the math
//! function list, and three lists of substitution functions that disagreed (`env()` passed
//! validation and was then never substituted). Adding a function meant finding all of them.
//! Here there is one table, and each of those places asks it.
//!
//! What the table does not hold is the functions' behaviour. The substitution functions each
//! need something different to resolve - `var()` the custom properties, `attr()` the element,
//! `env()` nothing, `light-dark()` the colour scheme - so their dispatch stays with the system
//! that has those to hand, and a test there checks it handles every one listed here.

use crate::colors::CssColor;
use crate::stylesheet::CssValue;

/// What a function is, as far as the places that dispatch on it care.
#[derive(Clone, Copy, Debug)]
pub(crate) enum FunctionKind {
    /// A colour notation whose arguments are the colour's components: `rgb()`, `oklch()`,
    /// `color()`. Each also takes the relative syntax (`rgb(from red r g b)`).
    ColorNotation,
    /// `alpha()`, which is a colour only in the relative syntax (`alpha(from red / 0.5)`).
    RelativeColor,
    /// A colour computed from other colours, checked and folded by its own module.
    ColorOperation(ColorOperation),
    /// A math function (css-values-4 §10), which may stand wherever a number, length, angle and
    /// so on may.
    Math,
    /// A function that is replaced by other tokens before the declaration is known. An
    /// arbitrary-substitution function (css-values-5 §7: `var()`, `env()`, `attr()`) can stand for
    /// any tokens at all, so a declaration holding one cannot be checked against its grammar until
    /// it has been substituted. `light-dark()` is substituted too, but it has a grammar of its own
    /// and a value holding one is validated like any other.
    Substitution { arbitrary: bool },
}

/// The two halves of a colour operation.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ColorOperation {
    /// Its canonical specified value (css-color-5 normalizes a `color-mix()` percentage there),
    /// or `None` when the arguments are not valid.
    pub canonical: fn(&[CssValue]) -> Option<CssValue>,
    /// The colour it computes to, or `None` when it cannot be worked out yet.
    pub resolve: fn(&[CssValue]) -> Option<CssColor>,
}

/// Every function the engine gives a meaning to. Names are ASCII case-insensitive, and a
/// vendor-prefixed spelling is the caller's to strip.
pub(crate) const FUNCTIONS: &[(&str, FunctionKind)] = &[
    // Colour notations (css-color-4 §§4-10).
    ("rgb", FunctionKind::ColorNotation),
    ("rgba", FunctionKind::ColorNotation),
    ("hsl", FunctionKind::ColorNotation),
    ("hsla", FunctionKind::ColorNotation),
    ("hwb", FunctionKind::ColorNotation),
    ("lab", FunctionKind::ColorNotation),
    ("lch", FunctionKind::ColorNotation),
    ("oklab", FunctionKind::ColorNotation),
    ("oklch", FunctionKind::ColorNotation),
    ("color", FunctionKind::ColorNotation),
    ("alpha", FunctionKind::RelativeColor),
    // Colours from other colours (css-color-5, css-color-6).
    (
        "color-mix",
        FunctionKind::ColorOperation(ColorOperation {
            canonical: crate::colors::mix::canonical,
            resolve: crate::colors::mix::resolve,
        }),
    ),
    (
        "color-layers",
        FunctionKind::ColorOperation(ColorOperation {
            canonical: crate::colors::layers::canonical,
            resolve: crate::colors::layers::resolve,
        }),
    ),
    (
        "contrast-color",
        FunctionKind::ColorOperation(ColorOperation {
            canonical: crate::colors::contrast::canonical,
            resolve: crate::colors::contrast::resolve,
        }),
    ),
    // Math functions (css-values-4 §10, css-values-5 for `progress()` and `calc-size()`).
    ("calc", FunctionKind::Math),
    ("calc-size", FunctionKind::Math),
    ("min", FunctionKind::Math),
    ("max", FunctionKind::Math),
    ("clamp", FunctionKind::Math),
    ("progress", FunctionKind::Math),
    ("round", FunctionKind::Math),
    ("mod", FunctionKind::Math),
    ("rem", FunctionKind::Math),
    ("abs", FunctionKind::Math),
    ("sign", FunctionKind::Math),
    ("pow", FunctionKind::Math),
    ("sqrt", FunctionKind::Math),
    ("hypot", FunctionKind::Math),
    ("log", FunctionKind::Math),
    ("exp", FunctionKind::Math),
    ("sin", FunctionKind::Math),
    ("cos", FunctionKind::Math),
    ("tan", FunctionKind::Math),
    ("asin", FunctionKind::Math),
    ("acos", FunctionKind::Math),
    ("atan", FunctionKind::Math),
    ("atan2", FunctionKind::Math),
    // Substitution functions.
    ("var", FunctionKind::Substitution { arbitrary: true }),
    ("env", FunctionKind::Substitution { arbitrary: true }),
    ("attr", FunctionKind::Substitution { arbitrary: true }),
    ("light-dark", FunctionKind::Substitution { arbitrary: false }),
    // The user-agent sheet's spelling of `light-dark()` for form controls.
    ("-internal-light-dark", FunctionKind::Substitution { arbitrary: false }),
];

/// What kind of function `name` is, or `None` for one the engine gives no meaning to.
#[must_use]
pub(crate) fn kind(name: &str) -> Option<FunctionKind> {
    FUNCTIONS
        .iter()
        .find(|(known, _)| known.eq_ignore_ascii_case(name))
        .map(|(_, kind)| *kind)
}

/// Whether `name` is a colour notation (`rgb()`, `oklch()`, `color()`, ...).
#[must_use]
pub(crate) fn is_color_notation(name: &str) -> bool {
    matches!(kind(name), Some(FunctionKind::ColorNotation))
}

/// The colour operation `name` is (`color-mix()`, ...), if it is one.
#[must_use]
pub(crate) fn color_operation(name: &str) -> Option<ColorOperation> {
    match kind(name) {
        Some(FunctionKind::ColorOperation(operation)) => Some(operation),
        _ => None,
    }
}

/// Whether `name` is a math function.
#[must_use]
pub(crate) fn is_math(name: &str) -> bool {
    matches!(kind(name), Some(FunctionKind::Math))
}

/// Whether `name` is substituted before its declaration is known.
#[must_use]
pub(crate) fn is_substitution(name: &str) -> bool {
    matches!(kind(name), Some(FunctionKind::Substitution { .. }))
}

/// Whether `name` can stand for any tokens at all, so its declaration waits to be validated.
#[must_use]
pub(crate) fn is_arbitrary_substitution(name: &str) -> bool {
    matches!(kind(name), Some(FunctionKind::Substitution { arbitrary: true }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The grammar's `<color-function>` and the registry name the same functions. The grammar
    /// comes from the definitions data and is patched in the generator, which cannot read this
    /// table, so this is what keeps the two from drifting: a colour function added to one and not
    /// the other either validates and then means nothing, or means something and never validates.
    #[test]
    fn the_color_function_grammar_names_the_registered_colour_functions() {
        let values: serde_json::Value =
            serde_json::from_str(crate::matcher::property_definitions::DEFINITIONS_VALUES).expect("values JSON");
        let syntax = values
            .as_array()
            .expect("a list of value definitions")
            .iter()
            .find(|entry| entry["name"] == "<color-function>")
            .and_then(|entry| entry["syntax"].as_str())
            .expect("a <color-function> definition");
        let mut in_grammar: Vec<String> = syntax
            .split('|')
            .map(|alternative| {
                alternative
                    .trim()
                    .trim_start_matches('<')
                    .trim_end_matches('>')
                    .trim_end_matches("()")
                    .to_string()
            })
            .collect();
        in_grammar.sort();

        let mut registered: Vec<String> = FUNCTIONS
            .iter()
            .filter(|(_, kind)| {
                matches!(
                    kind,
                    FunctionKind::ColorNotation | FunctionKind::RelativeColor | FunctionKind::ColorOperation(_)
                )
            })
            .map(|(name, _)| (*name).to_string())
            .collect();
        registered.sort();

        assert_eq!(in_grammar, registered);
    }

    #[test]
    fn names_are_case_insensitive() {
        assert!(is_color_notation("RGB"));
        assert!(color_operation("Color-Mix").is_some());
        assert!(is_math("CALC"));
        assert!(is_arbitrary_substitution("Var"));
        assert!(is_substitution("light-dark") && !is_arbitrary_substitution("light-dark"));
        assert!(kind("banana").is_none());
    }
}
