//! Base64 utility methods (`atob`/`btoa`) as described by
//! <https://html.spec.whatwg.org/multipage/webappapis.html#atob>, built on the
//! forgiving-base64 algorithms from <https://infra.spec.whatwg.org/#forgiving-base64>

use crate::dom_exception::{DomException, ErrorName};

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// The index of a character in the base64 alphabet, or None if it isn't in it
fn decode_char(c: char) -> Option<u32> {
    match c {
        'A'..='Z' => Some(c as u32 - 'A' as u32),
        'a'..='z' => Some(c as u32 - 'a' as u32 + 26),
        '0'..='9' => Some(c as u32 - '0' as u32 + 52),
        '+' => Some(62),
        '/' => Some(63),
        _ => None,
    }
}

/// Forgiving-base64 encode per the Infra standard (plain base64 with padding)
#[must_use]
pub fn forgiving_base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);

    let (chunks, remainder) = bytes.as_chunks::<3>();
    for chunk in chunks {
        let n = u32::from(chunk[0]) << 16 | u32::from(chunk[1]) << 8 | u32::from(chunk[2]);
        out.push(char::from(ALPHABET[(n >> 18) as usize & 0x3F]));
        out.push(char::from(ALPHABET[(n >> 12) as usize & 0x3F]));
        out.push(char::from(ALPHABET[(n >> 6) as usize & 0x3F]));
        out.push(char::from(ALPHABET[n as usize & 0x3F]));
    }

    match *remainder {
        [b0] => {
            let n = u32::from(b0) << 16;
            out.push(char::from(ALPHABET[(n >> 18) as usize & 0x3F]));
            out.push(char::from(ALPHABET[(n >> 12) as usize & 0x3F]));
            out.push_str("==");
        }
        [b0, b1] => {
            let n = u32::from(b0) << 16 | u32::from(b1) << 8;
            out.push(char::from(ALPHABET[(n >> 18) as usize & 0x3F]));
            out.push(char::from(ALPHABET[(n >> 12) as usize & 0x3F]));
            out.push(char::from(ALPHABET[(n >> 6) as usize & 0x3F]));
            out.push('=');
        }
        _ => {}
    }

    out
}

/// Forgiving-base64 decode per the Infra standard: ASCII whitespace is
/// stripped, padding is optional (but at most two `=` and only on a
/// 4-boundary), and leftover bits in the final chunk are discarded.
pub fn forgiving_base64_decode(data: &str) -> Result<Vec<u8>, DomException> {
    let mut chars: Vec<char> = data
        .chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\x0C' | '\r' | ' '))
        .collect();

    if chars.len().is_multiple_of(4) {
        if chars.ends_with(&['=', '=']) {
            chars.truncate(chars.len() - 2);
        } else if chars.ends_with(&['=']) {
            chars.truncate(chars.len() - 1);
        }
    }

    if chars.len() % 4 == 1 {
        return Err(DomException::with_name(
            ErrorName::InvalidCharacterError,
            "input length is invalid for base64",
        ));
    }

    let mut out = Vec::with_capacity(chars.len() / 4 * 3 + 2);
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;

    for c in chars {
        let Some(value) = decode_char(c) else {
            return Err(DomException::with_name(
                ErrorName::InvalidCharacterError,
                "input contains a character outside of the base64 alphabet",
            ));
        };

        buffer = buffer << 6 | value;
        bits += 6;
        if bits == 24 {
            out.push((buffer >> 16) as u8);
            out.push((buffer >> 8) as u8);
            out.push(buffer as u8);
            buffer = 0;
            bits = 0;
        }
    }

    match bits {
        12 => out.push((buffer >> 4) as u8),
        18 => {
            out.push((buffer >> 10) as u8);
            out.push((buffer >> 2) as u8);
        }
        _ => {}
    }

    Ok(out)
}

/// `btoa(data)`: base64-encode a string of Latin1 code points. Throws an
/// `InvalidCharacterError` if the string contains a code point above U+00FF.
pub fn btoa(data: &str) -> Result<String, DomException> {
    let mut bytes = Vec::with_capacity(data.len());
    for c in data.chars() {
        let cp = c as u32;
        if cp > 0xFF {
            return Err(DomException::with_name(
                ErrorName::InvalidCharacterError,
                "string contains a character outside of the Latin1 range",
            ));
        }
        bytes.push(cp as u8);
    }

    Ok(forgiving_base64_encode(&bytes))
}

/// `atob(data)`: forgiving-base64 decode to a binary string — each decoded
/// byte becomes one code point in the U+0000..=U+00FF range.
pub fn atob(data: &str) -> Result<String, DomException> {
    let bytes = forgiving_base64_decode(data)?;
    Ok(bytes.into_iter().map(char::from).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom_exception::DomException;

    #[test]
    fn btoa_encodes_all_padding_lengths() {
        assert_eq!(btoa("").as_deref(), Ok(""));
        assert_eq!(btoa("a").as_deref(), Ok("YQ=="));
        assert_eq!(btoa("ab").as_deref(), Ok("YWI="));
        assert_eq!(btoa("abc").as_deref(), Ok("YWJj"));
        assert_eq!(btoa("hello world").as_deref(), Ok("aGVsbG8gd29ybGQ="));
    }

    #[test]
    fn btoa_covers_latin1_range() {
        assert_eq!(btoa("\u{FF}").as_deref(), Ok("/w=="));
        assert_eq!(btoa("\u{00}").as_deref(), Ok("AA=="));
    }

    #[test]
    fn btoa_rejects_above_latin1() {
        let err = btoa("snowman \u{2603}").unwrap_err();
        assert_eq!(err.name(), "InvalidCharacterError");
        assert_eq!(err.code(), DomException::INVALID_CHARACTER_ERR);
    }

    #[test]
    fn atob_decodes_padded_and_unpadded() {
        assert_eq!(atob("").as_deref(), Ok(""));
        assert_eq!(atob("YQ==").as_deref(), Ok("a"));
        assert_eq!(atob("YQ").as_deref(), Ok("a"));
        assert_eq!(atob("YWJj").as_deref(), Ok("abc"));
        assert_eq!(atob("/w==").as_deref(), Ok("\u{FF}"));
    }

    #[test]
    fn atob_strips_ascii_whitespace() {
        assert_eq!(atob(" Y\tW\nJ\rj \x0C").as_deref(), Ok("abc"));
    }

    #[test]
    fn atob_discards_leftover_bits() {
        // "YR" is 011000 010001; the final 4 bits are dropped, leaving 'a'
        assert_eq!(atob("YR").as_deref(), Ok("a"));
    }

    #[test]
    fn atob_rejects_invalid_input() {
        // A single leftover character can never form a byte
        assert!(atob("A").is_err());
        // '=' is only removed on a 4-boundary, so this hits the alphabet check
        assert!(atob("YQ=").is_err());
        // Over-padding
        assert!(atob("YQ===").is_err());
        // Padding in the middle
        assert!(atob("YQ==YQ==").is_err());
        // Characters outside the alphabet
        assert!(atob("$$$$").is_err());
        assert!(atob("YWJj\u{2603}").is_err());

        let err = atob("A").unwrap_err();
        assert_eq!(err.name(), "InvalidCharacterError");
    }

    #[test]
    fn roundtrip_binary_data() {
        let bytes: Vec<u8> = (0..=255).collect();
        let encoded = forgiving_base64_encode(&bytes);
        assert_eq!(forgiving_base64_decode(&encoded), Ok(bytes));
    }
}
