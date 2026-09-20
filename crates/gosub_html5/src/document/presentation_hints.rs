//! HTML presentational hints: the attributes that mean a CSS declaration (HTML §15.3).
//!
//! `<table cellspacing=4>` is `border-spacing: 4px` and `<img width=50>` is `width: 50px`. The
//! mapping is part of HTML rather than of CSS, so it lives here: the style system asks the
//! document for an element's hints and only has to rank what comes back. It ranks them in the
//! author origin at specificity zero, ahead of every author sheet, which is what makes a hint
//! beat the user-agent sheet and lose to anything the page itself wrote.
//!
//! What is produced is CSS text, parsed by the real parser like any other declaration block.
//! Every value written here is built from a number or a hex triple this module formatted
//! itself, and [`push`] drops anything carrying the characters that could end a declaration or
//! raise its priority - so no attribute value can smuggle an `!important` or a second rule in.

use cow_utils::CowUtils;
use gosub_shared::css_colors::named_color_hex;

/// Append `property: value` when the value cannot escape the declaration it is written into.
///
/// Nothing this module builds contains any of these characters; the check is here so that stays
/// true of anything added later. `!` most of all: a presentational hint is never important
/// (HTML §15.3.1), and the only way one could become important is by writing the word.
fn push(out: &mut String, property: &str, value: &str) {
    if value.is_empty() || value.contains(['!', ';', '{', '}', '"', '\'', '\\']) {
        return;
    }
    if !out.is_empty() {
        out.push(';');
    }
    out.push_str(property);
    out.push(':');
    out.push_str(value);
}

/// The default padding of a cell inside a table.
///
/// The user-agent sheet gives it to cells that are *not* in one (`td:not(table td)`) and leaves
/// the in-table case to this, the way Blink's `HTMLTableCellElement` does. Either way it is the
/// 1px the rendering spec's default sheet gives `td` and `th`, and either way an author rule
/// outranks it; routing it through the hint is what lets one `cellpadding` on the table replace
/// it for every cell at once.
const DEFAULT_CELL_PADDING: &str = "1px";

/// A markup dimension, as HTML's "rules for parsing dimension values" read one: an optional run
/// of digits with an optional fraction, then a `%` for a percentage and anything else for a
/// length in px. That is what makes `width="300px"` mean 300px, since the `p` merely ends the
/// number, and `width="50%"` a percentage.
fn dimension(raw: &str) -> Option<String> {
    let raw = raw.trim_start_matches([' ', '\t', '\n', '\u{0c}', '\r']);
    let bytes = raw.as_bytes();
    let mut pos = 0;
    while bytes.get(pos).is_some_and(u8::is_ascii_digit) {
        pos += 1;
    }
    if pos == 0 {
        return None;
    }
    if bytes.get(pos) == Some(&b'.') {
        pos += 1;
        while bytes.get(pos).is_some_and(u8::is_ascii_digit) {
            pos += 1;
        }
    }
    let number = raw.get(..pos)?;
    // A trailing `.` parses as a number here but not in CSS, so it is dropped rather than
    // written out as `50.px`.
    let number = number.trim_end_matches('.');
    let rest = raw.get(pos..)?.trim_start_matches([' ', '\t', '\n', '\u{0c}', '\r']);
    Some(if rest.starts_with('%') {
        format!("{number}%")
    } else {
        format!("{number}px")
    })
}

/// The same, for the attributes whose value may not be zero: a `<table width=0>` is an error
/// rather than a zero-width table.
fn nonzero_dimension(raw: &str) -> Option<String> {
    let value = dimension(raw)?;
    let number = value.trim_end_matches(['%', 'p', 'x']);
    (number.parse::<f64>().ok()? != 0.0).then_some(value)
}

/// A non-negative integer, as HTML's rules for parsing one read it: leading whitespace, an
/// optional `+`, then the digits, and whatever follows is ignored.
fn non_negative_integer(raw: &str) -> Option<u32> {
    let raw = raw.trim_start_matches([' ', '\t', '\n', '\u{0c}', '\r']);
    let raw = raw.strip_prefix('+').unwrap_or(raw);
    let digits: String = raw.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// HTML's "rules for parsing a legacy colour value", which answer `bgcolor`.
///
/// It is not the CSS colour parser and is not meant to be: `bgcolor="chucknorris"` is a real
/// colour on the web (`#cc0000`), because the algorithm keeps the hex digits it finds and fills
/// in the rest. Only two inputs take the short path - a colour keyword and a three-digit hex -
/// and everything else goes through the mangling below. The result is written as `#rrggbb`, so
/// what reaches the cascade is always a colour the CSS parser reads back the same way.
fn legacy_color(raw: &str) -> Option<String> {
    if raw.is_empty() {
        return None;
    }
    let input = raw.trim_matches([' ', '\t', '\n', '\u{0c}', '\r']);
    // The one keyword the algorithm rejects: `transparent` is not a legacy colour.
    if input.eq_ignore_ascii_case("transparent") {
        return None;
    }
    if let Some(hex) = named_color_hex(input) {
        return Some(hex.to_string());
    }
    if input.len() == 4 {
        if let Some(digits) = input.strip_prefix('#') {
            if digits.bytes().all(|b| b.is_ascii_hexdigit()) {
                let doubled: String = digits.chars().flat_map(|c| [c, c]).collect();
                return Some(format!("#{doubled}"));
            }
        }
    }

    // Anything outside the BMP counts as two characters, and only the first 128 are read.
    let mut text: String = input
        .chars()
        .flat_map(|c| if c as u32 > 0xffff { ['0', '0'] } else { [c, '\0'] })
        .filter(|&c| c != '\0')
        .collect();
    text.truncate(128);
    let mut digits: Vec<u8> = text
        .strip_prefix('#')
        .unwrap_or(&text)
        .bytes()
        .map(|b| if b.is_ascii_hexdigit() { b } else { b'0' })
        .collect();
    while digits.is_empty() || !digits.len().is_multiple_of(3) {
        digits.push(b'0');
    }

    let mut length = digits.len() / 3;
    let mut offset = 0;
    // Keep the last eight digits of each component, drop the leading zeros all three share,
    // then keep the first two of what is left.
    if length > 8 {
        offset = length - 8;
        length = 8;
    }
    let component = |index: usize, offset: usize, length: usize| -> &[u8] {
        let start = index * (digits.len() / 3) + offset;
        digits.get(start..start + length).unwrap_or(&[])
    };
    while length > 2 && (0..3).all(|index| component(index, offset, length).first() == Some(&b'0')) {
        offset += 1;
        length -= 1;
    }
    length = length.min(2);

    let mut out = String::from("#");
    for index in 0..3 {
        let part = component(index, offset, length);
        let value = u8::from_str_radix(core::str::from_utf8(part).ok()?, 16).ok()?;
        out.push_str(&format!("{value:02x}"));
    }
    Some(out)
}

/// The elements whose `bgcolor` is a `background-color` (HTML §15.3.3 and §15.3.10).
fn maps_bgcolor(tag: &str) -> bool {
    matches!(tag, "body" | "table" | "thead" | "tbody" | "tfoot" | "tr" | "td" | "th")
}

/// What an element's attributes say about it in CSS, as the body of a declaration block.
///
/// `lookup` reads an attribute of the element itself. A cell's padding comes from the `<table>`
/// it sits in rather than from the cell, so the caller finds that table - this module has no
/// document to walk - and answers `in_table_cell` and `table_attr` from it. A `<td>` outside any
/// table is not a cell for this purpose and keeps the user-agent sheet's own padding.
pub fn hints_for<'a>(
    tag: &str,
    svg: bool,
    lookup: impl Fn(&str) -> Option<&'a str>,
    table_attr: impl Fn(&str) -> Option<&'a str>,
    in_table_cell: bool,
) -> Option<String> {
    let tag = tag.cow_to_ascii_lowercase();
    let tag = tag.as_ref();
    let mut out = String::new();

    // SVG has the same idea under another name: a presentation attribute is a declaration at
    // specificity zero in the author origin (SVG 2 §6.6), and `width` is one of them - which is
    // what sizes the `<svg>` box an HTML page embeds. Its value is CSS rather than markup, but
    // for this attribute the two agree: a bare number is a length in px.
    if svg {
        if let Some(width) = lookup("width").and_then(dimension) {
            push(&mut out, "width", &width);
        }
        return (!out.is_empty()).then_some(out);
    }

    // §15.3.10: the table's own cell spacing, in px.
    if tag == "table" {
        if let Some(spacing) = lookup("cellspacing").and_then(non_negative_integer) {
            push(&mut out, "border-spacing", &format!("{spacing}px"));
        }
    }

    // §15.3.10: a cell's padding comes from the table it is in, not from the cell.
    if in_table_cell {
        let padding = table_attr("cellpadding")
            .and_then(non_negative_integer)
            .map_or_else(|| DEFAULT_CELL_PADDING.to_string(), |value| format!("{value}px"));
        for side in ["padding-top", "padding-right", "padding-bottom", "padding-left"] {
            push(&mut out, side, &padding);
        }
    }

    if maps_bgcolor(tag) {
        if let Some(color) = lookup("bgcolor").and_then(legacy_color) {
            push(&mut out, "background-color", &color);
        }
    }

    // §15.3.9, the dimension attributes: a replaced element's `width` is a CSS width.
    let dimension_attr = match tag {
        "img" | "embed" | "iframe" | "object" | "video" => true,
        "input" => lookup("type").is_some_and(|value| value.eq_ignore_ascii_case("image")),
        _ => false,
    };
    if dimension_attr {
        if let Some(width) = lookup("width").and_then(dimension) {
            push(&mut out, "width", &width);
        }
    } else if matches!(tag, "table" | "td" | "th" | "col" | "colgroup" | "hr") {
        // §15.3.6 and §15.3.10: the same attribute on the table family and on `<hr>`, where a
        // zero is an error rather than a zero-width box.
        if let Some(width) = lookup("width").and_then(nonzero_dimension) {
            push(&mut out, "width", &width);
        }
    }

    (!out.is_empty()).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none(_: &str) -> Option<&'static str> {
        None
    }

    #[test]
    fn dimension_values_follow_the_html_rules() {
        assert_eq!(dimension("300"), Some("300px".to_string()));
        assert_eq!(dimension("300px"), Some("300px".to_string()));
        assert_eq!(dimension("50%"), Some("50%".to_string()));
        assert_eq!(dimension("  12.5 % "), Some("12.5%".to_string()));
        assert_eq!(dimension("auto"), None);
        // A trailing dot ends the number without being part of it.
        assert_eq!(dimension("50."), Some("50px".to_string()));
    }

    /// A zero is an error for the attributes that use the nonzero rules, and only for those.
    #[test]
    fn a_zero_is_only_an_error_where_the_rules_say_so() {
        assert_eq!(nonzero_dimension("0"), None);
        assert_eq!(nonzero_dimension("0%"), None);
        assert_eq!(dimension("0"), Some("0px".to_string()));
    }

    #[test]
    fn legacy_colours_keep_the_hex_digits_they_find() {
        assert_eq!(legacy_color("red").as_deref(), Some("#ff0000"));
        assert_eq!(legacy_color("#abc").as_deref(), Some("#aabbcc"));
        assert_eq!(legacy_color("#123456").as_deref(), Some("#123456"));
        assert_eq!(legacy_color("transparent"), None);
        // The algorithm's party trick: every non-hex character becomes a zero, which leaves
        // `chucknorris` a shade of red.
        assert_eq!(legacy_color("chucknorris").as_deref(), Some("#c00000"));
    }

    /// A hint is CSS text, so a value that could end the declaration is dropped rather than
    /// written out - `!important` above all, which a presentational hint may never be.
    #[test]
    fn a_value_cannot_escape_its_declaration() {
        let mut out = String::new();
        push(&mut out, "width", "10px !important");
        push(&mut out, "width", "10px;color:red");
        assert!(out.is_empty());
    }

    #[test]
    fn table_attributes_map_to_the_table_and_its_cells() {
        let table = hints_for(
            "TABLE",
            false,
            |name| (name == "cellspacing").then_some("4"),
            none,
            false,
        );
        assert_eq!(table.as_deref(), Some("border-spacing:4px"));

        let cell = hints_for("td", false, none, |name| (name == "cellpadding").then_some("6"), true);
        assert_eq!(
            cell.as_deref(),
            Some("padding-top:6px;padding-right:6px;padding-bottom:6px;padding-left:6px")
        );

        // A cell inside a table with no `cellpadding` keeps the 1px the default sheet gives it.
        let bare = hints_for("th", false, none, none, true);
        assert_eq!(
            bare.as_deref(),
            Some("padding-top:1px;padding-right:1px;padding-bottom:1px;padding-left:1px")
        );
        // A cell outside one has no hint at all.
        assert_eq!(hints_for("td", false, none, none, false), None);
    }

    /// `width` is only a CSS width on the elements the spec names, so a `<div width=100>` is
    /// still an attribute nothing reads.
    #[test]
    fn width_maps_only_where_the_spec_maps_it() {
        let width = |tag: &str| hints_for(tag, false, |name| (name == "width").then_some("120"), none, false);
        assert_eq!(width("img").as_deref(), Some("width:120px"));
        assert_eq!(width("hr").as_deref(), Some("width:120px"));
        assert_eq!(width("div"), None);
        assert_eq!(width("span"), None);

        // On `<input>` only the image button has dimensions.
        let input = |kind: Option<&'static str>| {
            let attr = |name: &str| match name {
                "width" => Some("10"),
                "type" => kind,
                _ => None,
            };
            hints_for("input", false, attr, none, false)
        };
        assert_eq!(input(Some("image")).as_deref(), Some("width:10px"));
        assert_eq!(input(Some("text")), None);
        assert_eq!(input(None), None);
    }

    #[test]
    fn bgcolor_maps_only_on_the_elements_that_carry_a_background() {
        let bg = |tag: &str| hints_for(tag, false, |name| (name == "bgcolor").then_some("#0f0"), none, false);
        assert_eq!(bg("body").as_deref(), Some("background-color:#00ff00"));
        assert_eq!(bg("tr").as_deref(), Some("background-color:#00ff00"));
        assert_eq!(bg("div"), None);
    }
}
