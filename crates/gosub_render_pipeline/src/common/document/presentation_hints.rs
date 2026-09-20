//! HTML presentational attributes - `bgcolor`, `width`, `cellspacing`, `cellpadding` - as typed
//! style.
//!
//! PENDING STEP 3B. These belong in the cascade, at the presentational-hint origin the HTML
//! spec gives them (HTML §15.2: they cascade as author-level declarations in a stylesheet of
//! their own, below any real author rule). Applying them here instead is why the two of them
//! need different precedence rules spelled out by hand: `cellspacing`/`cellpadding` beat
//! user-agent rules but lose to author ones, and `bgcolor`/`width` lose to everything. Moving
//! them into the cascade makes both of those fall out of the origin order instead.
//!
//! What this module owns is the value shaping and the precedence; the adapter owns the DOM
//! access, since only it can find an element's enclosing table or read its attributes.

use std::collections::HashMap;

use gosub_interface::style::{Color, ComputedStyle, LengthPercentage, LengthPercentageAuto, Prop};
use gosub_shared::css_colors::named_color_hex;

/// The table presentational attributes that reached one element.
#[derive(Debug, Default, Clone, Copy)]
pub struct TableHints {
    /// `cellspacing` on this `<table>`, in px. `None` when the element is not a table, the
    /// attribute is absent, or an author rule set `border-spacing`.
    pub border_spacing: Option<f32>,
    /// `cellpadding` from this cell's enclosing `<table>`, in px, per side in top-right-
    /// bottom-left order. A side is `None` when the element is not an in-table cell or an
    /// author rule set that side's padding - which is per side, since an author who writes only
    /// `padding-left` leaves the other three to the hint.
    pub cell_padding: [Option<f32>; 4],
}

/// The default cell padding an in-table `<td>`/`<th>` gets when the table says nothing.
///
/// WebKit's `HTMLTableCellElement::additionalPresentationAttributeStyle` does the same - see
/// the user-agent sheet's `td:not(table td)` comment.
pub const DEFAULT_CELL_PADDING: f32 = 1.0;

/// A presentational length attribute (`cellspacing="4"`), which is a plain number of pixels.
/// Negative values are clamped away, as they are everywhere else in the rendering spec.
#[must_use]
pub fn attr_px(raw: &str) -> Option<f32> {
    raw.trim().parse::<f32>().ok().map(|value| value.max(0.0))
}

/// Apply the table attributes, which sit between the user-agent and author origins: they beat
/// the user-agent sheet's `table { border-spacing: 2px }` and the cell padding it gives, and
/// lose to any author declaration - which the adapter has already checked before filling in
/// [`TableHints`].
pub fn apply_table_hints(style: &mut ComputedStyle, hints: TableHints) {
    if let Some(spacing) = hints.border_spacing {
        let inherited = style.inherited_mut();
        inherited.border_spacing_x = spacing;
        inherited.border_spacing_y = spacing;
        style.declared.set(Prop::BorderSpacingX);
        style.declared.set(Prop::BorderSpacingY);
    }
    if hints.cell_padding.iter().all(Option::is_none) {
        return;
    }
    let mut declared = style.declared;
    let padding = style.padding_mut();
    let sides: [(&mut LengthPercentage, Prop); 4] = [
        (&mut padding.top, Prop::PaddingTop),
        (&mut padding.right, Prop::PaddingRight),
        (&mut padding.bottom, Prop::PaddingBottom),
        (&mut padding.left, Prop::PaddingLeft),
    ];
    for ((field, prop), hint) in sides.into_iter().zip(hints.cell_padding) {
        if let Some(padding) = hint {
            *field = LengthPercentage::Px(padding);
            declared.set(prop);
        }
    }
    style.declared = declared;
}

/// Apply the general presentational attributes, which lose to every real declaration: they only
/// fill in a property the cascade left alone.
pub fn apply_presentation_attrs(style: &mut ComputedStyle, attrs: &HashMap<String, String>) {
    if !style.has(Prop::BackgroundColor) {
        if let Some(color) = attrs.get("bgcolor").and_then(|raw| legacy_color(raw.trim())) {
            style.background_mut().color = color;
            style.declared.set(Prop::BackgroundColor);
        }
    }
    if !style.has(Prop::Width) {
        if let Some(width) = attrs.get("width").and_then(|raw| legacy_width(raw.trim())) {
            style.size_mut().width = width;
            style.declared.set(Prop::Width);
        }
    }
}

/// The `width` attribute: a percentage, or a plain number of pixels.
fn legacy_width(raw: &str) -> Option<LengthPercentageAuto> {
    if let Some(percent) = raw.strip_suffix('%') {
        return percent.trim().parse::<f32>().ok().map(LengthPercentageAuto::Percent);
    }
    raw.parse::<f32>().ok().map(LengthPercentageAuto::Px)
}

/// A colour written in an attribute rather than in CSS. Only the forms this engine has ever
/// accepted here: a name, a hex triple or quad, and the `rgb()`/`rgba()` functions.
///
/// PENDING STEP 3B: the HTML spec's own "legacy colour value" algorithm is more forgiving than
/// this - it takes `bgcolor="chucknorris"` - and once these run through the cascade the CSS
/// colour parser answers them instead.
fn legacy_color(raw: &str) -> Option<Color> {
    if raw.eq_ignore_ascii_case("transparent") {
        return Some(Color::TRANSPARENT);
    }
    if raw.starts_with("rgb(") {
        return parse_rgb(raw);
    }
    if raw.starts_with("rgba(") {
        return parse_rgba(raw);
    }
    if raw.starts_with('#') {
        return parse_hex(raw);
    }
    named_color_hex(raw).and_then(parse_hex)
}

fn parse_rgb(raw: &str) -> Option<Color> {
    let inner = raw.trim_start_matches("rgb(").trim_end_matches(')');
    let parts: Vec<&str> = inner.split(',').collect();
    match parts.as_slice() {
        [r, g, b] => Some(Color::rgba(
            r.trim().parse().unwrap_or(0),
            g.trim().parse().unwrap_or(0),
            b.trim().parse().unwrap_or(0),
            255,
        )),
        _ => None,
    }
}

#[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "alpha is a byte")]
fn parse_rgba(raw: &str) -> Option<Color> {
    let inner = raw.trim_start_matches("rgba(").trim_end_matches(')');
    let parts: Vec<&str> = inner.split(',').collect();
    match parts.as_slice() {
        [r, g, b, a] => Some(Color::rgba(
            r.trim().parse().unwrap_or(0),
            g.trim().parse().unwrap_or(0),
            b.trim().parse().unwrap_or(0),
            (a.trim().parse::<f32>().unwrap_or(1.0) * 255.0) as u8,
        )),
        _ => None,
    }
}

fn parse_hex(raw: &str) -> Option<Color> {
    let hex = raw.trim_start_matches('#');
    let byte = |range: std::ops::Range<usize>| u8::from_str_radix(hex.get(range)?, 16).ok();
    match hex.len() {
        6 => Some(Color::rgba(byte(0..2)?, byte(2..4)?, byte(4..6)?, 255)),
        8 => Some(Color::rgba(byte(0..2)?, byte(2..4)?, byte(4..6)?, byte(6..8)?)),
        3 => {
            let nibble = |i: usize| u8::from_str_radix(&hex.get(i..i + 1)?.repeat(2), 16).ok();
            Some(Color::rgba(nibble(0)?, nibble(1)?, nibble(2)?, 255))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_colors_come_from_the_shared_table() {
        assert_eq!(legacy_color("rebeccapurple"), Some(Color::rgba(0x66, 0x33, 0x99, 255)));
        assert_eq!(legacy_color("notacolor"), None);
        assert_eq!(legacy_color("transparent"), Some(Color::TRANSPARENT));
    }

    #[test]
    fn hex_forms() {
        assert_eq!(legacy_color("#abc"), Some(Color::rgba(0xaa, 0xbb, 0xcc, 255)));
        assert_eq!(legacy_color("#123456"), Some(Color::rgba(0x12, 0x34, 0x56, 255)));
        assert_eq!(legacy_color("#12345678"), Some(Color::rgba(0x12, 0x34, 0x56, 0x78)));
    }

    /// The `width` attribute is a number of pixels or a percentage, and nothing else.
    #[test]
    fn width_attribute_forms() {
        assert_eq!(legacy_width("120"), Some(LengthPercentageAuto::Px(120.0)));
        assert_eq!(legacy_width("50%"), Some(LengthPercentageAuto::Percent(50.0)));
        assert_eq!(legacy_width("auto"), None);
    }

    /// A cell padding hint overrides the user-agent sheet, so it has to report itself as
    /// declared - the readers that ask "did anyone set this" must see it.
    #[test]
    fn table_hints_report_themselves_as_declared() {
        let mut style = ComputedStyle::initial();
        apply_table_hints(
            &mut style,
            TableHints {
                border_spacing: Some(4.0),
                cell_padding: [Some(2.0); 4],
            },
        );
        assert_eq!(style.inherited.border_spacing_x, 4.0);
        assert_eq!(style.inherited.border_spacing_y, 4.0);
        assert!(style.has(Prop::BorderSpacingY));
        assert_eq!(style.padding.left, LengthPercentage::Px(2.0));
        assert!(style.has(Prop::PaddingLeft));
    }
}
