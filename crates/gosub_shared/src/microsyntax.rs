//! HTML's common microsyntaxes (HTML 2.3.4): how attribute values are read as numbers. They
//! live here rather than in the HTML parser crate because the render pipeline reads attributes too.

/// The rules for parsing floating-point number values (HTML 2.3.4.3).
///
/// These are lenient where Rust's own parser is strict and strict where it is lenient. Leading
/// ASCII whitespace, a leading `+`, a bare `.5` and anything after the number (`5px`, `1.x`, `2e`)
/// are accepted. `NaN`, `inf`, `infinity` and a value that rounds past the largest double
/// (`1e999`) are errors, which is what keeps them out of the arithmetic that reads them. `-0` is
/// 0.
///
/// The algorithm accumulates the value digit by digit. This collects the digits it would read
/// into a decimal string instead, and lets the standard parser do the rounding, which it does
/// exactly: the nearest double, ties to even, as the spec's conversion step asks.
#[must_use]
pub fn parse_floating_point_number(input: &str) -> Option<f64> {
    let input = input.trim_start_matches(|c: char| c.is_ascii_whitespace());
    let bytes = input.as_bytes();
    let digits_from = |at: usize| {
        bytes[at.min(bytes.len())..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count()
    };

    let mut number = String::new();
    let mut position = 0;
    match bytes.first()? {
        b'-' => {
            number.push('-');
            position = 1;
        }
        b'+' => position = 1,
        _ => {}
    }

    // The integer part: digits, or a `.` straight followed by one.
    if bytes.get(position) == Some(&b'.') && bytes.get(position + 1).is_some_and(u8::is_ascii_digit) {
        number.push('0');
    } else {
        let count = digits_from(position);
        if count == 0 {
            return None;
        }
        number.push_str(&input[position..position + count]);
        position += count;
    }

    // The fraction. A `.` followed by anything but a digit or an exponent ends the number.
    if bytes.get(position) == Some(&b'.') {
        position += 1;
        let count = digits_from(position);
        if count > 0 {
            number.push('.');
            number.push_str(&input[position..position + count]);
            position += count;
        } else if !matches!(bytes.get(position), Some(b'e' | b'E')) {
            return convert(&number);
        }
    }

    // The exponent. Without digits after it, `e` and its sign are ignored.
    if matches!(bytes.get(position), Some(b'e' | b'E')) {
        position += 1;
        let sign = match bytes.get(position) {
            Some(b'-') => {
                position += 1;
                "-"
            }
            Some(b'+') => {
                position += 1;
                ""
            }
            _ => "",
        };
        let count = digits_from(position);
        if count > 0 {
            number.push('e');
            number.push_str(sign);
            number.push_str(&input[position..position + count]);
        }
    }

    convert(&number)
}

/// The conversion step: the nearest finite double, an error past the largest one, and no `-0`.
fn convert(number: &str) -> Option<f64> {
    let value = number.parse::<f64>().ok().filter(|value| value.is_finite())?;
    Some(if value == 0.0 { 0.0 } else { value })
}

#[cfg(test)]
mod tests {
    use super::parse_floating_point_number as parse;

    #[test]
    fn reads_plain_numbers() {
        assert_eq!(parse("0"), Some(0.0));
        assert_eq!(parse("42"), Some(42.0));
        assert_eq!(parse("-3.25"), Some(-3.25));
        assert_eq!(parse("1e3"), Some(1000.0));
        assert_eq!(parse("2.5E-2"), Some(0.025));
        assert_eq!(parse("1e+2"), Some(100.0));
        assert_eq!(parse("1.e2"), Some(100.0));
    }

    #[test]
    fn takes_what_the_rules_allow_and_rust_does_not() {
        assert_eq!(parse("  \t\n7"), Some(7.0));
        assert_eq!(parse("+7"), Some(7.0));
        assert_eq!(parse(".5"), Some(0.5));
        assert_eq!(parse("-.5"), Some(-0.5));
        assert_eq!(parse("5px"), Some(5.0));
        assert_eq!(parse("1.x"), Some(1.0));
        assert_eq!(parse("1."), Some(1.0));
        assert_eq!(parse("2e"), Some(2.0));
        assert_eq!(parse("2e-"), Some(2.0));
        assert_eq!(parse("2ex"), Some(2.0));
        assert_eq!(parse("3.5.6"), Some(3.5));
    }

    #[test]
    fn rejects_what_is_not_a_number() {
        assert_eq!(parse(""), None);
        assert_eq!(parse("   "), None);
        assert_eq!(parse("-"), None);
        assert_eq!(parse("+"), None);
        assert_eq!(parse("."), None);
        assert_eq!(parse(".e1"), None);
        assert_eq!(parse("-+1"), None);
        assert_eq!(parse("x1"), None);
        // Only ASCII whitespace is skipped.
        assert_eq!(parse("\u{a0}1"), None);
    }

    #[test]
    fn rejects_nan_and_infinities() {
        assert_eq!(parse("NaN"), None);
        assert_eq!(parse("nan"), None);
        assert_eq!(parse("inf"), None);
        assert_eq!(parse("-Infinity"), None);
        assert_eq!(parse("1e999"), None);
        assert_eq!(parse("-1e309"), None);
    }

    #[test]
    fn rounds_to_the_nearest_double() {
        assert_eq!(parse("1.7976931348623157e308"), Some(f64::MAX));
        assert_eq!(parse("1e-400"), Some(0.0));
        assert_eq!(parse("0.1"), Some(0.1));
    }

    #[test]
    fn negative_zero_is_zero() {
        let zero = parse("-0").expect("a number");
        assert!(zero == 0.0 && zero.is_sign_positive());
        assert!(parse("-0.0e5").expect("a number").is_sign_positive());
    }
}
