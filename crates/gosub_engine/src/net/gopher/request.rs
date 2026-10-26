//! A `gopher://` URL as the request it stands for (RFC 4266 §2).
//!
//! `gopher://<host>:<port>/<type><selector>`: the first character of the path is the item
//! type, the rest is the selector, percent-decoded. A search (type `7`) carries its terms
//! after a `%09` in the selector, or in the query, which is what the engine's own search form
//! submits.
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

        let path: Vec<u8> = percent_decode_str(url.path().strip_prefix('/').unwrap_or(url.path())).collect();
        let (item_type, mut selector) = match path.split_first() {
            None => (ROOT_TYPE, Vec::new()),
            Some((&t, rest)) if t.is_ascii_graphic() => (t as char, rest.to_vec()),
            Some(_) => return Err(format!("gopher URL with an invalid item type: {url}")),
        };

        // `%09` ends the selector and starts the search terms (RFC 4266 §2.1). A second tab
        // would begin a Gopher+ string, which is not spoken here.
        let mut search = None;
        if let Some(tab) = selector.iter().position(|&b| b == b'\t') {
            search = Some(selector.split_off(tab)[1..].to_vec());
        }

        if let Some(query) = url.query() {
            if item_type == SEARCH_TYPE {
                if search.is_none() {
                    search = Some(search_terms(query));
                }
            } else {
                // A `?` is not special to gopher; `Url` split it off a selector that had one.
                selector.push(b'?');
                selector.extend(percent_decode_str(query));
            }
        }

        check_line_safe(&selector, "selector")?;
        if let Some(terms) = &search {
            check_line_safe(terms, "search")?;
        }

        Ok(Self {
            host,
            port,
            item_type,
            selector,
            search,
        })
    }

    /// A search that still needs its terms: answered with a form, not a connection.
    pub fn needs_search_terms(&self) -> bool {
        self.item_type == SEARCH_TYPE && self.search.is_none()
    }

    /// The bytes sent to the server: the selector, the search terms after a tab, then CRLF.
    pub fn wire_line(&self) -> Vec<u8> {
        let mut line = self.selector.clone();
        if let Some(terms) = &self.search {
            line.push(b'\t');
            line.extend_from_slice(terms);
        }
        line.extend_from_slice(b"\r\n");
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
fn check_line_safe(bytes: &[u8], what: &str) -> Result<(), String> {
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
        assert!(parse("gopher://example/7/find%09a%09b").is_err());
    }

    #[test]
    fn a_url_without_a_host_is_refused() {
        assert!(parse("gopher:///1/").is_err());
    }
}
