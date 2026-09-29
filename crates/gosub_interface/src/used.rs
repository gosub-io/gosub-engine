//! The used-value stage: what a computed value comes to once layout has decided what it depends
//! on (css-cascade-5 §4.5).
//!
//! The computed style keeps a percentage as a percentage and a `calc()` that mixes one with a
//! length as a sum, because neither has a value before layout knows what it is a percentage of.
//! The functions here finish them. Each takes the basis its property refers to - the containing
//! block, the border box, the font size - since a percentage means something different on each,
//! and nothing here is stored: a used value is worked out where it is read.
//!
//! `currentcolor` is the other value the spec leaves to this stage. On the typed colour fields
//! it is settled when the typed style is built, which gives the same answer: only `color`
//! inherits among them, and the element's own `color` is final by then. A colour read from an
//! untyped value, such as a gradient stop, is resolved through
//! [`CssValue::used_color`](crate::css3::CssValue::used_color).
//!
//! The arithmetic is in `f64`, which is what layout works in.

use crate::style::{BorderGroup, LengthPercentage, LengthPercentageAuto, LetterSpacing};

/// A length or percentage in px, a percentage being of `basis`.
#[must_use]
pub fn length(value: LengthPercentage, basis: f64) -> f64 {
    match value {
        LengthPercentage::Px(px) => f64::from(px),
        LengthPercentage::Percent(pct) => basis * f64::from(pct) / 100.0,
        LengthPercentage::Calc { px, percent } => f64::from(px) + basis * f64::from(percent) / 100.0,
    }
}

/// A length, percentage or `auto` in px; `None` for `auto`.
#[must_use]
pub fn length_auto(value: LengthPercentageAuto, basis: f64) -> Option<f64> {
    match value {
        LengthPercentageAuto::Auto => None,
        LengthPercentageAuto::Px(px) => Some(length(LengthPercentage::Px(px), basis)),
        LengthPercentageAuto::Percent(pct) => Some(length(LengthPercentage::Percent(pct), basis)),
        LengthPercentageAuto::Calc { px, percent } => Some(length(LengthPercentage::Calc { px, percent }, basis)),
    }
}

/// `letter-spacing` in px. A percentage is of the font size (css-text-4 §8.2), and `normal`
/// adds nothing.
#[must_use]
pub fn letter_spacing(value: LetterSpacing, font_size: f64) -> f64 {
    match value {
        LetterSpacing::Normal => 0.0,
        LetterSpacing::Length(length_percentage) => length(length_percentage, font_size),
    }
}

/// One corner's radii: horizontal, then vertical.
pub type CornerRadius = (f64, f64);

/// The four corner radii of a box, top-left, top-right, bottom-right, bottom-left
/// (css-backgrounds-3 §5).
///
/// A percentage is of the box's width for the horizontal radius and of its height for the
/// vertical one, so `50%` on a rectangle is an ellipse. Where two adjacent radii add up to more
/// than the side they share, every radius is scaled down by the same factor until none overlap
/// (§5.5): `border-radius: 9999px`, the usual pill, comes to half the shorter side.
#[must_use]
pub fn border_radii(border: &BorderGroup, width: f64, height: f64) -> [CornerRadius; 4] {
    let corner = |value: LengthPercentage| (length(value, width).max(0.0), length(value, height).max(0.0));
    let radii = [
        corner(border.top_left_radius),
        corner(border.top_right_radius),
        corner(border.bottom_right_radius),
        corner(border.bottom_left_radius),
    ];
    let [tl, tr, br, bl] = radii;
    let fit = |side: f64, sum: f64| if sum > 0.0 { side / sum } else { f64::INFINITY };
    let factor = fit(width, tl.0 + tr.0)
        .min(fit(width, bl.0 + br.0))
        .min(fit(height, tl.1 + bl.1))
        .min(fit(height, tr.1 + br.1));
    if factor < 1.0 {
        radii.map(|(x, y)| (x * factor, y * factor))
    } else {
        radii
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn border(radius: LengthPercentage) -> BorderGroup {
        BorderGroup {
            top_left_radius: radius,
            top_right_radius: radius,
            bottom_right_radius: radius,
            bottom_left_radius: radius,
            ..(*crate::style::ComputedStyle::default().border).clone()
        }
    }

    #[test]
    fn a_percentage_is_of_its_basis() {
        assert_eq!(length(LengthPercentage::Percent(25.0), 200.0), 50.0);
        assert_eq!(
            length(
                LengthPercentage::Calc {
                    px: 10.0,
                    percent: 50.0
                },
                200.0
            ),
            110.0
        );
        assert_eq!(length_auto(LengthPercentageAuto::Auto, 200.0), None);
        assert_eq!(
            letter_spacing(LetterSpacing::Length(LengthPercentage::Percent(10.0)), 20.0),
            2.0
        );
    }

    /// `50%` on a rectangle is an ellipse, not 50px.
    #[test]
    fn a_percentage_radius_is_of_the_box() {
        let radii = border_radii(&border(LengthPercentage::Percent(50.0)), 200.0, 100.0);
        assert_eq!(radii, [(100.0, 50.0); 4]);
    }

    #[test]
    fn overlapping_radii_scale_down_together() {
        let radii = border_radii(&border(LengthPercentage::Px(9999.0)), 100.0, 30.0);
        assert!(
            radii.iter().all(|&(x, y)| (x - 15.0).abs() < 1e-9 && x == y),
            "{radii:?}"
        );
        // Radii that fit are left alone.
        let radii = border_radii(&border(LengthPercentage::Px(4.0)), 100.0, 30.0);
        assert_eq!(radii, [(4.0, 4.0); 4]);
    }
}
