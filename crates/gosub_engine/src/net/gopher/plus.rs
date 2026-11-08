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

use crate::net::protocols::BodyStream;
use std::io;
use std::pin::Pin;
use std::task::{ready, Context, Poll};
use tokio::io::{AsyncRead, AsyncReadExt, ReadBuf};

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
