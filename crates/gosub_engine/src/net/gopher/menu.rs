//! Gopher menus (RFC 1436 §3.8): one item per line, tab-separated.
//!
//! `<type><display>\t<selector>\t<host>\t<port>`, ended by a line holding a single `.`.
//! Servers in the wild send LF instead of CRLF, leave out the terminator, add Gopher+ fields
//! after the port and fill info lines (`i`) with junk, so parsing is lenient: whatever
//! fields a line has are taken, and the rest are empty.

use super::request::DEFAULT_PORT;

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
}

/// Parse a menu body into its items.
pub fn parse(body: &[u8]) -> Vec<Item> {
    let mut items = Vec::new();
    for line in lines(body) {
        if line == b"." {
            break;
        }
        let Some((&t, rest)) = line.split_first() else {
            continue;
        };
        let mut fields = rest.split(|&b| b == b'\t');
        let display = String::from_utf8_lossy(fields.next().unwrap_or_default()).into_owned();
        let selector = fields.next().unwrap_or_default().to_vec();
        let host = String::from_utf8_lossy(fields.next().unwrap_or_default())
            .trim()
            .to_string();
        let port = match fields.next() {
            None => Some(DEFAULT_PORT),
            Some(p) => std::str::from_utf8(p).ok().and_then(|p| p.trim().parse().ok()),
        };
        items.push(Item {
            item_type: t as char,
            display,
            selector,
            host,
            port,
        });
    }
    items
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
    fn gopher_plus_fields_after_the_port_are_ignored() {
        let items = parse(b"1Plus\t/p\thost\t70\t+\r\n");
        assert_eq!(items[0].port, Some(70));
    }

    #[test]
    fn short_lines_and_bad_ports_do_not_fail_the_menu() {
        let items = parse(b"iJust text\r\n1Broken\t/b\thost\tseventy\r\n\r\n");
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].host, "");
        assert_eq!(items[1].port, None);
    }

    #[test]
    fn an_empty_body_has_no_items() {
        assert!(parse(b"").is_empty());
    }
}
