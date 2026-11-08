//! Gopher+ (Anklesaria et al., "Gopher+: Upward compatible enhancements to the Internet Gopher
//! protocol", 1993): the reply to a Gopher+ request.
//!
//! Every reply to a request with a Gopher+ string starts with a header line (Gopher+ 2.3):
//! `+<bytes>` for a data block of that length, `+-1` for one ended by a line holding a single
//! `.`, `+-2` for one ended by the server closing. A `-` in place of the `+` is an error, its
//! message in the block: an error code first (1 not available, 2 try later, 3 moved), then
//! the administrator and the message. The UMN server puts the message on the header line
//! itself (`-1 message`), so that is read too.
//!
//! A reply that does not start with a header is a server that answered in plain gopher, and
//! is taken whole.
//!
//! An item's attributes (Gopher+ 2.5, 2.7) are blocks of text, each started by a `+NAME:`
//! line, and are shown as text: the item's descriptor (+INFO) as a menu line, its views
//! (+VIEWS) as links to each view. Everything in them is the server's and is escaped.

use super::menu::{self, Item, Plus};
use super::render::{self, plus_url, Lines};
use super::request::Request;
use crate::net::protocols::page::escape;
use crate::net::protocols::BodyStream;
use std::io;
use std::pin::Pin;
use std::task::{ready, Context, Poll};
use tokio::io::{AsyncRead, AsyncReadExt, ReadBuf};
use url::Url;

/// Where a Gopher+ data block ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Length {
    /// After this many bytes.
    Bytes(u64),
    /// At a line holding a single `.`.
    Dot,
    /// When the server closes.
    Close,
}

/// A Gopher+ reply's header line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    /// `+`, or `-` for an error.
    pub ok: bool,
    pub length: Length,
    /// An error's first line when the header line carries it, UMN style: its code and message.
    pub text: Vec<u8>,
}

/// The header at the start of a reply whose first bytes are `peek`, and how many bytes it
/// takes with its line break. `Ok(None)` when the reply has none. `closed` says the reply ends
/// within `peek`: a header without a line break then ends the reply.
pub fn header(peek: &[u8], closed: bool) -> Result<Option<(Header, usize)>, String> {
    let ok = match peek.first() {
        Some(b'+') => true,
        Some(b'-') => false,
        _ => return Ok(None),
    };
    let (line, len) = match peek.iter().position(|&b| b == b'\n') {
        Some(at) => (&peek[..at], at + 1),
        None if closed => (peek, peek.len()),
        None => return Err(format!("Gopher+ reply header longer than {} bytes", peek.len())),
    };
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    let (token, text) = match line[1..].iter().position(|&b| b == b' ') {
        Some(at) => (&line[1..1 + at], &line[2 + at..]),
        None => (&line[1..], &b""[..]),
    };
    let number = !token.is_empty() && token.iter().all(u8::is_ascii_digit);
    if !ok && number && !text.is_empty() {
        // `-<code> <message>`: there is no block.
        return Ok(Some((
            Header {
                ok,
                length: Length::Bytes(0),
                text: line[1..].to_vec(),
            },
            len,
        )));
    }
    let length = match token {
        b"-1" => Length::Dot,
        b"-2" => Length::Close,
        digits if number => match std::str::from_utf8(digits).ok().and_then(|d| d.parse().ok()) {
            Some(n) => Length::Bytes(n),
            None => return Ok(None),
        },
        _ => return Ok(None),
    };
    Ok(Some((
        Header {
            ok,
            length,
            text: Vec::new(),
        },
        len,
    )))
}

/// A data block's bytes, `peek` in hand and the rest to come, cut where `length` says it ends.
pub fn limit(mut peek: Vec<u8>, rest: Option<BodyStream>, length: Length) -> (Vec<u8>, Option<BodyStream>) {
    match length {
        Length::Close => (peek, rest),
        Length::Bytes(n) => {
            if peek.len() as u64 >= n {
                peek.truncate(n as usize);
                return (peek, None);
            }
            let left = n - peek.len() as u64;
            let rest = rest.map(|rest| BodyStream {
                reader: Box::new(rest.reader.take(left)),
                idle: rest.idle,
            });
            (peek, rest)
        }
        Length::Dot => {
            let mut end = DotEnd::default();
            let mut out = Vec::with_capacity(peek.len());
            if end.feed(&peek, &mut out) {
                return (out, None);
            }
            let rest = rest.map(|rest| BodyStream {
                reader: Box::new(DotEnded {
                    inner: rest.reader,
                    end,
                    out: Vec::new(),
                    at: 0,
                    ended: false,
                }),
                idle: rest.idle,
            });
            match rest {
                Some(rest) => (out, Some(rest)),
                None => (out, None),
            }
        }
    }
}

/// What an error reply says, as text: the meaning of its code, then the rest of its first
/// line and the lines after.
pub fn error_text(header: &Header, block: &[u8]) -> String {
    let mut text = header.text.clone();
    if !text.is_empty() {
        text.extend_from_slice(b"\n");
    }
    text.extend_from_slice(block);
    let encoding = super::menu::encoding_for(&text);
    let text = super::menu::decode(encoding, &text);
    let mut lines = text.lines().map(str::trim_end);
    let first = lines.next().unwrap_or_default();
    let (code, after) = first.split_once(' ').unwrap_or((first, ""));
    let mut out = match code {
        "1" => "Item is not available.".to_string(),
        "2" => "Try again later.".to_string(),
        "3" => "Item has moved.".to_string(),
        _ => format!("Gopher+ error {code}"),
    };
    for line in std::iter::once(after.trim()).chain(lines) {
        if !line.is_empty() && line != "." {
            out.push('\n');
            out.push_str(line);
        }
    }
    out
}

/// One attribute block: `+<name>: <value>` and the lines after it, each without the space
/// that starts it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block<'a> {
    pub name: &'a [u8],
    pub value: &'a [u8],
    pub lines: Vec<&'a [u8]>,
}

/// The attribute blocks of a body, up to a line holding a single `.`. Lines before the first
/// block are dropped.
pub fn blocks(body: &[u8]) -> Vec<Block<'_>> {
    let mut out: Vec<Block> = Vec::new();
    for line in menu::lines(body).take_while(|&line| line != b".") {
        if let Some(rest) = line.strip_prefix(b"+") {
            let end = rest.iter().position(|&b| b == b':' || b == b' ').unwrap_or(rest.len());
            let value = rest[end..].strip_prefix(b":").unwrap_or(&rest[end..]);
            out.push(Block {
                name: &rest[..end],
                value: value.trim_ascii_start(),
                lines: Vec::new(),
            });
        } else if let Some(block) = out.last_mut() {
            block.lines.push(line.strip_prefix(b" ").unwrap_or(line));
        }
    }
    out
}

/// One line of a +VIEWS block (Gopher+ A3.0): `<content type>[ <language>]: [<size>]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View<'a> {
    /// What a request for the view sends after the `+`: content type and language.
    pub name: &'a [u8],
    /// The size the server gives, without its angle brackets.
    pub size: Option<&'a [u8]>,
}

/// The views a +VIEWS block lists, the server's preferred first. A line that would not make
/// a request line is left out.
pub fn views<'a>(block: &Block<'a>) -> Vec<View<'a>> {
    block
        .lines
        .iter()
        .filter_map(|line| {
            let colon = line.iter().position(|&b| b == b':')?;
            let name = line[..colon].trim_ascii();
            if name.is_empty() || super::request::check_line_safe(name, "view").is_err() {
                return None;
            }
            let after = &line[colon + 1..];
            let size = after
                .iter()
                .position(|&b| b == b'<')
                .and_then(|open| {
                    let close = after[open..].iter().position(|&b| b == b'>')?;
                    Some(&after[open + 1..open + close])
                })
                .filter(|size| !size.is_empty());
            Some(View { name, size })
        })
        .collect()
}

/// A page of the attributes in `body`, the reply to `request` for `url` (`!` or `$`).
pub fn attributes_page(url: &Url, request: &Request, body: &[u8]) -> String {
    let encoding = menu::encoding_for(body);
    let host = url.host_str().unwrap_or_default();
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let port = request.port;
    let mut lines = Lines::menu(url, encoding);
    // The item asked about, until a +INFO block names one.
    let mut item = Some(Item {
        item_type: request.item_type,
        display: String::new(),
        selector: request.selector.clone(),
        host: String::new(),
        port: Some(port),
        plus: Plus::Item,
    });
    let mut out = render::page_head(url);
    for block in blocks(body) {
        let text = |bytes: &[u8]| escape(&menu::decode(encoding, bytes));
        out.push_str(&format!("<span class=\"t\">+{}:</span>", text(block.name)));
        match block.name {
            b"INFO" => {
                item = menu::parse_line(encoding, block.value);
                out.push(' ');
                out.push_str(&lines.line(block.value).unwrap_or_default());
            }
            b"VIEWS" => {
                out.push_str("<br>");
                for view in views(&block) {
                    let mut plus = b"+".to_vec();
                    plus.extend_from_slice(view.name);
                    let size = view
                        .size
                        .map(|size| format!(" <span class=\"t\">{}</span>", text(size)))
                        .unwrap_or_default();
                    match item.as_ref().and_then(|item| plus_url(item, host, port, &plus)) {
                        Some(href) => out.push_str(&format!(
                            "  <a href=\"{}\">{}</a>{size}<br>",
                            escape(href.as_str()),
                            text(view.name)
                        )),
                        None => out.push_str(&format!("  {}{size}<br>", text(view.name))),
                    }
                }
            }
            _ => {
                if !block.value.is_empty() {
                    out.push(' ');
                    out.push_str(&text(block.value));
                }
                out.push_str("<br>");
                for line in &block.lines {
                    out.push_str(&format!("  {}<br>", text(line)));
                }
            }
        }
    }
    out.push_str(&render::page_end());
    out
}

/// Finds the line holding a single `.` that ends a `+-1` block, a few bytes at a time: a `.`
/// at the start of a line is held back until the next byte says whether it is the end.
#[derive(Debug)]
struct DotEnd {
    held: Vec<u8>,
    line_start: bool,
}

impl Default for DotEnd {
    fn default() -> Self {
        DotEnd {
            held: Vec::new(),
            line_start: true,
        }
    }
}

impl DotEnd {
    /// Pass `input` to `out` up to the terminator. `true` once the terminator is found.
    fn feed(&mut self, input: &[u8], out: &mut Vec<u8>) -> bool {
        for &b in input {
            match (self.held.as_slice(), b) {
                ([], b'.') if self.line_start => self.held.push(b),
                ([b'.'], b'\r') => self.held.push(b),
                ([b'.'], b'\n') | ([b'.', b'\r'], b'\n') => {
                    self.held.clear();
                    return true;
                }
                _ => {
                    out.append(&mut self.held);
                    out.push(b);
                    self.line_start = b == b'\n';
                }
            }
        }
        false
    }

    /// The block closed without its terminator: a `.` held back at the very end is one.
    fn finish(&mut self) {
        self.held.clear();
    }
}

/// A `+-1` block's rest, ending at its terminator.
struct DotEnded {
    inner: Box<dyn AsyncRead + Send + Unpin>,
    end: DotEnd,
    out: Vec<u8>,
    at: usize,
    ended: bool,
}

impl AsyncRead for DotEnded {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        loop {
            if this.at < this.out.len() {
                let n = buf.remaining().min(this.out.len() - this.at);
                buf.put_slice(&this.out[this.at..this.at + n]);
                this.at += n;
                if this.at == this.out.len() {
                    this.out.clear();
                    this.at = 0;
                }
                return Poll::Ready(Ok(()));
            }
            if this.ended {
                return Poll::Ready(Ok(()));
            }
            let mut scratch = [0u8; 8 * 1024];
            let mut read = ReadBuf::new(&mut scratch);
            ready!(Pin::new(&mut this.inner).poll_read(cx, &mut read))?;
            let got = read.filled();
            if got.is_empty() {
                this.end.finish();
                this.ended = true;
                continue;
            }
            this.ended = this.end.feed(got, &mut this.out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::time::Duration;

    fn parsed(peek: &[u8]) -> Option<(Header, usize)> {
        header(peek, false).unwrap()
    }

    #[test]
    fn each_length_form_is_read() {
        let (h, len) = parsed(b"+5340\r\ndata").unwrap();
        assert_eq!((h.ok, h.length, len), (true, Length::Bytes(5340), 7));
        assert_eq!(parsed(b"+-1\r\n").unwrap().0.length, Length::Dot);
        assert_eq!(
            parsed(b"+-2\nx").unwrap(),
            (header(b"+-2\n", true).unwrap().unwrap().0, 4)
        );
        assert_eq!(parsed(b"+-2\nx").unwrap().0.length, Length::Close);
    }

    #[test]
    fn an_error_header_is_read() {
        let (h, _) = parsed(b"--1\r\n1 <admin@h>\r\n").unwrap();
        assert_eq!((h.ok, h.length), (false, Length::Dot));
        let (h, _) = parsed(b"--2\r\n").unwrap();
        assert_eq!((h.ok, h.length), (false, Length::Close));
        // The UMN server's form: the code and message on the header line, and no block.
        let (h, _) = parsed(b"-1 Item not found\r\n").unwrap();
        assert_eq!(
            (h.ok, h.length, h.text.as_slice()),
            (false, Length::Bytes(0), &b"1 Item not found"[..])
        );
    }

    #[test]
    fn a_reply_without_a_header_has_none() {
        for peek in [
            &b"iWelcome\t\t\t\r\n"[..],
            b"+Mirror\t/\th\t70\r\n",
            b"----- text\r\n",
            b"+\r\n",
            b"",
        ] {
            assert_eq!(parsed(peek), None, "{:?}", String::from_utf8_lossy(peek));
        }
    }

    #[test]
    fn a_header_without_its_line_break_is_whole_only_at_the_end() {
        assert!(header(b"+12345", false).is_err());
        assert_eq!(header(b"+-2", true).unwrap().unwrap().0.length, Length::Close);
    }

    /// A reader that hands out `chunks` one read at a time.
    struct Chunks(VecDeque<Vec<u8>>);

    impl AsyncRead for Chunks {
        fn poll_read(self: Pin<&mut Self>, _: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
            if let Some(chunk) = self.get_mut().0.pop_front() {
                buf.put_slice(&chunk);
            }
            Poll::Ready(Ok(()))
        }
    }

    /// The block `length` cuts from a body whose first `peek` bytes are in hand and whose
    /// rest arrives a byte at a time.
    async fn block(body: &[u8], peek: usize, length: Length) -> Vec<u8> {
        let rest: VecDeque<Vec<u8>> = body[peek..].iter().map(|&b| vec![b]).collect();
        let rest = BodyStream {
            reader: Box::new(Chunks(rest)),
            idle: Duration::from_secs(1),
        };
        let (mut out, rest) = limit(body[..peek].to_vec(), Some(rest), length);
        if let Some(mut rest) = rest {
            rest.reader.read_to_end(&mut out).await.unwrap();
        }
        out
    }

    #[tokio::test]
    async fn a_counted_block_ends_after_its_bytes_wherever_the_reads_fall() {
        let body = b"0123456789trailing";
        for peek in 0..body.len() {
            assert_eq!(block(body, peek, Length::Bytes(10)).await, b"0123456789", "peek {peek}");
        }
    }

    #[tokio::test]
    async fn a_dot_block_ends_at_its_terminator_wherever_the_reads_fall() {
        let body = b"one\r\n..two\r\n.x\r\n.\r\nafter\r\n";
        for peek in 0..body.len() {
            assert_eq!(
                block(body, peek, Length::Dot).await,
                b"one\r\n..two\r\n.x\r\n",
                "peek {peek}"
            );
        }
        assert_eq!(block(b".\r\n", 0, Length::Dot).await, b"");
        assert_eq!(block(b"a\n.\nb", 1, Length::Dot).await, b"a\n");
        assert_eq!(block(b"no end\r\n.", 2, Length::Dot).await, b"no end\r\n");
    }

    #[tokio::test]
    async fn a_close_block_is_everything() {
        assert_eq!(block(b"a\r\n.\r\nb", 2, Length::Close).await, b"a\r\n.\r\nb");
    }

    const ATTRIBUTES: &[u8] = b"+INFO: 0Some file\t/moo\thost2\t7070\t+\r\n\
+ADMIN:\r\n Admin: Frodo <fng@bogus.edu>\r\n Mod-Date: Wed Jul 28 1993 <19930728170201>\r\n\
+VIEWS:\r\n Text/plain: <10k>\r\n application/postscript: <100k>\r\n Text/plain De_DE: <15k>\r\n\
 bad\tview: <1k>\r\n nocolon\r\n\
+ABSTRACT:\r\n This is <b>short</b>.\r\nunindented\r\n.\r\n+AFTER:\r\n";

    #[test]
    fn attributes_parse_into_blocks() {
        let blocks = blocks(ATTRIBUTES);
        let names: Vec<&[u8]> = blocks.iter().map(|b| b.name).collect();
        assert_eq!(names, [&b"INFO"[..], b"ADMIN", b"VIEWS", b"ABSTRACT"]);
        assert_eq!(blocks[0].value, b"0Some file\t/moo\thost2\t7070\t+");
        assert_eq!(blocks[1].lines[0], b"Admin: Frodo <fng@bogus.edu>");
        assert_eq!(blocks[3].lines, [&b"This is <b>short</b>."[..], b"unindented"]);
    }

    #[test]
    fn views_parse_with_their_language_and_size() {
        let blocks = blocks(ATTRIBUTES);
        assert_eq!(
            views(&blocks[2]),
            [
                View {
                    name: b"Text/plain",
                    size: Some(b"10k")
                },
                View {
                    name: b"application/postscript",
                    size: Some(b"100k")
                },
                View {
                    name: b"Text/plain De_DE",
                    size: Some(b"15k")
                },
            ]
        );
        let block = Block {
            name: b"VIEWS",
            value: b"",
            lines: vec![b"application/gopher+-menu: <512b>", b"image/gif:"],
        };
        assert_eq!(views(&block)[0].name, b"application/gopher+-menu");
        assert_eq!(views(&block)[1].size, None);
    }

    #[test]
    fn an_attributes_page_links_the_item_and_each_view() {
        let url = Url::parse("gopher://h/0/moo%09%09!").unwrap();
        let request = Request::from_url(&url).unwrap();
        let page = attributes_page(&url, &request, ATTRIBUTES);
        assert!(
            page.contains("<a href=\"gopher://host2:7070/0/moo\">Some file</a>"),
            "{page}"
        );
        assert!(page.contains("<a href=\"gopher://host2:7070/0/moo%09%09+Text/plain%20De_DE\">Text/plain De_DE</a> <span class=\"t\">15k</span>"), "{page}");
        assert!(
            page.contains("<a href=\"gopher://host2:7070/0/moo%09%09+application/postscript\">"),
            "{page}"
        );
        assert!(page.contains("This is &lt;b&gt;short&lt;/b&gt;."), "{page}");
        assert!(page.contains("Frodo &lt;fng@bogus.edu&gt;"));
        assert!(!page.contains("<b>"));
        assert!(!page.contains("AFTER"));
    }

    #[test]
    fn an_attributes_page_without_info_links_views_to_the_item_asked_about() {
        let url = Url::parse("gopher://h:7070/I/pic%09%09!+VIEWS").unwrap();
        let request = Request::from_url(&url).unwrap();
        let page = attributes_page(&url, &request, b"+VIEWS:\r\n image/gif: <3k>\r\n");
        assert!(
            page.contains("<a href=\"gopher://h:7070/I/pic%09%09+image/gif\">image/gif</a>"),
            "{page}"
        );
    }

    #[test]
    fn a_hostile_attribute_is_text() {
        let url = Url::parse("gopher://h/0/x%09%09!").unwrap();
        let request = Request::from_url(&url).unwrap();
        let body = b"+INFO: 0<script>x</script>\t/\"><img>\th\t70\t+\r\n+<b>EVIL</b>: <i>v</i>\r\n <svg onload=x>\r\n+VIEWS:\r\n text/\"><script>: <<b>>\r\n";
        let page = attributes_page(&url, &request, body);
        for active in ["<script", "<img", "<b>", "<i>", "<svg"] {
            assert!(!page.contains(active), "{active}: {page}");
        }
    }

    #[test]
    fn an_error_says_what_its_code_means() {
        let h = Header {
            ok: false,
            length: Length::Dot,
            text: Vec::new(),
        };
        let text = error_text(&h, b"1 Frodo <fng@bogus.edu>\r\nNo such file\r\n.\r\n");
        assert_eq!(text, "Item is not available.\nFrodo <fng@bogus.edu>\nNo such file");
        let umn = Header {
            text: b"2 Too busy".to_vec(),
            ..h
        };
        assert_eq!(error_text(&umn, b""), "Try again later.\nToo busy");
        assert_eq!(error_text(&h, b"9\r\n"), "Gopher+ error 9");
    }
}
