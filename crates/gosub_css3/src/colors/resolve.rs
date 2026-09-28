//! Resolving a `<color>` value to a colour, in one place.
//!
//! A colour used to be worked out wherever it was needed: the typed style had its own
//! `currentcolor` substitution and its own system-colour table, the relative colour syntax had a
//! third way to read its origin, and a string parser turned `currentcolor` into black. Each place
//! knew a different subset - a system colour could not be the origin of `rgb(from ...)`, and nine
//! system colours had no value at all.
//!
//! [`resolve_color`] is the one answer. Its result says which of three things a value is,
//! because two of them look the same to an `Option`: a value that is not a colour, and one that
//! is a colour as soon as the element's own `color` is known (`currentcolor`, and any colour
//! function built on it). The computed stage has to leave the second kind standing; the typed
//! style, which knows the element's colour, finishes it.

use super::{CssColor, RgbColor};
use crate::matcher::styling::css_wide_keyword;
use crate::stylesheet::{fold_color_function, ColorStage, CssValue};

/// What a value is, as a colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Resolution {
    /// The colour it names.
    Color(CssColor),
    /// A colour once `currentcolor` has a value: the keyword itself, or a colour function that
    /// mentions it (`color-mix(in srgb, currentcolor, red)`).
    NeedsCurrent,
    /// Not a colour at all.
    NotAColor,
}

impl Resolution {
    /// The colour, when there is one.
    pub(crate) fn color(self) -> Option<CssColor> {
        match self {
            Resolution::Color(color) => Some(color),
            Resolution::NeedsCurrent | Resolution::NotAColor => None,
        }
    }
}

/// What a colour resolves against.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ColorContext {
    /// The element's own `color`, which `currentcolor` stands for. `None` where it is not known
    /// yet, which leaves anything built on `currentcolor` as [`Resolution::NeedsCurrent`].
    pub current: Option<CssColor>,
}

/// Resolve a `<color>` value.
///
/// System colours resolve to the light palette's value (see
/// `gosub_shared::css_colors::CSS_SYSTEM_COLORS`), and the deprecated ones to the colour
/// css-color-4 §6.3 maps them to.
pub(crate) fn resolve_color(value: &CssValue, context: &ColorContext) -> Resolution {
    match value {
        CssValue::Color(color) => Resolution::Color(*color),
        CssValue::String(keyword) => {
            if keyword.eq_ignore_ascii_case("currentcolor") {
                return context.current.map_or(Resolution::NeedsCurrent, Resolution::Color);
            }
            // The cascade resolves the CSS-wide keywords before a value gets here; one that
            // arrives anyway is not a colour, and must not be read as one.
            if css_wide_keyword(value).is_some() {
                return Resolution::NotAColor;
            }
            RgbColor::try_from_str(keyword).map_or(Resolution::NotAColor, |rgb| Resolution::Color(CssColor::from(rgb)))
        }
        CssValue::Function(name, args) => {
            if args.iter().any(mentions_current_color) {
                let Some(current) = context.current else {
                    return Resolution::NeedsCurrent;
                };
                let args: Vec<CssValue> = args.iter().map(|arg| with_current_color(arg, current)).collect();
                return fold(name, &args);
            }
            fold(name, args)
        }
        _ => Resolution::NotAColor,
    }
}

fn fold(name: &str, args: &[CssValue]) -> Resolution {
    fold_color_function(name, args, ColorStage::Computed).map_or(Resolution::NotAColor, Resolution::Color)
}

fn is_current_color(value: &CssValue) -> bool {
    matches!(value, CssValue::String(keyword) if keyword.eq_ignore_ascii_case("currentcolor"))
}

/// Whether `currentcolor` appears anywhere in a value, however deeply nested.
fn mentions_current_color(value: &CssValue) -> bool {
    match value {
        CssValue::Function(_, args) | CssValue::List(args) => args.iter().any(mentions_current_color),
        other => is_current_color(other),
    }
}

/// A value with every `currentcolor` in it replaced by `current`.
fn with_current_color(value: &CssValue, current: CssColor) -> CssValue {
    match value {
        CssValue::Function(name, args) => CssValue::Function(
            name.clone(),
            args.iter().map(|arg| with_current_color(arg, current)).collect(),
        ),
        CssValue::List(items) => CssValue::List(items.iter().map(|item| with_current_color(item, current)).collect()),
        other if is_current_color(other) => CssValue::Color(current),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keyword(text: &str) -> CssValue {
        CssValue::String(text.to_string())
    }

    fn rgb(r: f32, g: f32, b: f32) -> CssColor {
        CssColor::from(RgbColor::new(r, g, b, 255.0))
    }

    #[test]
    fn currentcolor_needs_the_elements_colour() {
        let none = ColorContext::default();
        assert_eq!(resolve_color(&keyword("currentColor"), &none), Resolution::NeedsCurrent);
        let red = ColorContext {
            current: Some(rgb(255.0, 0.0, 0.0)),
        };
        assert_eq!(
            resolve_color(&keyword("currentcolor"), &red),
            Resolution::Color(rgb(255.0, 0.0, 0.0))
        );

        // A colour function built on it waits the same way, and folds once it has a value.
        let contrast = CssValue::Function("contrast-color".to_string(), vec![keyword("currentcolor")]);
        assert_eq!(resolve_color(&contrast, &none), Resolution::NeedsCurrent);
        assert!(matches!(resolve_color(&contrast, &red), Resolution::Color(_)));
    }

    #[test]
    fn every_system_colour_resolves() {
        let none = ColorContext::default();
        for (name, _) in crate::colors::CSS_SYSTEM_COLORS {
            assert!(resolve_color(&keyword(name), &none).color().is_some(), "{name}");
        }
        for (name, current) in crate::colors::CSS_DEPRECATED_SYSTEM_COLORS {
            assert_eq!(
                resolve_color(&keyword(name), &none),
                resolve_color(&keyword(current), &none),
                "{name}"
            );
        }
    }

    #[test]
    fn a_non_colour_is_not_a_colour() {
        let none = ColorContext::default();
        for word in ["none", "no-repeat", "inherit", "banana"] {
            assert_eq!(resolve_color(&keyword(word), &none), Resolution::NotAColor, "{word}");
        }
    }
}
