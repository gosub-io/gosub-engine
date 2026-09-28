//! `contrast-color()` (css-color-5 section 8).
//!
//! `contrast-color(<color>)` resolves to white or black, whichever gives more contrast for text
//! on a solid background of that colour. A tie resolves to white. The draft leaves the contrast
//! algorithm to the UA. This uses the WCAG 2.1 contrast ratio. The winner of white and black
//! always reaches at least 4.58:1, which meets the AA large-text level the draft asks for.
//!
//! The specified value keeps the function, with its argument in canonical form. The computed
//! value is the colour it picks. An argument that cannot be resolved at computed-value time,
//! such as `currentcolor` or a system colour, keeps the function in the computed value too.

use super::relative::{canonical_origin, resolve_origin};
use super::space::Space;
use super::{ColorSyntax, CssColor};
use crate::stylesheet::CssValue;

/// The canonical specified value of `contrast-color(args)`, or `None` when it is not valid.
#[must_use]
pub(crate) fn canonical(args: &[CssValue]) -> Option<CssValue> {
    let [color] = args else {
        return None;
    };
    let color = canonical_origin(color)?;
    Some(CssValue::Function("contrast-color".to_string(), vec![color]))
}

/// The colour `contrast-color(args)` computes to: opaque white or black.
#[must_use]
pub(crate) fn resolve(args: &[CssValue]) -> Option<CssColor> {
    let [color] = args else {
        return None;
    };
    let background = resolve_origin(color)?;
    let luminance = relative_luminance(&background);
    // WCAG 2.1 contrast ratio against white (luminance 1) and black (luminance 0).
    let against_white = 1.05 / (luminance + 0.05);
    let against_black = (luminance + 0.05) / 0.05;
    let channel = if against_white >= against_black { 255.0 } else { 0.0 };
    Some(CssColor::from_parts(
        ColorSyntax::Rgb,
        [Some(channel), Some(channel), Some(channel)],
        Some(1.0),
        false,
    ))
}

/// The WCAG 2.1 relative luminance of a colour used as a solid background.
///
/// The colour is taken as opaque, since the draft describes a solid background. It is clipped to
/// the sRGB gamut, which is what a display shows of it.
fn relative_luminance(color: &CssColor) -> f64 {
    let [r, g, b] = color
        .in_space(Space::SrgbLinear)
        .map(|c| c.unwrap_or(0.0).clamp(0.0, 1.0));
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(text: &str) -> CssValue {
        CssValue::String(text.to_string())
    }

    fn picked(color: &str) -> String {
        resolve(&[word(color)]).map(|c| c.to_string()).unwrap_or_default()
    }

    #[test]
    fn light_backgrounds_get_black_and_dark_ones_white() {
        assert_eq!(picked("white"), "rgb(0, 0, 0)");
        assert_eq!(picked("pink"), "rgb(0, 0, 0)");
        assert_eq!(picked("black"), "rgb(255, 255, 255)");
        assert_eq!(picked("blue"), "rgb(255, 255, 255)");
    }

    #[test]
    fn out_of_gamut_colours_are_clipped_first() {
        let bright = CssColor::from_parts(
            ColorSyntax::Predefined(super::super::PredefinedSpace::Srgb),
            [Some(10.0), Some(10.0), Some(10.0)],
            Some(1.0),
            false,
        );
        let value = resolve(&[CssValue::Color(bright)]).map(|c| c.to_string());
        assert_eq!(value.as_deref(), Some("rgb(0, 0, 0)"));
    }

    #[test]
    fn an_unresolvable_argument_keeps_the_function() {
        assert_eq!(resolve(&[word("currentcolor")]), None);
    }

    #[test]
    fn only_a_single_colour_is_accepted() {
        assert_eq!(canonical(&[]), None);
        assert_eq!(canonical(&[word("white"), word("white")]), None);
        assert_eq!(canonical(&[word("max")]), None);
        assert_eq!(
            canonical(&[word("White")]).map(|v| v.to_string()).as_deref(),
            Some("contrast-color(white)")
        );
    }
}
