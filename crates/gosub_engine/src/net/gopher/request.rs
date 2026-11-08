//! A `gopher://` URL as the request it stands for (RFC 4266 §2).
//!
//! `gopher://<host>:<port>/<type><selector>`: the first character of the path is the item
//! type, the rest is the selector, percent-decoded. A search (type `7`) carries its terms
//! after a `%09` in the selector, or in the query, which is what the engine's own search form
//! submits. A second `%09` starts the Gopher+ string (RFC 4266 §2.3 - §2.8), which turns the
//! request into a Gopher+ one ([`Command`]).
//!
//! A `?` is not special to gopher, but a URL parser splits the query off at it. It goes back
//! where it was: onto the last field of the path, so a `?` after a Gopher+ string is part of
//! that string. Only a search URL without a `%09` reads its query as the search terms.
//!
//! URL parsing removes `.` and `..` path segments (and their `%2e` forms) before a URL gets
//! here, so a selector with dot segments only survives as `%2F..`, which is how
//! `render::gopher_url` writes it. A typed `gopher://h/1/a/../b` asks for `/b`.
//!
//! The selector goes onto the socket as raw bytes, so this is where a URL is kept from
//! speaking anything but gopher: CR, LF, TAB and NUL are refused. Without that a single URL
//! could put several lines, or a whole other protocol, on any port it can reach.

use percent_encoding::percent_decode_str;
use url::{Host, Url};

/// Gopher's port when the URL names none (RFC 4266 §2.1). The `url` crate only knows default
/// ports for special schemes.
pub const DEFAULT_PORT: u16 = 70;

/// Item type of a URL with an empty path: the server's root menu.
const ROOT_TYPE: char = '1';

/// Item type of a full-text search.
pub const SEARCH_TYPE: char = '7';

/// Name of the search form's field: its query arrives as `?q=<terms>`.
pub const SEARCH_FIELD: &str = "q";

/// One gopher request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// Host to connect to: a name, or an IP address without brackets.
    pub host: String,
    pub port: u16,
    pub item_type: char,
    pub selector: Vec<u8>,
    /// Search terms of a type `7` request; `None` before the user has entered any.
    pub search: Option<Vec<u8>>,
    /// The Gopher+ string after a second `%09` (RFC 4266 §2.1); `None` when it is empty.
    pub gopher_plus: Option<Vec<u8>>,
}

/// What a Gopher+ string asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command<'a> {
    /// `+`: the item, in its default view.
    Item,
    /// `+<view>`: an alternate view, `<content type>[ <language>]` (Gopher+ 2.6).
    View(&'a [u8]),
    /// `!` or `!<block names>`: the item's attributes (Gopher+ 2.5).
    Attributes(&'a [u8]),
    /// `$` or `$<block names>`: the attributes of every item in a directory (Gopher+ 2.7).
    Directory(&'a [u8]),
    /// `?`: the item's +ASK form (RFC 4266 §2.4), filled in and sent when it is submitted.
    Ask,
}

impl<'a> Command<'a> {
    fn of(plus: &'a [u8]) -> Option<Self> {
        match plus.split_first()? {
            (b'+', []) => Some(Command::Item),
            (b'+', view) => Some(Command::View(view)),
            (b'!', names) => Some(Command::Attributes(names)),
            (b'$', names) => Some(Command::Directory(names)),
            (b'?', []) => Some(Command::Ask),
            _ => None,
        }
    }
}

impl Request {
    /// The request `url` stands for, or why it stands for none.
    pub fn from_url(url: &Url) -> Result<Self, String> {
        if url.scheme() != "gopher" {
            return Err(format!("not a gopher URL: {url}"));
        }
        let host = match url.host() {
            Some(Host::Domain(d)) if !d.is_empty() => d.to_string(),
            Some(Host::Ipv4(ip)) => ip.to_string(),
            Some(Host::Ipv6(ip)) => ip.to_string(),
            _ => return Err(format!("gopher URL without a host: {url}")),
        };
        let port = url.port().unwrap_or(DEFAULT_PORT);

        let mut path: Vec<u8> = percent_decode_str(url.path().strip_prefix('/').unwrap_or(url.path())).collect();
        let mut query = url.query();
        if let Some(q) = query {
            let terms = path.first() == Some(&(SEARCH_TYPE as u8)) && !path.contains(&b'\t');
            if !terms {
                if path.is_empty() {
                    path.push(ROOT_TYPE as u8);
                }
                path.push(b'?');
                path.extend(percent_decode_str(q));
                query = None;
            }
        }
        let (item_type, mut selector) = match path.split_first() {
            None => (ROOT_TYPE, Vec::new()),
            Some((&t, rest)) if t.is_ascii_graphic() => (t as char, rest.to_vec()),
            Some(_) => return Err(format!("gopher URL with an invalid item type: {url}")),
        };

        // `%09` ends the selector and starts the search terms, a second `%09` ends those and
        // starts the Gopher+ string (RFC 4266 §2.1). Search terms left empty there are a
        // placeholder, not a search.
        let mut search = None;
        let mut gopher_plus = None;
        if let Some(tab) = selector.iter().position(|&b| b == b'\t') {
            let mut terms = selector.split_off(tab)[1..].to_vec();
            let second = terms.iter().position(|&b| b == b'\t');
            if let Some(tab) = second {
                gopher_plus = Some(terms.split_off(tab)[1..].to_vec()).filter(|plus| !plus.is_empty());
            }
            if !(terms.is_empty() && second.is_some()) {
                search = Some(terms);
            }
        }
        if let Some(query) = query {
            search = Some(search_terms(query));
        }

        check_line_safe(&selector, "selector")?;
        if let Some(terms) = &search {
            check_line_safe(terms, "search")?;
        }
        if let Some(plus) = &gopher_plus {
            check_line_safe(plus, "Gopher+ string")?;
            if Command::of(plus).is_none() {
                return Err(format!("gopher URL with an unknown Gopher+ string: {url}"));
            }
        }

        Ok(Self {
            host,
            port,
            item_type,
            selector,
            search,
            gopher_plus,
        })
    }

    /// A search that still needs its terms: answered with a form, not a connection.
    pub fn needs_search_terms(&self) -> bool {
        self.item_type == SEARCH_TYPE && self.search.is_none() && self.gopher_plus.is_none()
    }

    /// What the Gopher+ string asks for; `None` for a plain gopher request.
    pub fn command(&self) -> Option<Command<'_>> {
        Command::of(self.gopher_plus.as_deref()?)
    }

    /// The bytes sent to the server: the selector, the search terms after a tab, the Gopher+
    /// string after another, then CRLF. A form's URL (`?`) asks for the item's attributes,
    /// which hold the form.
    pub fn wire_line(&self) -> Vec<u8> {
        let mut line = self.fields();
        if let Some(plus) = &self.gopher_plus {
            line.push(b'\t');
            match self.command() {
                Some(Command::Ask) => line.push(b'!'),
                _ => line.extend_from_slice(plus),
            }
        }
        line.extend_from_slice(b"\r\n");
        line
    }

    /// The request that sends a form's answers (Gopher+ 2.4, 2.8; RFC 4266 §2.9): the item
    /// asked for with a data flag of `1`, then the answers as a period-terminated data block.
    /// Each answer is one line, already checked by the caller.
    pub fn answer_lines(&self, answers: &[Vec<u8>]) -> Vec<u8> {
        let mut out = self.fields();
        out.extend_from_slice(b"\t+\t1\r\n+-1\r\n");
        for answer in answers {
            out.extend_from_slice(answer);
            out.extend_from_slice(b"\r\n");
        }
        out.extend_from_slice(b".\r\n");
        out
    }

    /// The selector, and the search terms after a tab. A Gopher+ request to a search sends the
    /// tab with no terms after it (Gopher+ appendix II); any other item sends none (Gopher+
    /// 2.4), whatever RFC 4266 §2.3 says of URLs.
    fn fields(&self) -> Vec<u8> {
        let mut line = self.selector.clone();
        let empty = (self.gopher_plus.is_some() && self.item_type == SEARCH_TYPE).then_some(&[][..]);
        if let Some(terms) = self.search.as_deref().or(empty) {
            line.push(b'\t');
            line.extend_from_slice(terms);
        }
        line
    }
}

/// The search terms in a query: the search form's field when it is there, the whole query
/// otherwise (a URL written by hand, `gopher://host/7/find?terms`).
fn search_terms(query: &str) -> Vec<u8> {
    if let Some((_, value)) = url::form_urlencoded::parse(query.as_bytes()).find(|(k, _)| k == SEARCH_FIELD) {
        return value.into_owned().into_bytes();
    }
    percent_decode_str(query).collect()
}

/// Refuse a byte that would end the request line early or start another field.
pub fn check_line_safe(bytes: &[u8], what: &str) -> Result<(), String> {
    match bytes.iter().find(|&&b| matches!(b, b'\r' | b'\n' | b'\t' | 0)) {
        Some(b) => Err(format!("gopher {what} contains a forbidden byte 0x{b:02x}")),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(url: &str) -> Result<Request, String> {
        Request::from_url(&Url::parse(url).unwrap())
    }

    #[test]
    fn an_empty_path_is_the_root_menu_on_port_70() {
        let r = parse("gopher://gopher.example").unwrap();
        assert_eq!(r.host, "gopher.example");
        assert_eq!(r.port, 70);
        assert_eq!(r.item_type, '1');
        assert!(r.selector.is_empty());
        assert_eq!(r.wire_line(), b"\r\n");
    }

    #[test]
    fn the_first_path_character_is_the_item_type() {
        let r = parse("gopher://example:7070/0/docs/readme.txt").unwrap();
        assert_eq!(r.port, 7070);
        assert_eq!(r.item_type, '0');
        assert_eq!(r.selector, b"/docs/readme.txt");
    }

    #[test]
    fn the_selector_is_percent_decoded() {
        let r = parse("gopher://example/1/a%20b%25").unwrap();
        assert_eq!(r.selector, b"/a b%");
    }

    #[test]
    fn an_escaped_slash_keeps_a_dot_segment_in_the_selector() {
        assert_eq!(parse("gopher://example/1/a%2F../b").unwrap().selector, b"/a/../b");
        assert_eq!(parse("gopher://example/1/a/../b").unwrap().selector, b"/b");
    }

    #[test]
    fn an_ipv6_host_loses_its_brackets() {
        assert_eq!(parse("gopher://[::1]/").unwrap().host, "::1");
    }

    #[test]
    fn a_query_on_a_non_search_item_stays_in_the_selector() {
        let r = parse("gopher://example/0/cgi?x=1").unwrap();
        assert_eq!(r.selector, b"/cgi?x=1");
        assert_eq!(r.search, None);
    }

    #[test]
    fn a_search_takes_its_terms_from_a_tab() {
        let r = parse("gopher://example/7/find%09rust%20gopher").unwrap();
        assert_eq!(r.selector, b"/find");
        assert_eq!(r.search.as_deref(), Some(&b"rust gopher"[..]));
        assert_eq!(r.wire_line(), b"/find\trust gopher\r\n");
    }

    #[test]
    fn a_search_takes_its_terms_from_the_form_field() {
        let r = parse("gopher://example/7/find?q=rust+gopher").unwrap();
        assert_eq!(r.search.as_deref(), Some(&b"rust gopher"[..]));
    }

    #[test]
    fn a_search_takes_a_bare_query_whole() {
        let r = parse("gopher://example/7/find?rust%20gopher").unwrap();
        assert_eq!(r.search.as_deref(), Some(&b"rust gopher"[..]));
    }

    #[test]
    fn a_search_without_terms_needs_them() {
        assert!(parse("gopher://example/7/find").unwrap().needs_search_terms());
        assert!(!parse("gopher://example/7/find?q=x").unwrap().needs_search_terms());
        assert!(!parse("gopher://example/1/").unwrap().needs_search_terms());
    }

    #[test]
    fn line_breaks_in_the_selector_are_refused() {
        assert!(parse("gopher://example/1/a%0D%0AFLUSHALL").is_err());
        assert!(parse("gopher://example/1/a%0Ab").is_err());
        assert!(parse("gopher://example/1/a%00b").is_err());
    }

    #[test]
    fn line_breaks_in_the_search_are_refused() {
        assert!(parse("gopher://example/7/find?q=a%0D%0Ab").is_err());
        assert!(parse("gopher://example/7/find%09a%0Ab").is_err());
    }

    #[test]
    fn a_search_s_gopher_plus_string_follows_its_terms() {
        let r = parse("gopher://example/7sel%09terms%09+").unwrap();
        assert_eq!(r.selector, b"sel");
        assert_eq!(r.search.as_deref(), Some(&b"terms"[..]));
        assert_eq!(r.command(), Some(Command::Item));
        assert_eq!(r.wire_line(), b"sel\tterms\t+\r\n");
    }

    #[test]
    fn an_empty_search_before_a_gopher_plus_string_is_no_search() {
        let r = parse("gopher://example/1sel%09%09+").unwrap();
        assert_eq!(r.search, None);
        assert_eq!(r.gopher_plus.as_deref(), Some(&b"+"[..]));
        assert_eq!(r.wire_line(), b"sel\t+\r\n");
    }

    #[test]
    fn each_gopher_plus_string_has_its_wire_line() {
        for (url, line, command) in [
            ("gopher://e/0sel%09%09+", &b"sel\t+\r\n"[..], Command::Item),
            (
                "gopher://e/0sel%09%09+text/plain%20De_DE",
                b"sel\t+text/plain De_DE\r\n",
                Command::View(b"text/plain De_DE"),
            ),
            ("gopher://e/0sel%09%09!", b"sel\t!\r\n", Command::Attributes(b"")),
            (
                "gopher://e/0sel%09%09!+ABSTRACT%20+VIEWS",
                b"sel\t!+ABSTRACT +VIEWS\r\n",
                Command::Attributes(b"+ABSTRACT +VIEWS"),
            ),
            ("gopher://e/1dir%09%09$", b"dir\t$\r\n", Command::Directory(b"")),
            (
                "gopher://e/1dir%09%09$+VIEWS",
                b"dir\t$+VIEWS\r\n",
                Command::Directory(b"+VIEWS"),
            ),
            ("gopher://e/0form%09%09%3F", b"form\t!\r\n", Command::Ask),
            ("gopher://e/7find%09%09!", b"find\t\t!\r\n", Command::Attributes(b"")),
            ("gopher://e/7find%09%09+", b"find\t\t+\r\n", Command::Item),
        ] {
            let r = parse(url).unwrap();
            assert_eq!(r.command(), Some(command), "{url}");
            assert_eq!(r.wire_line(), line, "{url}");
        }
    }

    #[test]
    fn an_empty_gopher_plus_string_is_a_plain_request() {
        let r = parse("gopher://example/1sel%09%09").unwrap();
        assert_eq!((r.gopher_plus.as_deref(), r.command()), (None, None));
        assert_eq!(r.wire_line(), b"sel\r\n");
    }

    #[test]
    fn an_unknown_gopher_plus_string_is_refused() {
        for url in [
            "gopher://e/1s%09%09x",
            "gopher://e/1s%09%09?x",
            "gopher://e/1s%09%09%3F%3F",
        ] {
            assert!(parse(url).is_err(), "{url}");
        }
    }

    #[test]
    fn a_question_mark_after_a_gopher_plus_string_belongs_to_it() {
        for url in ["gopher://e/0s%09%09?", "gopher://e/0s%09%09%3F"] {
            let r = parse(url).unwrap();
            assert_eq!(r.selector, b"s", "{url}");
            assert_eq!(r.command(), Some(Command::Ask), "{url}");
        }
    }

    #[test]
    fn a_question_mark_goes_back_onto_the_last_field() {
        let r = parse("gopher://e/1a?b%09c?d").unwrap();
        assert_eq!(r.selector, b"a?b");
        assert_eq!(r.search.as_deref(), Some(&b"c?d"[..]));
        let r = parse("gopher://e/7find%09rust?x=1").unwrap();
        assert_eq!(r.search.as_deref(), Some(&b"rust?x=1"[..]));
        let r = parse("gopher://e/0a%3Fb?c").unwrap();
        assert_eq!(r.selector, b"a?b?c");
    }

    #[test]
    fn a_form_s_answers_follow_the_item_as_a_data_block() {
        let r = parse("gopher://e/0form%09%09?").unwrap();
        let line = r.answer_lines(&[b"42".to_vec(), b"Yes".to_vec()]);
        assert_eq!(line, b"form\t+\t1\r\n+-1\r\n42\r\nYes\r\n.\r\n");
        let r = parse("gopher://e/7find%09%09?").unwrap();
        assert_eq!(r.answer_lines(&[]), b"find\t\t+\t1\r\n+-1\r\n.\r\n");
    }

    #[test]
    fn line_breaks_in_the_gopher_plus_string_are_refused() {
        assert!(parse("gopher://example/1sel%09%09+%0D%0Ax").is_err());
        assert!(parse("gopher://example/1sel%09%09+%00").is_err());
        assert!(parse("gopher://example/1sel%09%09+%09x").is_err());
    }

    #[test]
    fn a_url_without_a_host_is_refused() {
        assert!(parse("gopher:///1/").is_err());
    }
}
