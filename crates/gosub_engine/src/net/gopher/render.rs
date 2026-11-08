//! Gopher responses as HTML pages.
//!
//! Menus, text files and search prompts become ordinary HTML, so the rest of the engine
//! (parsing, layout, history, links) treats a gopher page like any other. Everything that
//! came from the server is escaped: a display string is text, never markup.

use super::menu::{self, Item, Plus};
use super::mirrors;
use super::request::{DEFAULT_PORT, SEARCH_FIELD, SEARCH_TYPE};
use crate::net::protocols::page::{self, escape, title_for};
use encoding_rs::Encoding;
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

pub(super) const STYLE: &str = "body{margin:0;padding:16px 24px;font-family:monospace;line-height:1.4}\
pre{margin:0;white-space:pre-wrap}\
.t{color:#777}\
.e{color:#b00020}\
h1{font-size:1em;font-weight:bold;margin:0 0 12px}\
form{margin:12px 0}";

/// A menu page.
#[cfg(test)]
pub fn menu_page(url: &Url, body: &[u8]) -> String {
    menu_page_in(url, body, None)
}

/// A menu page, the mirrors it names remembered in `zone` when there is one.
pub fn menu_page_in(url: &Url, body: &[u8], zone: Option<&str>) -> String {
    let lines = Lines::menu(url, menu::encoding_for(body));
    let lines = match zone {
        Some(zone) => lines.remembering_mirrors(zone, url),
        None => lines,
    };
    whole(lines, url, body)
}

/// A text file (item type `0`), with the gopher terminator and dot-stuffing undone.
pub fn text_page(url: &Url, body: &[u8]) -> String {
    whole(Lines::text(menu::encoding_for(body)), url, body)
}

/// The page `lines` makes of a whole body.
fn whole(mut lines: Lines, url: &Url, body: &[u8]) -> String {
    let mut out = page_head(url);
    for line in menu::lines(body) {
        match lines.line(line) {
            Some(html) => out.push_str(&html),
            None => break,
        }
    }
    out.push_str(lines.end());
    out.push_str(&page_end());
    out
}

/// The start of a menu or text page, up to its first line.
pub fn page_head(url: &Url) -> String {
    page::document_head(&title_for(url), STYLE) + "<pre>"
}

/// What closes a page [`page_head`] began.
pub fn page_end() -> String {
    format!("</pre>{}", page::DOCUMENT_END)
}

/// A menu or a text file turned into its page a line at a time, so the page can be served as
/// its lines arrive. The text is read in the encoding given, and as windows-1252 from the
/// first line that is not valid UTF-8 on.
pub struct Lines {
    menu: Option<(String, u16)>,
    encoding: &'static Encoding,
    ended: bool,
    /// The type and URL of the item a `+` line now would be a mirror of.
    mirrored: Option<(char, Url)>,
    /// The last line's break is held back, for its item's mirrors to join the line.
    open: bool,
    /// Where the mirrors are remembered for a navigation from this menu ([`mirrors`]): the
    /// zone and the menu's URL.
    remember: Option<(String, Url)>,
}

impl Lines {
    /// The lines of the menu at `url`.
    pub fn menu(url: &Url, encoding: &'static Encoding) -> Self {
        let host = url.host_str().unwrap_or_default();
        let host = host.trim_start_matches('[').trim_end_matches(']').to_string();
        let port = url.port().unwrap_or(DEFAULT_PORT);
        Lines {
            menu: Some((host, port)),
            encoding,
            ended: false,
            mirrored: None,
            open: false,
            remember: None,
        }
    }

    /// These lines, remembering the mirrors the menu at `url` names in `zone`.
    pub fn remembering_mirrors(mut self, zone: &str, url: &Url) -> Self {
        self.remember = Some((zone.to_string(), url.clone()));
        self
    }

    /// The lines of a text file.
    pub fn text(encoding: &'static Encoding) -> Self {
        Lines {
            menu: None,
            encoding,
            ended: false,
            mirrored: None,
            open: false,
            remember: None,
        }
    }

    /// One line, without its line break, as page markup. `None` from the line holding a
    /// single `.` on: it ends the body.
    pub fn line(&mut self, line: &[u8]) -> Option<String> {
        if self.ended || line == b"." {
            self.ended = true;
            return None;
        }
        if self.encoding == encoding_rs::UTF_8 && std::str::from_utf8(line).is_err() {
            self.encoding = encoding_rs::WINDOWS_1252;
        }
        let Some((host, port)) = &self.menu else {
            let line = line.strip_prefix(b".").filter(|l| l.starts_with(b".")).unwrap_or(line);
            return Some(escape(&menu::decode(self.encoding, line)) + "\n");
        };
        let Some(item) = menu::parse_line(self.encoding, line) else {
            self.mirrored = None;
            return Some(String::new());
        };
        if item.item_type == '+' {
            if let Some((item_type, of)) = &self.mirrored {
                let Some(mirror) = mirror_url(&item, *item_type) else {
                    return Some(String::new());
                };
                if let Some((zone, page)) = &self.remember {
                    mirrors::remember(zone, page, of, &mirror);
                }
                return Some(mirror_link(&mirror));
            }
        }
        // `<br>`, not a newline: the engine loses a preserved newline between inline
        // elements, and a browser renders either as a line break.
        let markup = menu_line(&item, host, *port, self.encoding);
        let mirrored = mirrored(&item, host, *port).map(|url| (item.item_type, url));
        let mut out = self.end().to_string();
        out.push_str(&markup);
        self.mirrored = mirrored;
        self.open = self.mirrored.is_some();
        if !self.open {
            out.push_str("<br>");
        }
        Some(out)
    }

    /// What closes the last line, once no line follows.
    pub fn end(&mut self) -> &'static str {
        if std::mem::take(&mut self.open) {
            "<br>"
        } else {
            ""
        }
    }
}

/// The URL of an item a `+` line after it can be a mirror of: one on a gopher server.
fn mirrored(item: &Item, host: &str, port: u16) -> Option<Url> {
    if matches!(item.item_type, 'i' | '3') {
        return None;
    }
    let url = Url::parse(&link_for(item, host, port)?).ok()?;
    (url.scheme() == "gopher").then_some(url)
}

/// Where a `+` line says the item of `item_type` before it is served too: its own selector on
/// its own server. A redundant server is another server, so a line without a host names none.
fn mirror_url(line: &Item, item_type: char) -> Option<Url> {
    if line.host.is_empty() {
        return None;
    }
    gopher_url(&line.host, line.port?, item_type, &line.selector)
}

/// A mirror on the line of the item it serves.
fn mirror_link(mirror: &Url) -> String {
    let host = mirror.host_str().unwrap_or_default();
    let server = format!("{host}:{}", mirror.port().unwrap_or(DEFAULT_PORT));
    format!(
        " <span class=\"t\">(mirror:</span> <a href=\"{}\">{}</a><span class=\"t\">)</span>",
        escape(mirror.as_str()),
        escape(&server)
    )
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
fn menu_line(item: &Item, host: &str, port: u16, encoding: &'static Encoding) -> String {
    let display = escape(&item.display);
    // The padding sits outside the span: the engine drops preserved spaces at the end of an
    // inline element.
    let label = tag_for(item.item_type);
    let tag = format!("<span class=\"t\">{label}</span>{:pad$}", "", pad = 5 - label.len());
    match item.item_type {
        'i' => format!("{:<5} {display}", ""),
        '3' => format!("{tag} <span class=\"e\">{display}</span>"),
        // CSO phone books are left out on purpose.
        '2' => format!("{tag} {display} <span class=\"t\">(not supported)</span>"),
        '8' | 'T' => match session_for(item, host, port, encoding) {
            Some(session) => format!("{tag} {display} <span class=\"t\">{}</span>", escape(&session)),
            None => format!("{tag} {display}"),
        },
        _ => match link_for(item, host, port) {
            Some(href) => format!(
                "{tag} <a href=\"{}\">{display}</a>{}",
                escape(&href),
                plus_links(item, host, port)
            ),
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

/// Links to what Gopher+ adds to an item, after its own link: its attributes, and its form
/// for an item that has one. The item's own link stays a plain gopher request, which every
/// server answers, though a server may answer an item with a form only with an error.
fn plus_links(item: &Item, host: &str, port: u16) -> String {
    let link = |plus: &[u8], label: &str| match plus_url(item, host, port, plus) {
        Some(url) => format!(" <a href=\"{}\">{label}</a>", escape(url.as_str())),
        None => String::new(),
    };
    match item.plus {
        Plus::No => String::new(),
        Plus::Item => link(b"!", "[info]"),
        Plus::Ask => link(b"?", "[form]") + &link(b"!", "[info]"),
    }
}

/// The URL of `item` with the Gopher+ string `plus`, or `None` for an item that has no gopher
/// URL: telnet, CSO, mirrors, info and error lines, links off gopherspace.
pub fn plus_url(item: &Item, host: &str, port: u16, plus: &[u8]) -> Option<Url> {
    match item.item_type {
        '2' | '8' | 'T' | '+' | 'i' | '3' => None,
        'h' if item.selector.starts_with(b"URL:") => None,
        t => {
            let (host, port) = if item.host.is_empty() {
                (host, Some(port))
            } else {
                (item.host.as_str(), item.port)
            };
            gopher_plus_url(host, port?, t, &item.selector, plus)
        }
    }
}

/// What a telnet (`8`) or tn3270 (`T`) item opens, as text: the engine opens no sessions.
/// `None` when the host is not one `gopher_url` would link to. The selector, when set, is the
/// login name.
fn session_for(item: &Item, host: &str, port: u16, encoding: &'static Encoding) -> Option<String> {
    let (host, port) = if item.host.is_empty() {
        (host, port)
    } else {
        (item.host.as_str(), item.port?)
    };
    gopher_url(host, port, item.item_type, b"")?;
    let protocol = if item.item_type == 'T' { "tn3270" } else { "telnet" };
    let host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    let login = menu::decode(encoding, &item.selector);
    Some(if login.is_empty() {
        format!("{protocol} {host}:{port}")
    } else {
        format!("{protocol} {host}:{port} (login: {login})")
    })
}

/// The URL of an item on `host:port`, or `None` when the host is not a plain host: a menu
/// line's host field must not carry URL syntax that sends the link somewhere it does not
/// name.
pub fn gopher_url(host: &str, port: u16, item_type: char, selector: &[u8]) -> Option<Url> {
    gopher_plus_url(host, port, item_type, selector, b"")
}

/// [`gopher_url`] with a Gopher+ string after two `%09`s (RFC 4266 §2.3), the search terms
/// between them left empty. An empty `plus` is none.
pub fn gopher_plus_url(host: &str, port: u16, item_type: char, selector: &[u8], plus: &[u8]) -> Option<Url> {
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
    if !plus.is_empty() {
        path.extend_from_slice(b"\t\t");
        path.extend_from_slice(plus);
    }
    let url = Url::parse(&format!(
        "gopher://{authority}{port_part}/{}",
        keep_dot_segments(&percent_encode(&path, SELECTOR).to_string())?
    ))
    .ok()?;
    let parsed_host = url.host_str()?.trim_start_matches('[').trim_end_matches(']');
    parsed_host.eq_ignore_ascii_case(host).then_some(url)
}

/// `path` with every `.` or `..` segment joined to its neighbour by a `%2F`. A URL parser
/// removes dot segments, gopher-path or not, but a selector is opaque and must reach the
/// server as it is. `%2F` decodes back to the same `/`. `None` for a path that is nothing but
/// a dot segment.
fn keep_dot_segments(path: &str) -> Option<String> {
    let is_dot = |s: &str| s == "." || s == "..";
    let segments: Vec<&str> = path.split('/').collect();
    if segments.len() == 1 && is_dot(path) {
        return None;
    }
    let mut out = String::with_capacity(path.len());
    for (i, segment) in segments.iter().enumerate() {
        if i > 0 {
            let glue = is_dot(segment) || (i == 1 && is_dot(segments[0]));
            out.push_str(if glue { "%2F" } else { "/" });
        }
        out.push_str(segment);
    }
    Some(out)
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
        // Common extensions outside RFC 1436.
        'P' => "PDF",
        'r' => "RTF",
        'x' => "XML",
        'c' => "CAL",
        'M' => "MIME",
        ':' => "IMG",
        '<' => "SND",
        '+' => "MIR",
        _ => "?",
    }
}

fn page(title: &str, body: &str) -> String {
    page::document(title, STYLE, body)
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
        let html = menu_page(
            &url("gopher://h/"),
            b"8Login\tguest\tbbs.example\t23\r\nTMainframe\t\tibm.example\t2323\r\n8Here\t\t\t\r\n",
        );
        assert!(!html.contains("<a "));
        assert!(html.contains("Login <span class=\"t\">telnet bbs.example:23 (login: guest)</span>"));
        assert!(html.contains("Mainframe <span class=\"t\">tn3270 ibm.example:2323</span>"));
        assert!(html.contains("Here <span class=\"t\">telnet h:70</span>"));
    }

    #[test]
    fn a_telnet_item_with_a_bad_host_or_port_shows_no_target() {
        for line in [
            &b"8X\t\tevil.example/x\t23\r\n"[..],
            b"8X\t\ta@b\t23\r\n",
            b"8X\t\th\ttwenty\r\n",
        ] {
            let html = menu_page(&url("gopher://h/"), line);
            assert!(!html.contains("telnet"), "{html}");
            assert!(html.contains("TEL</span>   X"), "{html}");
        }
    }

    #[test]
    fn a_cso_item_is_marked_unsupported_and_not_linked() {
        let html = menu_page(&url("gopher://h/"), b"2Phone book\t\tns.example\t105\r\n");
        assert!(!html.contains("<a "));
        assert!(html.contains("CSO</span>   Phone book <span class=\"t\">(not supported)</span>"));
    }

    #[test]
    fn a_telnet_login_is_escaped() {
        let html = menu_page(&url("gopher://h/"), b"8X\t<b>me</b>\th\t23\r\n8Y\t\t::1\t23\r\n");
        assert!(!html.contains("<b>"));
        assert!(html.contains("(login: &lt;b&gt;me&lt;/b&gt;)"));
        assert!(html.contains("telnet [::1]:23"));
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
    fn extension_types_are_tagged_and_linked() {
        let body = b"PPaper\t/p.pdf\th\t70\r\nrNotes\t/n.rtf\th\t70\r\nxFeed\t/f.xml\th\t70\r\n\
                     cDates\t/d.ics\th\t70\r\nMMail\t/m.eml\th\t70\r\n:Pic\t/b.bmp\th\t70\r\n\
                     <Tune\t/t.wav\th\t70\r\n";
        let html = menu_page(&url("gopher://h/"), body);
        for (tag, href) in [
            ("PDF", "gopher://h/P/p.pdf"),
            ("RTF", "gopher://h/r/n.rtf"),
            ("XML", "gopher://h/x/f.xml"),
            ("CAL", "gopher://h/c/d.ics"),
            ("MIME", "gopher://h/M/m.eml"),
            ("IMG", "gopher://h/:/b.bmp"),
            ("SND", "gopher://h/%3C/t.wav"),
        ] {
            assert!(
                html.contains(&format!("<span class=\"t\">{tag}</span>")),
                "{tag}: {html}"
            );
            assert!(html.contains(&format!("<a href=\"{href}\">")), "{href}: {html}");
        }
        assert!(!html.contains("<span class=\"t\">?</span>"));
    }

    #[test]
    fn a_mirror_at_the_top_is_tagged_but_not_linked() {
        let html = menu_page(&url("gopher://h/"), b"+Mirror\t/\tm.example\t70\r\n");
        assert!(html.contains("<span class=\"t\">MIR</span>   Mirror<br>"));
        assert!(!html.contains("<a "));
    }

    #[test]
    fn mirrors_join_the_line_of_the_item_before_them() {
        let html = menu_page(
            &url("gopher://h/"),
            b"0Notes\t/n.txt\tprimary.example\t70\r\n+Notes\t/copy/n.txt\tm1.example\t7070\r\n\
              +Notes\t/n.txt\t::1\t70\r\n1Next\t/next\th\t70\r\n",
        );
        assert!(
            html.contains(
                "<a href=\"gopher://primary.example/0/n.txt\">Notes</a> \
                 <span class=\"t\">(mirror:</span> <a href=\"gopher://m1.example:7070/0/copy/n.txt\">m1.example:7070</a>\
                 <span class=\"t\">)</span> \
                 <span class=\"t\">(mirror:</span> <a href=\"gopher://[::1]/0/n.txt\">[::1]:70</a>\
                 <span class=\"t\">)</span><br>"
            ),
            "{html}"
        );
        assert!(!html.contains("MIR"));
        assert!(html.contains(">Next</a><br></pre>"));
    }

    #[test]
    fn a_mirror_after_a_line_that_is_no_item_is_text() {
        for before in [
            &b"iWelcome\t\t\t\r\n"[..],
            b"3Gone\t\t\t\r\n",
            b"8Login\t\tbbs.example\t23\r\n",
            b"2Phone book\t\tns.example\t105\r\n",
            b"1Bad\t/b\tevil.example/x\t70\r\n",
            b"\r\n",
        ] {
            let mut body = before.to_vec();
            body.extend_from_slice(b"+Mirror\t/m\tm.example\t70\r\n");
            let html = menu_page(&url("gopher://h/"), &body);
            assert!(html.contains("<span class=\"t\">MIR</span>   Mirror<br>"), "{html}");
            assert!(!html.contains("m.example"), "{html}");
        }
    }

    #[test]
    fn a_mirror_without_a_plain_host_is_left_out() {
        let html = menu_page(
            &url("gopher://h/"),
            b"0A\t/a\th\t70\r\n+A\t/a\t\t70\r\n+A\t/a\tevil.example/x\t70\r\n+A\t/a\tm.example\tseventy\r\n\
              +A\t/a\tm.example\t70\r\n",
        );
        assert_eq!(html.matches("(mirror:").count(), 1, "{html}");
        assert!(html.contains("<a href=\"gopher://m.example/0/a\">m.example:70</a>"));
        assert!(!html.contains("MIR"));
    }

    #[test]
    fn a_hostile_mirror_cannot_break_out_of_its_link() {
        let html = menu_page(
            &url("gopher://h/"),
            b"0A\t/a\th\t70\r\n+A\t/\"><script>\tm.example\t70\r\n",
        );
        assert!(!html.contains("<script>"));
        assert!(html.contains("gopher://m.example/0/%22%3E%3Cscript%3E"));
    }

    #[test]
    fn a_streamed_menu_closes_the_line_of_its_last_item() {
        let mut lines = Lines::menu(&url("gopher://h/"), encoding_rs::UTF_8);
        let first = lines.line(b"0A\t/a\th\t70").unwrap();
        assert!(first.ends_with("</a>"), "{first}");
        assert!(lines.line(b"+A\t/a\tm.example\t70").unwrap().contains("(mirror:"));
        assert!(lines.line(b"iAfter\t\t\t").unwrap().starts_with("<br>"));
        assert_eq!(lines.end(), "");
        lines.line(b"0B\t/b\th\t70");
        assert_eq!(lines.end(), "<br>");
    }

    #[test]
    fn a_menu_remembers_its_mirrors_only_where_asked() {
        let menu = url("gopher://h/1/remembers");
        let body = b"0A\t/a\th\t70\r\n+A\t/a\tm.example\t70\r\n";
        let item = url("gopher://h/0/a");
        let page = |lines: Lines| whole(lines, &menu, body);
        page(Lines::menu(&menu, encoding_rs::UTF_8));
        assert!(mirrors::of("render-zone", &menu, &item).is_empty());
        page(Lines::menu(&menu, encoding_rs::UTF_8).remembering_mirrors("render-zone", &menu));
        assert_eq!(
            mirrors::of("render-zone", &menu, &item),
            vec![url("gopher://m.example/0/a")]
        );
    }

    #[test]
    fn extension_types_round_trip_through_their_url() {
        for t in ['P', 'r', 'x', 'c', 'M', ':', '<'] {
            let u = gopher_url("h", 70, t, b"/s").unwrap();
            let r = Request::from_url(&u).unwrap();
            assert_eq!((r.item_type, r.selector.as_slice()), (t, &b"/s"[..]), "{u}");
        }
    }

    #[test]
    fn a_selector_with_dot_segments_round_trips_through_its_url() {
        for selector in [
            &b"/a/../b"[..],
            b"/a/./b",
            b"/a/..",
            b"/a/.",
            b"/../x",
            b"/./.././",
            b"/a/%2e%2E/b",
        ] {
            let u = gopher_url("h", 70, '1', selector).unwrap();
            let u = Url::parse(u.as_str()).unwrap();
            let r = Request::from_url(&u).unwrap();
            assert_eq!(r.item_type, '1', "{u}");
            assert_eq!(r.selector, selector, "{u}");
        }
    }

    #[test]
    fn only_the_slash_before_a_dot_segment_is_escaped() {
        let u = gopher_url("h", 70, '1', b"/a/../b/...").unwrap();
        assert_eq!(u.as_str(), "gopher://h/1/a%2F../b/...");
    }

    #[test]
    fn a_dot_item_type_keeps_its_selector() {
        let u = gopher_url("h", 70, '.', b"/x").unwrap();
        assert_eq!(u.as_str(), "gopher://h/.%2Fx");
        assert_eq!(Request::from_url(&u).unwrap().selector, b"/x");
        assert_eq!(gopher_url("h", 70, '.', b""), None);
    }

    #[test]
    fn a_gopher_plus_item_links_plainly_and_to_its_attributes() {
        let html = menu_page(
            &url("gopher://h:7070/"),
            b"0Plus\t/a b\th\t70\t+\r\n1Here\t/d\t\t\t+\r\n0Old\t/o\th\t70\r\n8Tel\t\th\t23\t+\r\n",
        );
        assert!(
            html.contains("<a href=\"gopher://h/0/a%20b\">Plus</a> <a href=\"gopher://h/0/a%20b%09%09!\">[info]</a>")
        );
        assert!(html.contains("<a href=\"gopher://h:7070/1/d%09%09!\">[info]</a>"));
        assert_eq!(html.matches("[info]").count(), 2);
    }

    #[test]
    fn a_gopher_plus_item_with_a_mirror_shows_its_extras_then_the_mirror() {
        let html = menu_page(
            &url("gopher://h/"),
            b"0Doc\t/d\th\t70\t+\r\n+Doc\t/copy/d\tm.example\t70\t+\r\n0Next\t/n\th\t70\r\n",
        );
        assert!(
            html.contains(
                "<a href=\"gopher://h/0/d\">Doc</a> <a href=\"gopher://h/0/d%09%09!\">[info]</a> \
<span class=\"t\">(mirror:</span> <a href=\"gopher://m.example/0/copy/d\">m.example:70</a>\
<span class=\"t\">)</span><br>"
            ),
            "{html}"
        );
        assert_eq!(html.matches("[info]").count(), 1, "{html}");
    }

    #[test]
    fn an_item_with_a_form_links_to_it() {
        let html = menu_page(&url("gopher://h/"), b"0Ask me\t/q\th\t70\t?\r\n");
        assert!(
            html.contains(
                "<a href=\"gopher://h/0/q\">Ask me</a> <a href=\"gopher://h/0/q%09%09%3F\">[form]</a> \
<a href=\"gopher://h/0/q%09%09!\">[info]</a>"
            ),
            "{html}"
        );
    }

    #[test]
    fn a_gopher_plus_string_round_trips_through_its_url() {
        for (selector, plus) in [
            (&b"/a"[..], &b"+"[..]),
            (b"/odd ?#%/../x", b"+text/plain De_DE"),
            (b"/a", b"+application/gopher+-menu"),
            (b"", b"!"),
            (b"/d", b"$+VIEWS +ABSTRACT"),
            (b"/f", b"?"),
            (b"/f", b"+../.."),
        ] {
            let u = gopher_plus_url("h", 70, '0', selector, plus).unwrap();
            let r = Request::from_url(&Url::parse(u.as_str()).unwrap()).unwrap();
            assert_eq!(r.selector, selector, "{u}");
            assert_eq!(r.search, None, "{u}");
            assert_eq!(r.gopher_plus.as_deref(), Some(plus), "{u}");
        }
        let u = gopher_plus_url("h", 70, '7', b"/find", b"!").unwrap();
        assert_eq!(u.as_str(), "gopher://h/7/find%09%09!");
        assert_eq!(Request::from_url(&u).unwrap().wire_line(), b"/find\t\t!\r\n");
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
    fn a_latin1_text_file_keeps_its_characters() {
        let html = text_page(&url("gopher://h/0/x"), b"Gr\xfc\xdfe aus K\xf6ln\r\n.\r\n");
        assert!(html.contains("<pre>Gr\u{fc}\u{df}e aus K\u{f6}ln\n</pre>"));
    }

    #[test]
    fn a_utf8_text_file_is_unchanged() {
        let html = text_page(&url("gopher://h/0/x"), "Gr\u{fc}\u{df}e \u{2603}\r\n".as_bytes());
        assert!(html.contains("<pre>Gr\u{fc}\u{df}e \u{2603}\n</pre>"));
    }

    #[test]
    fn a_latin1_menu_keeps_its_characters() {
        let html = menu_page(&url("gopher://h/"), b"0Caf\xe9\t/c\th\t70\r\n");
        assert!(html.contains(">Caf\u{e9}</a>"));
    }

    #[test]
    fn the_search_form_submits_to_the_search_url() {
        let html = search_page(&url("gopher://h/7/find"));
        assert!(html.contains("action=\"gopher://h/7/find\""));
        assert!(html.contains("name=\"q\""));
    }
}
