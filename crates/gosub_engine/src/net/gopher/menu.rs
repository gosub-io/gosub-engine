//! Gopher menus (RFC 1436 §3.8): one item per line, tab-separated.
//!
//! `<type><display>\t<selector>\t<host>\t<port>`, ended by a line holding a single `.`.
//! Servers in the wild send LF instead of CRLF, leave out the terminator and fill info lines
//! (`i`) with junk, so parsing is lenient: whatever fields a line has are taken, and the rest
//! are empty. A Gopher+ server adds a fifth field (Gopher+ 2.2, 2.8): `+` for a Gopher+ item,
//! `?` for one with an +ASK form.

use super::request::DEFAULT_PORT;
use encoding_rs::Encoding;
use std::borrow::Cow;

/// One menu line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub item_type: char,
    pub display: String,
    /// Raw bytes: a selector is the server's to interpret and need not be text.
    pub selector: Vec<u8>,
    pub host: String,
    /// `None` when the line's port is not a number.
    pub port: Option<u16>,
    pub plus: Plus,
}

/// What the field after the port says of an item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plus {
    /// No field, or one that is neither `+` nor `?`: a plain gopher item.
    No,
    /// `+`: a Gopher+ item, with attributes to ask for.
    Item,
    /// `?`: a Gopher+ item with an +ASK form.
    Ask,
}

/// Parse a menu body into its items.
pub fn parse(body: &[u8]) -> Vec<Item> {
    let encoding = encoding_for(body);
    lines(body)
        .take_while(|&line| line != b".")
        .filter_map(|line| parse_line(encoding, line))
        .collect()
}

/// One menu line, its text in `encoding`. `None` for an empty line.
pub fn parse_line(encoding: &'static Encoding, line: &[u8]) -> Option<Item> {
    let (&t, rest) = line.split_first()?;
    let mut fields = rest.split(|&b| b == b'\t');
    let display = decode(encoding, fields.next().unwrap_or_default()).into_owned();
    let selector = fields.next().unwrap_or_default().to_vec();
    let host = decode(encoding, fields.next().unwrap_or_default()).trim().to_string();
    let port = match fields.next() {
        None => Some(DEFAULT_PORT),
        Some(p) => std::str::from_utf8(p).ok().and_then(|p| p.trim().parse().ok()),
    };
    let plus = match fields.next().map(<[u8]>::trim_ascii) {
        Some(b"+") => Plus::Item,
        Some(b"?") => Plus::Ask,
        _ => Plus::No,
    };
    Some(Item {
        item_type: t as char,
        display,
        selector,
        host,
        port,
        plus,
    })
}

/// The encoding a body's text is in. Gopher has no charset field: UTF-8 when the whole body
/// is valid UTF-8, windows-1252 otherwise, which covers Latin-1 and maps every byte.
pub fn encoding_for(body: &[u8]) -> &'static Encoding {
    if std::str::from_utf8(body).is_ok() {
        encoding_rs::UTF_8
    } else {
        encoding_rs::WINDOWS_1252
    }
}

/// Part of a body as text, in the body's encoding.
pub fn decode<'a>(encoding: &'static Encoding, bytes: &'a [u8]) -> Cow<'a, str> {
    encoding.decode_without_bom_handling(bytes).0
}

/// The lines of a body, LF or CRLF terminated.
pub fn lines(body: &[u8]) -> impl Iterator<Item = &[u8]> {
    let body = body.strip_suffix(b"\n").unwrap_or(body);
    body.split(|&b| b == b'\n')
        .map(|line| line.strip_suffix(b"\r").unwrap_or(line))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_menu_parses_into_items() {
        let body = b"1Docs\t/docs\tgopher.example\t70\r\n\
                     0About\t/about.txt\tother.example\t7070\r\n\
                     iWelcome\t\terror.host\t1\r\n\
                     .\r\n";
        let items = parse(body);
        assert_eq!(items.len(), 3);
        assert_eq!(
            items[0],
            Item {
                item_type: '1',
                display: "Docs".into(),
                selector: b"/docs".to_vec(),
                host: "gopher.example".into(),
                port: Some(70),
                plus: Plus::No,
            }
        );
        assert_eq!(items[1].port, Some(7070));
        assert_eq!(items[2].item_type, 'i');
        assert_eq!(items[2].display, "Welcome");
    }

    #[test]
    fn nothing_after_the_terminator_is_read() {
        let items = parse(b"iBefore\r\n.\r\niAfter\r\n");
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn bare_lf_and_a_missing_terminator_are_accepted() {
        let items = parse(b"1One\t/1\thost\t70\n1Two\t/2\thost\t70\n");
        assert_eq!(items.len(), 2);
        assert_eq!(items[1].selector, b"/2");
    }

    #[test]
    fn the_field_after_the_port_marks_a_gopher_plus_item() {
        let items = parse(b"1Plus\t/p\thost\t70\t+\r\n0Form\t/f\thost\t70\t?\r\n0Old\t/o\thost\t70\r\n");
        assert_eq!((items[0].port, items[0].plus), (Some(70), Plus::Item));
        assert_eq!(items[1].plus, Plus::Ask);
        assert_eq!(items[2].plus, Plus::No);
    }

    #[test]
    fn a_field_after_the_port_that_is_not_plus_or_ask_is_no_gopher_plus() {
        for line in [
            &b"1X\t/x\th\t70\t\r\n"[..],
            b"1X\t/x\th\t70\t++\r\n",
            b"1X\t/x\th\t70\tjunk\r\n",
            b"1X\t/x\th\t70\t!\r\n",
        ] {
            assert_eq!(parse(line)[0].plus, Plus::No, "{:?}", String::from_utf8_lossy(line));
        }
        assert_eq!(parse(b"1X\t/x\th\t70\t + \t\r\n")[0].plus, Plus::Item);
    }

    #[test]
    fn short_lines_and_bad_ports_do_not_fail_the_menu() {
        let items = parse(b"iJust text\r\n1Broken\t/b\thost\tseventy\r\n\r\n");
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].host, "");
        assert_eq!(items[1].port, None);
    }

    #[test]
    fn a_latin1_menu_is_read_as_windows_1252() {
        let items = parse(b"0Caf\xe9 \x80\t/x\th\xe9\t70\r\n");
        assert_eq!(items[0].display, "Caf\u{e9} \u{20ac}");
        assert_eq!(items[0].host, "h\u{e9}");
        assert_eq!(items[0].selector, b"/x");
    }

    #[test]
    fn a_utf8_menu_is_read_as_utf8() {
        let items = parse("0Caf\u{e9} \u{20ac}\t/x\th\t70\r\n".as_bytes());
        assert_eq!(items[0].display, "Caf\u{e9} \u{20ac}");
    }

    #[test]
    fn the_encoding_is_decided_once_for_the_whole_body() {
        // The first line is valid UTF-8 on its own; the second is not, so neither is.
        let items = parse(b"i\xc3\xa9\t\t\t\r\ni\xe9\t\t\t\r\n");
        assert_eq!(items[0].display, "\u{c3}\u{a9}");
        assert_eq!(items[1].display, "\u{e9}");
    }

    #[test]
    fn an_empty_body_has_no_items() {
        assert!(parse(b"").is_empty());
    }
}
