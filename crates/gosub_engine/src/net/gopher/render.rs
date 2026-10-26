//! Gopher responses as HTML pages.
//!
//! Menus, text files and search prompts become ordinary HTML, so the rest of the engine
//! (parsing, layout, history, links) treats a gopher page like any other. Everything that
//! came from the server is escaped: a display string is text, never markup.

use super::menu::{self, Item};
use super::request::{DEFAULT_PORT, SEARCH_FIELD, SEARCH_TYPE};
use percent_encoding::{percent_encode, AsciiSet, CONTROLS};
use url::Url;

/// Bytes escaped in a selector placed in a URL path: everything a URL parser would read as
/// structure or change, plus `%` itself so a decoded selector round-trips.
const SELECTOR: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'[')
    .add(b'\\')
    .add(b']')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}');

const STYLE: &str = "body{margin:0;padding:16px 24px;font-family:monospace;line-height:1.4}\
pre{margin:0;white-space:pre-wrap}\
.t{color:#777}\
.e{color:#b00020}\
h1{font-size:1em;font-weight:bold;margin:0 0 12px}\
form{margin:12px 0}";

/// A menu page.
pub fn menu_page(url: &Url, body: &[u8]) -> String {
    let host = url.host_str().unwrap_or_default();
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let port = url.port().unwrap_or(DEFAULT_PORT);
    let mut out = String::new();
    for item in menu::parse(body) {
        out.push_str(&menu_line(&item, host, port));
        // `<br>`, not a newline: the engine loses a preserved newline between inline
        // elements, and a browser renders either as a line break.
        out.push_str("<br>");
    }
    page(&title_for(url), &format!("<pre>{out}</pre>"))
}

/// A text file (item type `0`), with the gopher terminator and dot-stuffing undone.
pub fn text_page(url: &Url, body: &[u8]) -> String {
    let mut text = String::new();
    for line in menu::lines(body) {
        if line == b"." {
            break;
        }
        let line = line.strip_prefix(b".").filter(|l| l.starts_with(b".")).unwrap_or(line);
        text.push_str(&String::from_utf8_lossy(line));
        text.push('\n');
    }
    page(&title_for(url), &format!("<pre>{}</pre>", escape(&text)))
}

/// The prompt for a search (item type `7`) the user has not entered terms for yet. Submits
/// to the same URL, the terms in the query (see `request::search_terms`).
pub fn search_page(url: &Url) -> String {
    let mut action = url.clone();
    action.set_query(None);
    action.set_fragment(None);
    let body = format!(
        "<h1>Search {title}</h1>\
<form method=\"get\" action=\"{action}\">\
<input type=\"search\" name=\"{SEARCH_FIELD}\" size=\"40\" autofocus> \
<button type=\"submit\">Search</button>\
</form>",
        title = escape(&title_for(url)),
        action = escape(action.as_str()),
    );
    page(&title_for(url), &body)
}

/// A page for a gopher error (item type `3` answered on its own).
pub fn error_page(url: &Url, message: &str) -> String {
    page(
        &title_for(url),
        &format!("<pre class=\"e\">{}</pre>", escape(message.trim_end())),
    )
}

/// One menu item as a line of the menu page: a type tag, then the display string, linked
/// when the item is something the engine can go to.
fn menu_line(item: &Item, host: &str, port: u16) -> String {
    let display = escape(&item.display);
    // The padding sits outside the span: the engine drops preserved spaces at the end of an
    // inline element.
    let label = tag_for(item.item_type);
    let tag = format!("<span class=\"t\">{label}</span>{:pad$}", "", pad = 5 - label.len());
    match item.item_type {
        'i' => format!("{:<5} {display}", ""),
        '3' => format!("{tag} <span class=\"e\">{display}</span>"),
        _ => match link_for(item, host, port) {
            Some(href) => format!("{tag} <a href=\"{}\">{display}</a>", escape(&href)),
            None => format!("{tag} {display}"),
        },
    }
}

/// Where a menu item leads, or `None` when it leads nowhere the engine goes: telnet, CSO,
/// mirrors, a malformed host or port.
fn link_for(item: &Item, host: &str, port: u16) -> Option<String> {
    match item.item_type {
        '2' | '8' | 'T' | '+' => None,
        // `h` with `URL:<url>` is a link off gopherspace (a common extension).
        'h' if item.selector.starts_with(b"URL:") => {
            let target = Url::parse(std::str::from_utf8(&item.selector[4..]).ok()?).ok()?;
            matches!(target.scheme(), "http" | "https" | "gopher").then(|| target.to_string())
        }
        t => {
            // An item without a host is on the server that sent the menu.
            let (host, port) = if item.host.is_empty() {
                (host, Some(port))
            } else {
                (item.host.as_str(), item.port)
            };
            gopher_url(host, port?, t, &item.selector).map(|u| u.to_string())
        }
    }
}

/// The URL of an item on `host:port`, or `None` when the host is not a plain host: a menu
/// line's host field must not carry URL syntax that sends the link somewhere it does not
/// name.
pub fn gopher_url(host: &str, port: u16, item_type: char, selector: &[u8]) -> Option<Url> {
    if host.is_empty() || !item_type.is_ascii_graphic() {
        return None;
    }
    let authority = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    let port_part = if port == DEFAULT_PORT {
        String::new()
    } else {
        format!(":{port}")
    };
    let mut path = vec![item_type as u8];
    path.extend_from_slice(selector);
    let url = Url::parse(&format!(
        "gopher://{authority}{port_part}/{}",
        percent_encode(&path, SELECTOR)
    ))
    .ok()?;
    let parsed_host = url.host_str()?.trim_start_matches('[').trim_end_matches(']');
    parsed_host.eq_ignore_ascii_case(host).then_some(url)
}

/// The fixed-width tag in front of a menu item.
fn tag_for(item_type: char) -> &'static str {
    match item_type {
        '0' => "TXT",
        '1' => "DIR",
        SEARCH_TYPE => "FIND",
        'h' => "HTML",
        'g' | 'I' | 'p' => "IMG",
        '4' | '5' | '6' | '9' | 'd' | 's' | ';' => "BIN",
        '3' => "ERR",
        '8' | 'T' => "TEL",
        '2' => "CSO",
        _ => "?",
    }
}

/// A page's title: where it is, without the scheme.
fn title_for(url: &Url) -> String {
    let s = url.as_str();
    s.strip_prefix("gopher://").unwrap_or(s).to_string()
}

fn page(title: &str, body: &str) -> String {
    format!(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>{}</title><style>{STYLE}</style></head>\
<body>{body}</body></html>",
        escape(title)
    )
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::gopher::request::Request;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn menu_items_become_links() {
        let html = menu_page(
            &url("gopher://home.example/"),
            b"1Docs\t/docs\tgopher.example\t70\r\n0Read me\t/a b.txt\tother.example\t7070\r\n.\r\n",
        );
        assert!(html.contains("<a href=\"gopher://gopher.example/1/docs\">Docs</a>"));
        assert!(html.contains("<a href=\"gopher://other.example:7070/0/a%20b.txt\">Read me</a>"));
    }

    #[test]
    fn info_and_error_lines_are_text() {
        let html = menu_page(&url("gopher://h/"), b"iHello\t\t\t\r\n3Gone\t\t\t\r\n");
        assert!(html.contains("      Hello"));
        assert!(html.contains("<span class=\"e\">Gone</span>"));
        assert!(!html.contains("<a "));
    }

    #[test]
    fn each_menu_item_is_a_line() {
        let html = menu_page(&url("gopher://h/"), b"iOne\t\t\t\r\niTwo\t\t\t\r\n");
        assert!(html.contains("One<br>"));
        assert!(html.contains("Two<br>"));
    }

    #[test]
    fn a_hostile_display_string_is_text() {
        let html = menu_page(
            &url("gopher://h/"),
            b"1<script>alert(1)</script>\t/x\th\t70\r\ni<img src=x onerror=y>\t\t\t\r\n",
        );
        assert!(!html.contains("<script>"));
        assert!(!html.contains("<img"));
        assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
    }

    #[test]
    fn a_hostile_selector_cannot_break_out_of_the_link() {
        let html = menu_page(&url("gopher://h/"), b"1X\t/\"><script>\th\t70\r\n");
        assert!(!html.contains("<script>"));
        assert!(html.contains("gopher://h/1/%22%3E%3Cscript%3E"));
    }

    #[test]
    fn a_host_field_with_url_syntax_is_not_linked() {
        for host in ["evil.example/x", "a@b", "h#frag", "h:99", "two words"] {
            let line = format!("1X\t/s\t{host}\t70\r\n");
            let html = menu_page(&url("gopher://h/"), line.as_bytes());
            assert!(!html.contains("<a "), "{host} was linked: {html}");
        }
    }

    #[test]
    fn an_item_without_a_host_is_on_the_menu_s_server() {
        let html = menu_page(&url("gopher://h:7070/"), b"0Local\t/l.txt\t\t\r\n");
        assert!(html.contains("<a href=\"gopher://h:7070/0/l.txt\">"));
    }

    #[test]
    fn url_items_link_off_gopherspace_but_only_to_the_web() {
        let html = menu_page(
            &url("gopher://h/"),
            b"hWeb\tURL:https://example.org/\th\t70\r\nhLocal\tURL:file:///etc/passwd\th\t70\r\n",
        );
        assert!(html.contains("<a href=\"https://example.org/\">Web</a>"));
        assert!(!html.contains("file:"));
    }

    #[test]
    fn telnet_items_are_not_linked() {
        let html = menu_page(&url("gopher://h/"), b"8Login\t\th\t23\r\n");
        assert!(!html.contains("<a "));
    }

    #[test]
    fn a_selector_round_trips_through_its_url() {
        let selector = b"/odd ?#%[]\xff/path";
        let u = gopher_url("h", 70, '0', selector).unwrap();
        let r = Request::from_url(&u).unwrap();
        assert_eq!(r.item_type, '0');
        assert_eq!(r.selector, selector);
    }

    #[test]
    fn an_ipv6_host_is_bracketed() {
        assert_eq!(gopher_url("::1", 70, '1', b"/").unwrap().as_str(), "gopher://[::1]/1/");
    }

    #[test]
    fn text_is_escaped_and_unstuffed() {
        let html = text_page(&url("gopher://h/0/x"), b"<b>bold</b>\r\n..dot\r\n.\r\nafter\r\n");
        assert!(html.contains("&lt;b&gt;bold&lt;/b&gt;\n.dot\n</pre>"));
        assert!(!html.contains("after"));
    }

    #[test]
    fn the_search_form_submits_to_the_search_url() {
        let html = search_page(&url("gopher://h/7/find"));
        assert!(html.contains("action=\"gopher://h/7/find\""));
        assert!(html.contains("name=\"q\""));
    }
}
