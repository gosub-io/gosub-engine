//! A menu or text page served as its lines arrive.
//!
//! The page's head goes first, then each line's markup once the line is whole, then the page's
//! end when the body ends: at its terminator, or when the server closes. The body has no
//! length, so the bytes read are capped as a body read whole is: the page is held in memory to
//! be parsed anyway.

use super::menu;
use super::render::{self, Lines};
use crate::net::protocols::socket::invalid;
use encoding_rs::Encoding;
use std::io;
use std::pin::Pin;
use std::task::{ready, Context, Poll};
use tokio::io::{AsyncRead, ReadBuf};

/// The page of a body whose first bytes are `peek` and whose rest `inner` reads, at most
/// `max_bytes` of body in all. Reads the page from just after its head: that goes in the
/// response's first bytes.
pub struct PageStream {
    inner: Box<dyn AsyncRead + Send + Unpin>,
    lines: Lines,
    /// Body bytes after the last whole line.
    partial: Vec<u8>,
    /// Markup not yet read, from `at`.
    out: Vec<u8>,
    at: usize,
    read: usize,
    max_bytes: usize,
    host: String,
    ended: bool,
}

impl PageStream {
    pub fn new(
        lines: Lines,
        peek: Vec<u8>,
        inner: Box<dyn AsyncRead + Send + Unpin>,
        host: &str,
        max_bytes: usize,
    ) -> Self {
        let mut stream = PageStream {
            inner,
            lines,
            read: peek.len(),
            partial: peek,
            out: Vec::new(),
            at: 0,
            max_bytes,
            host: host.to_string(),
            ended: false,
        };
        stream.take_lines();
        stream
    }

    /// Turn every whole line in `partial` into markup, up to the terminator.
    fn take_lines(&mut self) {
        let mut start = 0;
        while let Some(len) = self.partial[start..].iter().position(|&b| b == b'\n') {
            let line = &self.partial[start..start + len];
            start += len + 1;
            match self.lines.line(line.strip_suffix(b"\r").unwrap_or(line)) {
                Some(html) => self.out.extend_from_slice(html.as_bytes()),
                None => {
                    self.end();
                    return;
                }
            }
        }
        self.partial.drain(..start);
    }

    /// The server closed: a last line without a line break is a line too.
    fn finish(&mut self) {
        if !self.partial.is_empty() {
            let line = std::mem::take(&mut self.partial);
            if let Some(html) = self.lines.line(line.strip_suffix(b"\r").unwrap_or(&line)) {
                self.out.extend_from_slice(html.as_bytes());
            }
        }
        self.end();
    }

    fn end(&mut self) {
        self.partial = Vec::new();
        self.out.extend_from_slice(render::page_end().as_bytes());
        self.ended = true;
    }
}

impl AsyncRead for PageStream {
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
                this.finish();
                continue;
            }
            this.read += got.len();
            if this.read > this.max_bytes {
                this.ended = true;
                let message = format!("response from {} is larger than {} bytes", this.host, this.max_bytes);
                return Poll::Ready(Err(invalid(message).to_io()));
            }
            this.partial.extend_from_slice(got);
            this.take_lines();
        }
    }
}

/// The encoding a streamed body starts in, by the whole lines of its first bytes: a character
/// cut off at the end of them is no sign of anything.
pub fn encoding_for(peek: &[u8]) -> &'static Encoding {
    let whole = peek
        .iter()
        .rposition(|&b| b == b'\n')
        .map_or(&peek[..0], |i| &peek[..=i]);
    menu::encoding_for(whole)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use tokio::io::AsyncReadExt;
    use url::Url;

    /// A reader that hands out `chunks` one read at a time, as a socket might.
    struct Chunks(VecDeque<Vec<u8>>);

    impl AsyncRead for Chunks {
        fn poll_read(self: Pin<&mut Self>, _: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
            let this = self.get_mut();
            if let Some(mut chunk) = this.0.pop_front() {
                let n = buf.remaining().min(chunk.len());
                buf.put_slice(&chunk[..n]);
                if n < chunk.len() {
                    this.0.push_front(chunk.split_off(n));
                }
            }
            Poll::Ready(Ok(()))
        }
    }

    fn url() -> Url {
        Url::parse("gopher://h/1/").unwrap()
    }

    /// The page a body makes, its first `peek` bytes in hand and the rest read in `chunks`,
    /// and what the same body makes read whole.
    async fn both(text: bool, body: &[u8], peek: usize, chunks: &[usize]) -> (String, String) {
        let (peek, mut rest) = (body[..peek].to_vec(), &body[peek..]);
        let mut parts = VecDeque::new();
        for &n in chunks {
            let (part, tail) = rest.split_at(n.min(rest.len()));
            parts.push_back(part.to_vec());
            rest = tail;
        }
        parts.push_back(rest.to_vec());
        // An empty read is the end.
        parts.retain(|part| !part.is_empty());
        let encoding = encoding_for(&peek);
        let lines = if text {
            Lines::text(encoding)
        } else {
            Lines::menu(&url(), encoding)
        };
        let mut page = render::page_head(&url());
        PageStream::new(lines, peek, Box::new(Chunks(parts)), "h", usize::MAX)
            .read_to_string(&mut page)
            .await
            .unwrap();
        let whole = if text {
            render::text_page(&url(), body)
        } else {
            render::menu_page(&url(), body)
        };
        (page, whole)
    }

    #[tokio::test]
    async fn a_menu_split_anywhere_makes_the_page_it_makes_whole() {
        let body = b"iHello\t\t\t\r\n1Sub\t/sub\th\t70\r\n0Text\t/t\th\t70\r\n.\r\niAfter\t\t\t\r\n";
        for at in 0..body.len() {
            let (page, whole) = both(false, body, 0, &[at, 1, 2]).await;
            assert_eq!(page, whole, "split at {at}");
            assert!(page.contains("<a href=\"gopher://h/1/sub\">Sub</a>"));
            assert!(!page.contains("After"));
        }
    }

    #[tokio::test]
    async fn a_text_file_split_anywhere_is_unstuffed_and_ends_at_its_dot() {
        let body = b"first\r\n..dot\r\n.\r\nafter\r\n";
        for at in 0..body.len() {
            let (page, whole) = both(true, body, 0, &[at, 1]).await;
            assert_eq!(page, whole, "split at {at}");
            assert!(page.contains("<pre>first\n.dot\n</pre>"), "split at {at}: {page}");
        }
    }

    #[tokio::test]
    async fn chunk_boundaries_inside_crlf_and_the_terminator_are_whole_lines() {
        let body = b"iOne\t\t\t\r\niTwo\t\t\t\r\n.\r\n";
        // Inside the first CRLF, then inside `.` CR LF.
        let (page, whole) = both(false, body, 0, &[9, 10, 1, 1]).await;
        assert_eq!(page, whole);
        let (page, whole) = both(false, body, 0, &[20, 1, 1]).await;
        assert_eq!(page, whole);
        assert!(page.ends_with("Two<br></pre></body></html>"), "{page}");
    }

    #[tokio::test]
    async fn a_last_line_without_a_break_is_read() {
        let (page, whole) = both(true, b"one\r\ntwo", 3, &[2]).await;
        assert_eq!(page, whole);
        assert!(page.contains("<pre>one\ntwo\n</pre>"));
    }

    #[tokio::test]
    async fn the_peek_s_lines_come_first() {
        let (page, whole) = both(false, b"iA\t\t\t\r\niB\t\t\t\r\n.\r\n", 12, &[]).await;
        assert_eq!(page, whole);
    }

    #[tokio::test]
    async fn a_latin1_body_is_windows_1252_from_its_first_line() {
        let body = b"Gr\xfc\xdfe\r\naus K\xf6ln\r\n";
        for peek in 0..body.len() {
            let (page, _) = both(true, body, peek, &[3]).await;
            assert!(
                page.contains("<pre>Gr\u{fc}\u{df}e\naus K\u{f6}ln\n</pre>"),
                "peek {peek}: {page}"
            );
        }
    }

    #[tokio::test]
    async fn a_utf8_character_split_by_a_read_stays_whole() {
        let body = "Gr\u{fc}\u{df}e \u{2603}\r\n".as_bytes();
        for at in 0..body.len() {
            let (page, whole) = both(true, body, at, &[1]).await;
            assert_eq!(page, whole, "split at {at}");
        }
    }

    #[tokio::test]
    async fn a_line_that_is_not_utf8_turns_the_rest_to_windows_1252() {
        // Decided by the first line only, unlike a body read whole.
        let body = b"i\xc3\xa9\t\t\t\r\ni\xe9\t\t\t\r\ni\xc3\xa9\t\t\t\r\n";
        let (page, _) = both(false, body, 11, &[]).await;
        assert!(
            page.contains("      \u{e9}<br>      \u{e9}<br>      \u{c3}\u{a9}<br>"),
            "{page}"
        );
    }

    #[tokio::test]
    async fn a_body_over_the_cap_ends_in_an_error() {
        let body = b"iOne\t\t\t\r\n".repeat(10);
        let mut stream = PageStream::new(
            Lines::menu(&url(), encoding_rs::UTF_8),
            Vec::new(),
            Box::new(Chunks(VecDeque::from([body]))),
            "h",
            50,
        );
        let err = stream.read_to_end(&mut Vec::new()).await.unwrap_err();
        assert!(err.to_string().contains("larger than 50 bytes"), "{err}");
    }

    #[test]
    fn a_character_cut_off_at_the_end_of_the_peek_is_no_sign_of_latin1() {
        assert_eq!(encoding_for(b"iA\r\ni\xe2\x98"), encoding_rs::UTF_8);
        assert_eq!(encoding_for(b"i\xe9\r\ni"), encoding_rs::WINDOWS_1252);
        assert_eq!(encoding_for(b"no line yet \xe9"), encoding_rs::UTF_8);
    }
}
