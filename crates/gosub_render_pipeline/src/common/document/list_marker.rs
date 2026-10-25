//! The text of a list item's `::marker`: its `list-style-type` applied to its ordinal.
//!
//! The counter styles are the predefined ones of css-counter-styles-3 §6, each with the symbols
//! and suffix that section gives it. A style that cannot write a number - roman past 3999, an
//! alphabetic style below 1 - falls back to `decimal`, as §3.1.3 says.

use cow_utils::CowUtils;
use gosub_interface::style::ListStyleType;

/// The marker text, suffix included (`"3. "`, `"• "`), or `None` for `list-style-type: none`.
#[must_use]
pub fn marker_text(style: &ListStyleType, ordinal: i64) -> Option<String> {
    // The symbolic styles end in a space, the numeric ones in a full stop and a space.
    let symbol = |symbol: char| Some(format!("{symbol} "));
    let numbered = |text: String| Some(format!("{text}. "));
    match style {
        ListStyleType::None => None,
        ListStyleType::String(text) => Some(text.to_string()),
        ListStyleType::Disc => symbol('\u{2022}'),
        ListStyleType::Circle => symbol('\u{25E6}'),
        ListStyleType::Square => symbol('\u{25AA}'),
        ListStyleType::DisclosureOpen => symbol('\u{25BE}'),
        ListStyleType::DisclosureClosed => symbol('\u{25B8}'),
        ListStyleType::Decimal => numbered(ordinal.to_string()),
        ListStyleType::DecimalLeadingZero => numbered(decimal_leading_zero(ordinal)),
        ListStyleType::LowerRoman => numbered(roman(ordinal).unwrap_or_else(|| ordinal.to_string())),
        ListStyleType::UpperRoman => numbered(roman(ordinal).map_or_else(
            || ordinal.to_string(),
            |text| text.cow_to_ascii_uppercase().into_owned(),
        )),
        ListStyleType::LowerAlpha => numbered(alphabetic(ordinal, &LATIN).unwrap_or_else(|| ordinal.to_string())),
        ListStyleType::UpperAlpha => numbered(alphabetic(ordinal, &LATIN).map_or_else(
            || ordinal.to_string(),
            |text| text.cow_to_ascii_uppercase().into_owned(),
        )),
        ListStyleType::LowerGreek => numbered(alphabetic(ordinal, &GREEK).unwrap_or_else(|| ordinal.to_string())),
    }
}

const LATIN: [char; 26] = [
    'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm', 'n', 'o', 'p', 'q', 'r', 's', 't', 'u', 'v', 'w',
    'x', 'y', 'z',
];

/// `lower-greek`: the 24 letters, without the final sigma.
const GREEK: [char; 24] = [
    'α', 'β', 'γ', 'δ', 'ε', 'ζ', 'η', 'θ', 'ι', 'κ', 'λ', 'μ', 'ν', 'ξ', 'ο', 'π', 'ρ', 'σ', 'τ', 'υ', 'φ', 'χ', 'ψ',
    'ω',
];

/// `decimal-leading-zero`: at least two digits, the sign in front of the padding.
fn decimal_leading_zero(ordinal: i64) -> String {
    if ordinal < 0 {
        format!("-{:02}", ordinal.unsigned_abs())
    } else {
        format!("{ordinal:02}")
    }
}

/// An `additive` roman numeral, lowercase; `None` outside 1..=3999, which it has no symbols for.
fn roman(ordinal: i64) -> Option<String> {
    const SYMBOLS: [(i64, &str); 13] = [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ];
    if !(1..=3999).contains(&ordinal) {
        return None;
    }
    let mut rest = ordinal;
    let mut out = String::new();
    for (value, symbol) in SYMBOLS {
        while rest >= value {
            out.push_str(symbol);
            rest -= value;
        }
    }
    Some(out)
}

/// An `alphabetic` counter: bijective base-n over `letters` (`z`, `aa`, `ab`); `None` below 1.
fn alphabetic(ordinal: i64, letters: &[char]) -> Option<String> {
    if ordinal < 1 {
        return None;
    }
    let base = letters.len() as u64;
    let mut rest = ordinal.unsigned_abs();
    let mut out = Vec::new();
    while rest > 0 {
        rest -= 1;
        #[expect(clippy::cast_possible_truncation, reason = "the remainder is below the letter count")]
        out.push(letters[(rest % base) as usize]);
        rest /= base;
    }
    Some(out.iter().rev().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn text(style: ListStyleType, ordinal: i64) -> String {
        marker_text(&style, ordinal).unwrap_or_default()
    }

    #[test]
    fn symbols_carry_a_space_and_numbers_a_full_stop() {
        assert_eq!(text(ListStyleType::Disc, 1), "\u{2022} ");
        assert_eq!(text(ListStyleType::Square, 7), "\u{25AA} ");
        assert_eq!(text(ListStyleType::Decimal, 3), "3. ");
        assert_eq!(text(ListStyleType::Decimal, -2), "-2. ");
        assert_eq!(text(ListStyleType::DecimalLeadingZero, 7), "07. ");
        assert_eq!(text(ListStyleType::DecimalLeadingZero, -7), "-07. ");
        assert_eq!(text(ListStyleType::String(Arc::from("-> ")), 4), "-> ");
        assert_eq!(marker_text(&ListStyleType::None, 1), None);
    }

    #[test]
    fn roman_numerals_up_to_3999() {
        assert_eq!(text(ListStyleType::LowerRoman, 4), "iv. ");
        assert_eq!(text(ListStyleType::UpperRoman, 1994), "MCMXCIV. ");
        assert_eq!(text(ListStyleType::UpperRoman, 3999), "MMMCMXCIX. ");
        // Past what the symbols can write, and below one, the number is shown in decimal.
        assert_eq!(text(ListStyleType::LowerRoman, 4000), "4000. ");
        assert_eq!(text(ListStyleType::LowerRoman, 0), "0. ");
    }

    #[test]
    fn alphabetic_counts_on_after_the_last_letter() {
        assert_eq!(text(ListStyleType::LowerAlpha, 1), "a. ");
        assert_eq!(text(ListStyleType::LowerAlpha, 26), "z. ");
        assert_eq!(text(ListStyleType::LowerAlpha, 27), "aa. ");
        assert_eq!(text(ListStyleType::UpperAlpha, 703), "AAA. ");
        assert_eq!(text(ListStyleType::LowerGreek, 24), "ω. ");
        assert_eq!(text(ListStyleType::LowerGreek, 25), "αα. ");
        assert_eq!(text(ListStyleType::LowerAlpha, 0), "0. ");
    }
}
