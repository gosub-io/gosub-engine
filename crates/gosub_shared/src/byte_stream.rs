use std::char::REPLACEMENT_CHARACTER;
use std::fmt::{Debug, Display, Formatter};
use std::io::Read;
use std::{fmt, io};

pub const CHAR_LF: char = '\u{000A}';
pub const CHAR_CR: char = '\u{000D}';

/// Encoding defines the way the buffer stream is read, as what defines a "character".
#[derive(PartialEq, Debug)]
pub enum Encoding {
    /// Unknown encoding. Won't read anything from the stream until the encoding is set.
    Unknown,
    /// Stream is of single-byte Latin-1 / ISO-8859-1 chars (0-255)
    Latin1,
    /// Stream is of UTF-8 characters
    UTF8,
    /// Stream consists of 16-bit UTF characters (Little Endian)
    UTF16LE,
    /// Stream consists of 16-bit UTF characters (Big Endian)
    UTF16BE,
}

/// Defines a single character/element in the stream. This is either a UTF8 character, or
/// a surrogate characters since these cannot be stored in a single char. Note that characters
/// are not the same as bytes, since a single character can be multiple bytes in UTF8 or UTF16.
///
/// Eof is denoted as a separate element, so is Empty to indicate that the buffer is empty but
/// not yet closed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Character {
    /// Standard UTF character
    Ch(char),
    /// Surrogate character (since they cannot be stored in char)
    Surrogate(u16),
    /// Stream buffer empty and closed (or open but exhausted)
    StreamEnd,
}

use Character::{Ch, StreamEnd, Surrogate};

/// Converts the given character to a char. This is only valid for UTF8 characters. Surrogate
/// and EOF characters are converted to 0x0000
impl From<&Character> for char {
    fn from(c: &Character) -> Self {
        match c {
            Ch(c) => *c,
            Surrogate(..) | StreamEnd => 0x0000 as char,
        }
    }
}

impl From<Character> for char {
    fn from(c: Character) -> Self {
        match c {
            Ch(c) => c,
            Surrogate(..) | StreamEnd => 0x0000 as char,
        }
    }
}

impl fmt::Display for Character {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Ch(ch) => write!(f, "{ch}"),
            Surrogate(surrogate) => write!(f, "U+{surrogate:04X}"),
            StreamEnd => write!(f, "StreamEnd"),
        }
    }
}

impl Character {
    /// Returns true when the character is a whitespace
    #[must_use]
    pub fn is_whitespace(&self) -> bool {
        matches!(self, Ch(c) if c.is_whitespace())
    }

    /// Returns true when the character is a numerical
    #[must_use]
    pub fn is_numeric(&self) -> bool {
        matches!(self, Ch(c) if c.is_numeric())
    }

    /// Converts a slice of characters into a string
    pub fn slice_to_string(v: Vec<Character>) -> String {
        v.iter().map(char::from).collect()
    }
}

/// Configuration structure for a bytestream.
pub struct Config {
    /// Treat any CRLF pairs as a single LF
    pub cr_lf_as_one: bool,
    /// Replace any CR (without a pairing LF) with LF
    pub replace_cr_as_lf: bool,
    /// Are high ascii characters read as-is or converted to a replacement character
    pub replace_high_ascii: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            cr_lf_as_one: true,
            replace_cr_as_lf: false,
            replace_high_ascii: false,
        }
    }
}

pub struct ByteStream {
    /// Raw bytes of the source data, exactly as supplied. Kept so `detect_encoding` and
    /// `set_encoding` can re-interpret the source without the caller re-supplying it.
    raw: Vec<u8>,
    /// First byte of `raw` not yet transcoded into `text`. Lets `append_str` resume
    /// transcoding instead of re-scanning the whole buffer (O(n) total, not O(n^2)).
    /// May also point at a trailing element that cannot be classified yet on an open
    /// stream (incomplete multi-byte sequence, or a CR whose follower is unknown).
    raw_processed: usize,
    /// The decoded source as WTF-8: valid UTF-8 plus 3-byte encodings of lone
    /// surrogates (only produced by UTF-16 input). Newlines are already normalized
    /// per `config` (CRLF merge / lone-CR replacement), so reads never re-interpret
    /// them. This costs ~1 byte per input byte instead of a per-character table.
    text: Vec<u8>,
    /// Current byte position in `text`; always on a character boundary.
    text_pos: usize,
    /// Byte offset in `text` of the first character of each line (index 0 = line 1)
    line_starts: Vec<usize>,
    /// Cached index into `line_starts` from the last `location()` call (lookup hint)
    last_line_idx: std::cell::Cell<usize>,
    /// Cached `(text position, column)` of the last `location()` call. Columns count
    /// characters, so this resumes counting instead of re-scanning a (possibly very
    /// long, e.g. minified) line from its start on every call.
    col_cache: std::cell::Cell<(usize, usize)>,
    /// True when the stream is closed (no more data will be added)
    closed: bool,
    /// Current encoding
    encoding: Encoding,
    /// Configuration for the stream
    config: Config,
}

/// Opaque snapshot of stream position for mark/reset.
pub struct StreamMark {
    text_pos: usize,
}

/// True for WTF-8 continuation bytes (`0b10xxxxxx`)
#[inline]
const fn is_continuation(b: u8) -> bool {
    b & 0xC0 == 0x80
}

/// Number of characters in a WTF-8 byte slice (a lone surrogate counts as one)
#[inline]
fn count_chars(bytes: &[u8]) -> usize {
    bytes.iter().filter(|&&b| !is_continuation(b)).count()
}

/// What follows a run of bytes handed to the transcoder, so a trailing CR can be
/// classified (pair with a following LF, or stand alone).
#[derive(Clone, Copy, PartialEq)]
enum RunEnd {
    /// A decoded character follows the run (e.g. a replacement char for an invalid
    /// sequence); it is known not to be a LF.
    Char,
    /// The raw buffer ends after this run; whether more data can arrive depends on
    /// whether the stream is closed.
    Stream,
}

/// Generic stream trait
pub trait Stream {
    /// Read current character without advancing
    fn read(&self) -> Character;
    /// Read current character and advance to next
    fn read_and_next(&mut self) -> Character;
    /// Look ahead without advancing
    fn look_ahead(&self, offset: usize) -> Character;
    /// Advance with 1 character
    fn next(&mut self);
    /// Advance with offset characters
    fn next_n(&mut self, offset: usize);
    /// Unread the current character
    fn prev(&mut self);
    /// Unread n characters
    fn prev_n(&mut self, n: usize);
    /// Seek to a specific position in bytes
    fn seek_bytes(&mut self, offset: usize);
    /// Return the current position in bytes
    fn tell_bytes(&self) -> usize;
    /// Retrieves a slice of the buffer without advancing
    fn get_slice(&mut self, len: usize) -> Vec<Character>;
    /// Resets the stream back to the start position
    fn reset_stream(&mut self);
    /// Closes the stream (no more data can be added)
    fn close(&mut self);
    /// Returns true when the stream is closed
    fn closed(&self) -> bool;
    /// Returns true when the stream is exhausted (position past all chars)
    fn exhausted(&self) -> bool;
    /// Returns true when the stream is closed and exhausted
    fn eof(&self) -> bool;
    /// Returns the current line/column/offset location
    fn location(&self) -> Location;
}

impl Default for ByteStream {
    fn default() -> Self {
        Self::new(Encoding::Unknown, None)
    }
}

impl Stream for ByteStream {
    #[inline]
    fn read(&self) -> Character {
        match self.char_at(self.text_pos) {
            Some((ch, _)) => ch,
            None => StreamEnd,
        }
    }

    #[inline]
    fn read_and_next(&mut self) -> Character {
        // Newlines were already normalized when `text` was produced, so this is a
        // plain decode-and-advance.
        match self.char_at(self.text_pos) {
            Some((ch, width)) => {
                self.text_pos += width;
                ch
            }
            None => StreamEnd,
        }
    }

    #[inline]
    fn look_ahead(&self, offset: usize) -> Character {
        let mut pos = self.text_pos;
        for _ in 0..offset {
            match self.char_at(pos) {
                Some((_, width)) => pos += width,
                None => return StreamEnd,
            }
        }
        match self.char_at(pos) {
            Some((ch, _)) => ch,
            None => StreamEnd,
        }
    }

    #[inline]
    fn next(&mut self) {
        self.next_n(1);
    }

    #[inline]
    fn next_n(&mut self, offset: usize) {
        for _ in 0..offset {
            match self.char_at(self.text_pos) {
                Some((_, width)) => self.text_pos += width,
                None => break,
            }
        }
    }

    #[inline]
    fn prev(&mut self) {
        self.prev_n(1);
    }

    #[inline]
    fn prev_n(&mut self, n: usize) {
        for _ in 0..n {
            while self.text_pos > 0 {
                self.text_pos -= 1;
                if !is_continuation(self.text[self.text_pos]) {
                    break;
                }
            }
        }
    }

    fn seek_bytes(&mut self, offset: usize) {
        // Land on the first character boundary at or after `offset`.
        let mut pos = offset.min(self.text.len());
        while pos < self.text.len() && is_continuation(self.text[pos]) {
            pos += 1;
        }
        self.text_pos = pos;
    }

    #[inline]
    fn tell_bytes(&self) -> usize {
        self.text_pos
    }

    fn get_slice(&mut self, len: usize) -> Vec<Character> {
        let mut slice = Vec::with_capacity(len);
        let mut pos = self.text_pos;
        for _ in 0..len {
            match self.char_at(pos) {
                Some((ch, width)) => {
                    slice.push(ch);
                    pos += width;
                }
                None => slice.push(StreamEnd),
            }
        }
        slice
    }

    fn reset_stream(&mut self) {
        self.text_pos = 0;
    }

    fn close(&mut self) {
        self.closed = true;
    }

    #[inline]
    fn closed(&self) -> bool {
        self.closed
    }

    #[inline]
    fn exhausted(&self) -> bool {
        self.text_pos >= self.text.len()
    }

    #[inline]
    fn eof(&self) -> bool {
        self.closed() && self.exhausted()
    }

    #[inline]
    fn location(&self) -> Location {
        // Find the last line that starts at or before text_pos. The stream advances
        // mostly monotonically, so start from the cached line index of the previous
        // call and walk from there — amortized O(1) instead of a binary search per call.
        let mut idx = self.last_line_idx.get().min(self.line_starts.len() - 1);
        while idx > 0 && self.line_starts[idx] > self.text_pos {
            idx -= 1;
        }
        while idx + 1 < self.line_starts.len() && self.line_starts[idx + 1] <= self.text_pos {
            idx += 1;
        }
        self.last_line_idx.set(idx);

        // Columns count characters, not bytes, so they must be tallied. Resume from
        // the previous call's position when it lies on the same line; otherwise count
        // from the line start. Monotonic scans stay amortized O(1) even on a single
        // multi-megabyte (e.g. minified) line.
        let line_start = self.line_starts[idx];
        let (cache_pos, cache_col) = self.col_cache.get();
        let column = if cache_pos >= line_start && cache_pos <= self.text_pos {
            cache_col + count_chars(&self.text[cache_pos..self.text_pos])
        } else {
            1 + count_chars(&self.text[line_start..self.text_pos])
        };
        self.col_cache.set((self.text_pos, column));

        Location {
            line: idx + 1,
            column,
            offset: self.text_pos,
        }
    }
}

impl ByteStream {
    #[must_use]
    pub fn new(encoding: Encoding, config: Option<Config>) -> Self {
        Self {
            config: config.unwrap_or_default(),
            raw: Vec::new(),
            raw_processed: 0,
            text: Vec::new(),
            text_pos: 0,
            line_starts: vec![0],
            last_line_idx: std::cell::Cell::new(0),
            col_cache: std::cell::Cell::new((0, 1)),
            closed: false,
            encoding,
        }
    }

    /// Create a stream from a string, fully decoded and closed, ready for parsing.
    pub fn from_str(s: &str, encoding: Encoding) -> Self {
        let mut stream = Self::new(encoding, None);
        stream.raw = Vec::from(s.as_bytes());
        // Close before transcoding so the buffer is processed exactly once.
        stream.closed = true;
        stream.transcode_pending();
        stream
    }

    /// Take a snapshot of the current position for later restoration.
    pub fn mark(&self) -> StreamMark {
        StreamMark {
            text_pos: self.text_pos,
        }
    }

    /// Restore position to a previously saved mark.
    pub fn reset_to_mark(&mut self, mark: StreamMark) {
        self.text_pos = mark.text_pos;
    }

    /// Decode the WTF-8 character starting at byte `pos` in `text`, returning it with
    /// its byte width. Returns `None` at (or past) the end of the decoded text.
    /// `text` is produced by our own transcoder, so it is always well-formed WTF-8.
    ///
    /// The ASCII path must stay small enough to inline into the tokenizers' per-char
    /// loops; the multi-byte tail is outlined to keep it that way.
    #[inline(always)]
    fn char_at(&self, pos: usize) -> Option<(Character, usize)> {
        let b0 = *self.text.get(pos)?;
        if b0 < 0x80 {
            return Some((Ch(b0 as char), 1));
        }
        self.char_at_multibyte(pos, b0)
    }

    #[cold]
    fn char_at_multibyte(&self, pos: usize, b0: u8) -> Option<(Character, usize)> {
        let (cp, width) = if b0 < 0xE0 {
            ((u32::from(b0 & 0x1F) << 6) | u32::from(self.text[pos + 1] & 0x3F), 2)
        } else if b0 < 0xF0 {
            (
                (u32::from(b0 & 0x0F) << 12)
                    | (u32::from(self.text[pos + 1] & 0x3F) << 6)
                    | u32::from(self.text[pos + 2] & 0x3F),
                3,
            )
        } else {
            (
                (u32::from(b0 & 0x07) << 18)
                    | (u32::from(self.text[pos + 1] & 0x3F) << 12)
                    | (u32::from(self.text[pos + 2] & 0x3F) << 6)
                    | u32::from(self.text[pos + 3] & 0x3F),
                4,
            )
        };

        if (0xD800..=0xDFFF).contains(&cp) {
            // WTF-8 encoding of a lone surrogate (UTF-16 input)
            #[allow(clippy::cast_possible_truncation)] // PANIC-SAFE: cp <= 0xDFFF fits u16
            return Some((Surrogate(cp as u16), width));
        }
        Some((char::from_u32(cp).map_or(Ch(REPLACEMENT_CHARACTER), Ch), width))
    }

    /// Reset all transcode state; the next `transcode_pending` starts from scratch.
    /// Used by the full-load paths (`read_from_str`, `read_from_file`, `set_encoding`).
    fn restart_transcode(&mut self) {
        self.text.clear();
        self.text_pos = 0;
        self.raw_processed = 0;
        self.line_starts.clear();
        self.line_starts.push(0);
        self.last_line_idx.set(0);
        self.col_cache.set((0, 1));
    }

    /// Transcode `raw[raw_processed..]` into normalized WTF-8 in `text`. On an open
    /// stream this stops before a trailing element that cannot be classified yet
    /// (incomplete multi-byte sequence, or a CR whose following character is unknown
    /// while newline normalization is on); the element is picked up again on the next
    /// append or on close.
    fn transcode_pending(&mut self) {
        match self.encoding {
            Encoding::Unknown => {
                // Unknown encoding: decode nothing until an encoding is set.
                self.raw_processed = self.raw.len();
            }
            Encoding::UTF8 => self.transcode_utf8(),
            Encoding::Latin1 => self.transcode_latin1(),
            Encoding::UTF16LE => self.transcode_utf16(u16::from_le_bytes),
            Encoding::UTF16BE => self.transcode_utf16(u16::from_be_bytes),
        }
    }

    /// Append `bytes` to `text`, recording a line start after every LF.
    fn push_run(text: &mut Vec<u8>, line_starts: &mut Vec<usize>, bytes: &[u8]) {
        let base = text.len();
        text.extend_from_slice(bytes);
        for i in memchr::memchr_iter(b'\n', bytes) {
            line_starts.push(base + i + 1);
        }
    }

    /// Append a single ASCII byte to `text`, recording a line start after a LF.
    fn push_byte(&mut self, b: u8) {
        debug_assert!(b < 0x80);
        self.text.push(b);
        if b == b'\n' {
            self.line_starts.push(self.text.len());
        }
    }

    /// Append a character to `text` as UTF-8.
    fn push_char(&mut self, c: char) {
        if (c as u32) < 0x80 {
            #[allow(clippy::cast_possible_truncation)] // PANIC-SAFE: < 0x80 fits u8
            self.push_byte(c as u8);
            return;
        }
        let mut buf = [0u8; 4];
        self.text.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
    }

    /// Append a lone surrogate to `text` as its 3-byte WTF-8 encoding.
    fn push_surrogate(&mut self, cu: u16) {
        #[allow(clippy::cast_possible_truncation)] // PANIC-SAFE: masked to < 0x100
        self.text.extend_from_slice(&[
            0xE0 | (cu >> 12) as u8,
            0x80 | ((cu >> 6) & 0x3F) as u8,
            0x80 | (cu & 0x3F) as u8,
        ]);
    }

    /// Transcode pending UTF-8 input: copy the longest valid prefix in bulk
    /// (normalizing newlines), replace invalid sequences, and stop at an incomplete
    /// trailing sequence on an open stream.
    fn transcode_utf8(&mut self) {
        while self.raw_processed < self.raw.len() {
            let start = self.raw_processed;
            match std::str::from_utf8(&self.raw[start..]) {
                Ok(_) => {
                    let consumed = self.emit_utf8_run(start, self.raw.len(), RunEnd::Stream);
                    self.raw_processed = start + consumed;
                    return;
                }
                Err(e) => {
                    let valid_end = start + e.valid_up_to();
                    let after = if e.error_len().is_some() {
                        RunEnd::Char // the invalid sequence becomes a replacement char
                    } else {
                        RunEnd::Stream
                    };
                    let consumed = self.emit_utf8_run(start, valid_end, after);
                    self.raw_processed = start + consumed;
                    if start + consumed < valid_end {
                        // A trailing CR was left pending for the next append.
                        return;
                    }
                    match e.error_len() {
                        Some(n) => {
                            self.push_char(REPLACEMENT_CHARACTER);
                            self.raw_processed += n;
                        }
                        None => {
                            // Incomplete sequence at the end of the buffer. On a closed
                            // stream it decodes to a single replacement character; on an
                            // open stream we stop and leave the tail for the next append.
                            if self.closed {
                                self.push_char(REPLACEMENT_CHARACTER);
                                self.raw_processed = self.raw.len();
                            }
                            return;
                        }
                    }
                }
            }
        }
    }

    /// Copy the valid UTF-8 bytes `raw[start..end]` into `text`, normalizing newlines
    /// per config. `after` says what follows the run so a trailing CR can be
    /// classified. Returns the number of bytes consumed; a trailing CR that cannot be
    /// classified yet on an open stream is left unconsumed.
    fn emit_utf8_run(&mut self, start: usize, end: usize, after: RunEnd) -> usize {
        let cr_merge = self.config.cr_lf_as_one;
        let cr_replace = self.config.replace_cr_as_lf;
        let closed = self.closed;
        let (raw, text, line_starts) = (&self.raw, &mut self.text, &mut self.line_starts);

        if !cr_merge && !cr_replace {
            Self::push_run(text, line_starts, &raw[start..end]);
            return end - start;
        }

        let mut i = start;
        let mut seg = start;
        while i < end {
            if raw[i] != b'\r' {
                i += 1;
                continue;
            }
            Self::push_run(text, line_starts, &raw[seg..i]);
            let next_is_lf = if i + 1 < end {
                raw[i + 1] == b'\n'
            } else {
                match after {
                    RunEnd::Char => false,
                    RunEnd::Stream if closed => false,
                    // The follower is unknown; leave the CR for the next append.
                    RunEnd::Stream => return i - start,
                }
            };
            if next_is_lf {
                if cr_merge {
                    Self::push_run(text, line_starts, b"\n");
                } else {
                    // A CR directly before a LF is kept; only lone CRs are replaced.
                    Self::push_run(text, line_starts, b"\r\n");
                }
                i += 2;
            } else {
                Self::push_run(text, line_starts, if cr_replace { b"\n" } else { b"\r" });
                i += 1;
            }
            seg = i;
        }
        Self::push_run(text, line_starts, &raw[seg..end]);
        end - start
    }

    /// Transcode pending Latin-1 input: every byte is one character; bytes above 0x7F
    /// map to the same Unicode code point (or '?' with `replace_high_ascii`).
    fn transcode_latin1(&mut self) {
        let cr_merge = self.config.cr_lf_as_one;
        let cr_replace = self.config.replace_cr_as_lf;
        let normalize_cr = cr_merge || cr_replace;

        let mut pos = self.raw_processed;
        while pos < self.raw.len() {
            let b = self.raw[pos];
            if b == b'\r' && normalize_cr {
                let next_is_lf = if pos + 1 < self.raw.len() {
                    self.raw[pos + 1] == b'\n'
                } else if self.closed {
                    false
                } else {
                    break; // follower unknown; leave the CR for the next append
                };
                if next_is_lf {
                    if cr_merge {
                        self.push_byte(b'\n');
                    } else {
                        self.push_byte(b'\r');
                        self.push_byte(b'\n');
                    }
                    pos += 2;
                } else {
                    self.push_byte(if cr_replace { b'\n' } else { b'\r' });
                    pos += 1;
                }
            } else if b < 0x80 {
                self.push_byte(b);
                pos += 1;
            } else if self.config.replace_high_ascii {
                self.push_byte(b'?');
                pos += 1;
            } else {
                self.push_char(b as char);
                pos += 1;
            }
        }
        self.raw_processed = pos;
    }

    /// Transcode pending UTF-16 input (endianness given by `conv`). Lone surrogates
    /// are preserved via their WTF-8 encoding. An odd trailing byte is never consumed.
    fn transcode_utf16(&mut self, conv: fn([u8; 2]) -> u16) {
        let cr_merge = self.config.cr_lf_as_one;
        let cr_replace = self.config.replace_cr_as_lf;
        let normalize_cr = cr_merge || cr_replace;

        let mut pos = self.raw_processed;
        while pos + 2 <= self.raw.len() {
            let cu = conv([self.raw[pos], self.raw[pos + 1]]);
            let next_cu = if pos + 4 <= self.raw.len() {
                Some(conv([self.raw[pos + 2], self.raw[pos + 3]]))
            } else {
                None
            };

            if cu == 0x000D && normalize_cr {
                let next_is_lf = if let Some(next) = next_cu {
                    next == 0x000A
                } else if self.closed {
                    false
                } else {
                    break; // follower unknown; leave the CR for the next append
                };
                if next_is_lf {
                    if cr_merge {
                        self.push_byte(b'\n');
                    } else {
                        self.push_byte(b'\r');
                        self.push_byte(b'\n');
                    }
                    pos += 4;
                } else {
                    self.push_byte(if cr_replace { b'\n' } else { b'\r' });
                    pos += 2;
                }
                continue;
            }

            let (ch, len) = decode_utf16_char(cu, || next_cu);
            match ch {
                Ch(c) => self.push_char(c),
                Surrogate(s) => self.push_surrogate(s),
                StreamEnd => {}
            }
            pos += len;
        }
        self.raw_processed = pos;
    }

    pub fn read_from_file(&mut self, mut f: impl Read) -> io::Result<()> {
        self.raw.clear();
        f.read_to_end(&mut self.raw)?;
        self.restart_transcode();
        self.close();
        self.reset_stream();
        Ok(())
    }

    pub fn read_from_str(&mut self, s: &str, encoding: Option<Encoding>) {
        self.raw = Vec::from(s.as_bytes());
        self.closed = false;
        if let Some(enc) = encoding {
            self.encoding = enc;
        }
        self.restart_transcode();
        self.transcode_pending();
        self.reset_stream();
    }

    pub fn append_str(&mut self, s: &str) {
        self.raw.extend_from_slice(s.as_bytes());
        // Resume transcoding from the first unprocessed byte instead of re-scanning
        // the whole buffer. text_pos indexes already-transcoded text, so it stays valid.
        self.transcode_pending();
    }

    pub fn close(&mut self) {
        self.closed = true;
        // Resume from any pending trailing element (incomplete sequence, unclassified
        // CR) so it resolves now. O(tail), not a full re-transcode.
        self.transcode_pending();
    }

    pub fn read_from_bytes(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.raw = bytes.to_vec();
        self.restart_transcode();
        self.close();
        self.reset_stream();
        Ok(())
    }

    #[cfg(test)]
    fn chars_left(&self) -> usize {
        count_chars(&self.text[self.text_pos..])
    }
}

impl ByteStream {
    /// Detect the given encoding from stream analysis
    pub fn detect_encoding(&self) -> Encoding {
        let mut buf = self.raw.as_slice();

        // Check for BOM
        if buf.starts_with(b"\xEF\xBB\xBF") {
            return Encoding::UTF8;
        } else if buf.starts_with(b"\xFF\xFE") {
            return Encoding::UTF16LE;
        } else if buf.starts_with(b"\xFE\xFF") {
            return Encoding::UTF16BE;
        }

        // Cap the buffer size we will check to max 64KB
        const MAX_BUF_SIZE: usize = 64 * 1024;
        let mut complete = true;
        if buf.len() > MAX_BUF_SIZE {
            buf = &buf[..MAX_BUF_SIZE];
            complete = false;
        }

        let mut encoding_detector = chardetng::EncodingDetector::new(chardetng::Iso2022JpDetection::Deny);
        encoding_detector.feed(buf, complete);

        let encoding = encoding_detector.guess(None, chardetng::Utf8Detection::Allow);
        if encoding == encoding_rs::UTF_16BE {
            Encoding::UTF16BE
        } else if encoding == encoding_rs::UTF_16LE {
            Encoding::UTF16LE
        } else {
            // Default to UTF-8 for all other detected encodings (including ASCII-compatible ones)
            Encoding::UTF8
        }
    }

    pub fn set_encoding(&mut self, e: Encoding) {
        if self.encoding == e {
            // Already decoded with this encoding; nothing to do.
            return;
        }
        // Positions cannot be mapped exactly across encodings; preserve the character
        // index, which is what downstream consumers observe. (Spec-correct handling of
        // a mid-parse encoding change would restart the parse entirely.)
        let char_index = count_chars(&self.text[..self.text_pos]);
        self.encoding = e;
        self.restart_transcode();
        self.transcode_pending();
        self.reset_stream();
        self.next_n(char_index);
    }
}

/// Location holds the start position of the given element in the data source
#[derive(Clone, PartialEq, Eq, Hash, Copy)]
pub struct Location {
    /// Line number, starting with 1
    pub line: usize,
    /// Column number, starting with 1
    pub column: usize,
    /// Byte offset, starting with 0
    pub offset: usize,
}

impl Default for Location {
    /// Default to line 1, column 1
    fn default() -> Self {
        Self::new(1, 1, 0)
    }
}

impl Location {
    /// Create a new Location
    #[must_use]
    pub fn new(line: usize, column: usize, offset: usize) -> Self {
        Self { line, column, offset }
    }
}

impl Display for Location {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "({}:{})", self.line, self.column)
    }
}

impl Debug for Location {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "({}:{})", self.line, self.column)
    }
}

/// Decode one UTF-16 code unit, calling `next_cu` to fetch the following code unit when a
/// surrogate pair is encountered. Returns `(Character, bytes_consumed)`.
fn decode_utf16_char(cu: u16, next_cu: impl FnOnce() -> Option<u16>) -> (Character, usize) {
    match cu {
        0xD800..=0xDBFF => {
            // High surrogate — must be followed by a low surrogate
            match next_cu() {
                Some(low @ 0xDC00..=0xDFFF) => {
                    let codepoint = 0x10000u32 + ((u32::from(cu) - 0xD800) << 10) + (u32::from(low) - 0xDC00);
                    (char::from_u32(codepoint).map_or(Ch(REPLACEMENT_CHARACTER), Ch), 4)
                }
                _ => (Surrogate(cu), 2), // unpaired high surrogate
            }
        }
        0xDC00..=0xDFFF => (Surrogate(cu), 2), // lone low surrogate
        _ => (char::from_u32(u32::from(cu)).map_or(Ch(REPLACEMENT_CHARACTER), Ch), 2),
    }
}

#[cfg(test)]
mod test {
    use super::*;

    // ── from_str constructor ──────────────────────────────────────────────────

    #[test]
    fn test_from_str() {
        let mut stream = ByteStream::from_str("hello", Encoding::UTF8);
        assert!(stream.closed());
        assert!(!stream.eof());
        assert_eq!(stream.read_and_next(), Ch('h'));
        assert_eq!(stream.read_and_next(), Ch('e'));
        assert_eq!(stream.read_and_next(), Ch('l'));
        assert_eq!(stream.read_and_next(), Ch('l'));
        assert_eq!(stream.read_and_next(), Ch('o'));
        assert!(matches!(stream.read_and_next(), StreamEnd));
        assert!(stream.eof());
    }

    // ── look_ahead ────────────────────────────────────────────────────────────

    #[test]
    fn test_look_ahead() {
        let mut stream = ByteStream::from_str("abcd", Encoding::UTF8);
        assert_eq!(stream.look_ahead(0), Ch('a'));
        assert_eq!(stream.look_ahead(1), Ch('b'));
        assert_eq!(stream.look_ahead(3), Ch('d'));
        assert_eq!(stream.look_ahead(4), StreamEnd);
        assert_eq!(stream.look_ahead(99), StreamEnd);
        // position must not have moved
        assert_eq!(stream.read_and_next(), Ch('a'));
        stream.next();
        assert_eq!(stream.look_ahead(0), Ch('c'));
        assert_eq!(stream.look_ahead(1), Ch('d'));
    }

    // ── mark / reset_to_mark ─────────────────────────────────────────────────

    #[test]
    fn test_mark_reset() {
        let mut stream = ByteStream::from_str("abcde", Encoding::UTF8);
        assert_eq!(stream.read_and_next(), Ch('a'));
        assert_eq!(stream.read_and_next(), Ch('b'));
        let mark = stream.mark();
        assert_eq!(stream.read_and_next(), Ch('c'));
        assert_eq!(stream.read_and_next(), Ch('d'));
        stream.reset_to_mark(mark);
        assert_eq!(stream.read_and_next(), Ch('c'));
        assert_eq!(stream.read_and_next(), Ch('d'));
        assert_eq!(stream.read_and_next(), Ch('e'));
        assert!(matches!(stream.read_and_next(), StreamEnd));
    }

    #[test]
    fn test_mark_preserves_location() {
        let mut stream = ByteStream::from_str("ab\ncd", Encoding::UTF8);
        stream.read_and_next(); // 'a' → (1,2)
        stream.read_and_next(); // 'b' → (1,3)
        stream.read_and_next(); // '\n' → (2,1)
        let mark = stream.mark();
        assert_eq!(stream.location(), Location::new(2, 1, 3));
        stream.read_and_next(); // 'c' → (2,2)
        stream.read_and_next(); // 'd' → (2,3)
        stream.reset_to_mark(mark);
        assert_eq!(stream.location(), Location::new(2, 1, 3));
        assert_eq!(stream.read_and_next(), Ch('c'));
    }

    // ── get_slice ────────────────────────────────────────────────────────────

    #[test]
    fn test_get_slice() {
        let mut stream = ByteStream::from_str("abcde", Encoding::UTF8);
        stream.next(); // skip 'a', now at 'b'
        let slice = stream.get_slice(3);
        assert_eq!(Character::slice_to_string(slice), "bcd");
        // position must not have advanced
        assert_eq!(stream.read_and_next(), Ch('b'));
    }

    #[test]
    fn test_get_slice_past_end() {
        let mut stream = ByteStream::from_str("ab", Encoding::UTF8);
        let slice = stream.get_slice(5);
        // returns as many as available, pads with StreamEnd
        let chars: Vec<_> = slice.into_iter().filter(|c| matches!(c, Ch(_))).collect();
        assert_eq!(chars, vec![Ch('a'), Ch('b')]);
        // position unchanged
        assert_eq!(stream.read(), Ch('a'));
    }

    // ── location tracking ────────────────────────────────────────────────────

    #[test]
    fn test_location_basic() {
        let mut stream = ByteStream::from_str("ab\ncd\ne", Encoding::UTF8);
        assert_eq!(stream.location(), Location::new(1, 1, 0));
        stream.read_and_next(); // 'a'
        assert_eq!(stream.location(), Location::new(1, 2, 1));
        stream.read_and_next(); // 'b'
        assert_eq!(stream.location(), Location::new(1, 3, 2));
        stream.read_and_next(); // '\n'
        assert_eq!(stream.location(), Location::new(2, 1, 3));
        stream.read_and_next(); // 'c'
        assert_eq!(stream.location(), Location::new(2, 2, 4));
        stream.read_and_next(); // 'd'
        assert_eq!(stream.location(), Location::new(2, 3, 5));
        stream.read_and_next(); // '\n'
        assert_eq!(stream.location(), Location::new(3, 1, 6));
        stream.read_and_next(); // 'e'
        assert_eq!(stream.location(), Location::new(3, 2, 7));
    }

    #[test]
    fn test_location_prev() {
        let mut stream = ByteStream::from_str("ab\nc", Encoding::UTF8);
        stream.read_and_next(); // 'a'
        stream.read_and_next(); // 'b'
        stream.read_and_next(); // '\n' → line 2 col 1
        stream.read_and_next(); // 'c' → line 2 col 2
        assert_eq!(stream.location(), Location::new(2, 2, 4));
        stream.prev(); // back to 'c' start → (2,1)
        assert_eq!(stream.location(), Location::new(2, 1, 3));
        stream.prev(); // back across '\n' → (1,3)
        assert_eq!(stream.location(), Location::new(1, 3, 2));
        stream.prev(); // back to 'a' → (1,2)
        assert_eq!(stream.location(), Location::new(1, 2, 1));
    }

    // ── replace_cr_as_lf config ───────────────────────────────────────────────

    #[test]
    fn test_replace_cr_as_lf() {
        let mut stream = ByteStream::new(
            Encoding::UTF8,
            Some(Config {
                cr_lf_as_one: false,
                replace_cr_as_lf: true,
                replace_high_ascii: false,
            }),
        );
        stream.read_from_str("a\rb\r\nc", Some(Encoding::UTF8));
        stream.close();
        // standalone CR → LF; CR+LF pair: CR is replaced but LF follows
        assert_eq!(stream.read_and_next(), Ch('a'));
        assert_eq!(stream.read_and_next(), Ch('\n')); // CR → LF
        assert_eq!(stream.read_and_next(), Ch('b'));
        assert_eq!(stream.read_and_next(), Ch('\r')); // CR before LF not replaced (next is LF)
        assert_eq!(stream.read_and_next(), Ch('\n'));
        assert_eq!(stream.read_and_next(), Ch('c'));
    }

    // ── UTF-16 LE ─────────────────────────────────────────────────────────────

    #[test]
    fn test_utf16le_basic() {
        // "Hi" in UTF-16 LE: H=0x48,0x00  i=0x69,0x00
        let mut stream = ByteStream::new(Encoding::UTF16LE, None);
        stream.read_from_bytes(&[0x48, 0x00, 0x69, 0x00]).unwrap();
        assert_eq!(stream.read_and_next(), Ch('H'));
        assert_eq!(stream.read_and_next(), Ch('i'));
        assert!(matches!(stream.read_and_next(), StreamEnd));
    }

    // ── UTF-16 surrogate pairs ───────────────────────────────────────────────

    #[test]
    fn test_utf16_surrogate_pair() {
        // U+1F600 😀 as UTF-16 BE surrogate pair: 0xD83D 0xDE00
        let mut stream = ByteStream::new(Encoding::UTF16BE, None);
        stream.read_from_bytes(&[0xD8, 0x3D, 0xDE, 0x00]).unwrap();
        assert_eq!(stream.read_and_next(), Ch('😀'));
        assert!(matches!(stream.read_and_next(), StreamEnd));
    }

    #[test]
    fn test_utf16_lone_surrogate() {
        // An unpaired high surrogate: 0xD800 with no following low surrogate
        let mut stream = ByteStream::new(Encoding::UTF16BE, None);
        stream.read_from_bytes(&[0xD8, 0x00, 0x00, 0x41]).unwrap(); // lone D800 then 'A'
        assert!(matches!(stream.read_and_next(), Surrogate(0xD800)));
        assert_eq!(stream.read_and_next(), Ch('A'));
    }

    // ── detect_encoding ──────────────────────────────────────────────────────

    #[test]
    fn test_detect_encoding_utf8_bom() {
        let mut stream = ByteStream::new(Encoding::Unknown, None);
        stream.read_from_bytes(&[0xEF, 0xBB, 0xBF, 0x41]).unwrap(); // UTF-8 BOM + 'A'
        assert_eq!(stream.detect_encoding(), Encoding::UTF8);
    }

    #[test]
    fn test_detect_encoding_utf16le_bom() {
        let mut stream = ByteStream::new(Encoding::Unknown, None);
        stream.read_from_bytes(&[0xFF, 0xFE, 0x41, 0x00]).unwrap();
        assert_eq!(stream.detect_encoding(), Encoding::UTF16LE);
    }

    #[test]
    fn test_detect_encoding_utf16be_bom() {
        let mut stream = ByteStream::new(Encoding::Unknown, None);
        stream.read_from_bytes(&[0xFE, 0xFF, 0x00, 0x41]).unwrap();
        assert_eq!(stream.detect_encoding(), Encoding::UTF16BE);
    }

    // ── Unknown encoding ─────────────────────────────────────────────────────

    #[test]
    fn test_unknown_encoding_reads_nothing() {
        let mut stream = ByteStream::new(Encoding::Unknown, None);
        stream.read_from_bytes(b"hello").unwrap();
        // Unknown encoding: decode_buffer does nothing, chars stays empty
        assert_eq!(stream.read(), StreamEnd);
        assert!(stream.exhausted());
    }

    // ── Latin-1 high bytes ───────────────────────────────────────────────────

    #[test]
    fn test_latin1_high_bytes() {
        // Without replace_high_ascii: bytes 0xE9 (é) and 0xFC (ü) pass through as char
        let mut stream = ByteStream::new(
            Encoding::Latin1,
            Some(Config {
                cr_lf_as_one: false,
                replace_cr_as_lf: false,
                replace_high_ascii: false,
            }),
        );
        stream.read_from_bytes(&[0x41, 0xE9, 0xFC]).unwrap(); // A, é, ü
        assert_eq!(stream.read_and_next(), Ch('A'));
        assert_eq!(stream.read_and_next(), Ch('é'));
        assert_eq!(stream.read_and_next(), Ch('ü'));
    }

    #[test]
    fn test_latin1_replace_high_ascii() {
        let mut stream = ByteStream::new(
            Encoding::Latin1,
            Some(Config {
                cr_lf_as_one: false,
                replace_cr_as_lf: false,
                replace_high_ascii: true,
            }),
        );
        stream.read_from_bytes(&[0x41, 0xE9, 0x42]).unwrap(); // A, é (replaced), B
        assert_eq!(stream.read_and_next(), Ch('A'));
        assert_eq!(stream.read_and_next(), Ch('?'));
        assert_eq!(stream.read_and_next(), Ch('B'));
    }

    // ── tell_bytes / seek_bytes with multi-byte UTF-8 ─────────────────────────

    #[test]
    fn test_tell_bytes_utf8() {
        // "aé" = 0x61 0xC3 0xA9 (3 bytes for 2 chars)
        let mut stream = ByteStream::from_str("aéb", Encoding::UTF8);
        assert_eq!(stream.tell_bytes(), 0);
        stream.read_and_next(); // 'a' — 1 byte
        assert_eq!(stream.tell_bytes(), 1);
        stream.read_and_next(); // 'é' — 2 bytes
        assert_eq!(stream.tell_bytes(), 3);
        stream.read_and_next(); // 'b' — 1 byte
        assert_eq!(stream.tell_bytes(), 4);
    }

    #[test]
    fn test_seek_bytes_utf8() {
        let mut stream = ByteStream::from_str("aéb", Encoding::UTF8);
        stream.seek_bytes(3); // byte offset 3 = start of 'b'
        assert_eq!(stream.read_and_next(), Ch('b'));
        stream.seek_bytes(1); // byte offset 1 = start of 'é'
        assert_eq!(stream.read_and_next(), Ch('é'));
    }

    #[test]
    fn test_seek_bytes_updates_location() {
        // "ab\ncd" — after seeking to byte 3 ('c') location should be (2,1)
        let mut stream = ByteStream::from_str("ab\ncd", Encoding::UTF8);
        stream.seek_bytes(3);
        assert_eq!(stream.location(), Location::new(2, 1, 3));
        assert_eq!(stream.read(), Ch('c'));
    }

    #[test]
    fn test_stream() {
        let mut stream = ByteStream::new(
            Encoding::UTF8,
            Some(Config {
                cr_lf_as_one: true,
                replace_cr_as_lf: false,
                replace_high_ascii: true,
            }),
        );
        assert!(stream.exhausted());
        assert!(!stream.eof());

        stream.read_from_str("foo", Some(Encoding::Latin1));
        stream.close();
        assert!(!stream.eof());

        stream.read_from_str("f👽f", Some(Encoding::UTF8));
        stream.close();
        assert!(!stream.eof());
        assert_eq!(stream.read_and_next(), Ch('f'));
        assert!(!stream.eof());
        assert_eq!(stream.read_and_next(), Ch('👽'));
        assert!(!stream.eof());
        assert_eq!(stream.read_and_next(), Ch('f'));
        assert!(stream.eof());

        stream.reset_stream();
        stream.set_encoding(Encoding::Latin1);
        assert_eq!(stream.read_and_next(), Ch('f'));
        assert_eq!(stream.read_and_next(), Ch('?'));
        assert_eq!(stream.read_and_next(), Ch('?'));
        assert_eq!(stream.read_and_next(), Ch('?'));
        assert_eq!(stream.read_and_next(), Ch('?'));
        assert_eq!(stream.read_and_next(), Ch('f'));
        assert!(matches!(stream.read_and_next(), StreamEnd));
        assert!(matches!(stream.read_and_next(), StreamEnd));
        assert!(matches!(stream.read_and_next(), StreamEnd));

        stream.prev(); // unread 'f'
        stream.prev(); // Unread '?'
        stream.prev(); // Unread '?'
        assert_eq!(stream.read_and_next(), Ch('?'));
        assert_eq!(stream.read_and_next(), Ch('?'));
        assert_eq!(stream.read_and_next(), Ch('f'));
        assert!(matches!(stream.read_and_next(), StreamEnd));

        stream.reset_stream();
        stream.prev();
        assert_eq!(stream.read_and_next(), Ch('f'));
        stream.prev_n(4);
        assert_eq!(stream.read_and_next(), Ch('f'));
        assert_eq!(stream.read_and_next(), Ch('?'));
        stream.prev_n(4);
        assert_eq!(stream.read_and_next(), Ch('f'));

        stream.read_from_str("abc", Some(Encoding::UTF8));
        stream.reset_stream();
        assert_eq!(stream.read_and_next(), Ch('a'));
        stream.prev();
        assert_eq!(stream.read_and_next(), Ch('a'));
        assert_eq!(stream.read_and_next(), Ch('b'));
        stream.prev();
        assert_eq!(stream.read_and_next(), Ch('b'));
        assert_eq!(stream.read_and_next(), Ch('c'));
        stream.prev();
        assert_eq!(stream.read_and_next(), Ch('c'));
        assert!(matches!(stream.read_and_next(), StreamEnd));
        stream.prev();
        assert_eq!(stream.read_and_next(), Ch('c'));
    }

    #[test]
    fn test_eof() {
        let mut stream = ByteStream::new(Encoding::UTF8, None);
        stream.read_from_str("abc", Some(Encoding::UTF8));
        stream.close();
        assert_eq!(stream.chars_left(), 3);
        assert_eq!(stream.read_and_next(), Ch('a'));
        assert_eq!(stream.read_and_next(), Ch('b'));
        assert_eq!(stream.read_and_next(), Ch('c'));
        assert!(matches!(stream.read_and_next(), StreamEnd));
        assert!(matches!(stream.read_and_next(), StreamEnd));
        assert!(matches!(stream.read_and_next(), StreamEnd));
        assert!(matches!(stream.read_and_next(), StreamEnd));
        stream.prev();
        assert_eq!(stream.read_and_next(), Ch('c'));
        assert!(matches!(stream.read_and_next(), StreamEnd));
        stream.prev();
        stream.prev();
        assert_eq!(stream.read_and_next(), Ch('b'));
        assert_eq!(stream.read_and_next(), Ch('c'));
        assert!(matches!(stream.read_and_next(), StreamEnd));
        stream.prev();
        stream.prev();
        assert!(!matches!(stream.read_and_next(), StreamEnd));
        stream.prev();
        stream.prev();
        stream.prev();
        assert_eq!(stream.read_and_next(), Ch('a'));
        stream.prev();
        assert_eq!(stream.read_and_next(), Ch('a'));
        stream.prev();
        stream.prev();
        assert_eq!(stream.read_and_next(), Ch('a'));
        stream.prev();
        stream.prev();
        stream.prev();
        stream.prev();
        stream.prev();
        stream.prev();
        assert_eq!(stream.read_and_next(), Ch('a'));
        assert_eq!(stream.read_and_next(), Ch('b'));
        assert_eq!(stream.read_and_next(), Ch('c'));
        assert!(matches!(stream.read_and_next(), StreamEnd));
        stream.prev();
        assert_eq!(stream.read_and_next(), Ch('c'));
        assert!(matches!(stream.read_and_next(), StreamEnd));
        stream.prev();
        assert_eq!(stream.read_and_next(), Ch('c'));
    }

    #[test]
    fn stream_closing() {
        let mut stream = ByteStream::new(Encoding::UTF8, None);
        stream.read_from_str("abc", Some(Encoding::UTF8));
        assert_eq!(stream.read_and_next(), Ch('a'));
        assert_eq!(stream.read_and_next(), Ch('b'));
        assert_eq!(stream.read_and_next(), Ch('c'));
        assert!(matches!(stream.read_and_next(), StreamEnd));
        assert!(matches!(stream.read_and_next(), StreamEnd));

        stream.append_str("def");
        assert_eq!(stream.read_and_next(), Ch('d'));
        assert_eq!(stream.read_and_next(), Ch('e'));
        assert_eq!(stream.read_and_next(), Ch('f'));
        assert!(matches!(stream.read_and_next(), StreamEnd));

        stream.append_str("ghi");
        stream.close();
        assert_eq!(stream.read_and_next(), Ch('g'));
        assert_eq!(stream.read_and_next(), Ch('h'));
        assert_eq!(stream.read_and_next(), Ch('i'));
        assert!(matches!(stream.read_and_next(), StreamEnd));
        assert!(matches!(stream.read_and_next(), StreamEnd));
    }

    #[test]
    fn append_char_by_char_matches_full_decode() {
        // Exercises incremental decode + incremental line_starts, including CR/LF and
        // multibyte characters landing on append boundaries. Feeding the text one char
        // at a time must produce the exact same chars and line/column/offset as decoding
        // the whole thing at once.
        let text = "a\r\nb\nc\rd\r\n\r\ne_é_😀_f\ng\r";

        let mut full = ByteStream::from_str(text, Encoding::UTF8);

        let mut inc = ByteStream::new(Encoding::UTF8, None);
        let mut buf = [0u8; 4];
        for ch in text.chars() {
            inc.append_str(ch.encode_utf8(&mut buf));
        }
        inc.close();

        let loc = |s: &ByteStream| (s.location().line, s.location().column, s.location().offset);
        loop {
            assert_eq!(full.read(), inc.read(), "char mismatch at offset {}", full.tell_bytes());
            assert_eq!(
                loc(&full),
                loc(&inc),
                "location mismatch at offset {}",
                full.tell_bytes()
            );
            if full.eof() {
                break;
            }
            full.next();
            inc.next();
        }
    }

    #[test]
    fn advance() {
        let mut stream = ByteStream::new(Encoding::UTF8, None);
        stream.read_from_str("abc", Some(Encoding::UTF8));
        stream.close();
        assert_eq!(stream.read(), Ch('a'));
        assert_eq!(stream.read(), Ch('a'));
        assert_eq!(stream.read(), Ch('a'));
        stream.next();
        assert_eq!(stream.read(), Ch('b'));
        stream.next();
        assert_eq!(stream.read(), Ch('c'));
        stream.next();
        assert_eq!(stream.read(), StreamEnd);

        stream.prev_n(10);
        assert_eq!(stream.read(), Ch('a'));
        stream.next_n(2);
        assert_eq!(stream.read(), Ch('c'));
    }

    #[test]
    fn test_prev_with_utf8() {
        let mut stream = ByteStream::new(
            Encoding::UTF8,
            Some(Config {
                cr_lf_as_one: true,
                replace_cr_as_lf: false,
                replace_high_ascii: true,
            }),
        );
        stream.read_from_str("a👽b", Some(Encoding::UTF8));
        stream.close();

        assert_eq!(stream.read_and_next(), Ch('a'));
        assert_eq!(stream.read_and_next(), Ch('👽'));
        assert_eq!(stream.read_and_next(), Ch('b'));
        assert_eq!(stream.read_and_next(), StreamEnd);
        stream.prev();
        assert_eq!(stream.read_and_next(), Ch('b'));
        stream.prev_n(2);
        assert_eq!(stream.read_and_next(), Ch('👽'));
        stream.prev_n(3);
        assert_eq!(stream.read_and_next(), Ch('a'));
    }

    #[test]
    fn test_switch_encoding() {
        let mut stream = ByteStream::new(
            Encoding::UTF8,
            Some(Config {
                cr_lf_as_one: true,
                replace_cr_as_lf: false,
                replace_high_ascii: true,
            }),
        );
        stream.read_from_str("a👽b", Some(Encoding::UTF8));
        stream.close();

        stream.set_encoding(Encoding::Latin1);
        stream.seek_bytes(3);
        assert_eq!(stream.read_and_next(), Ch('?'));
        assert_eq!(stream.read_and_next(), Ch('?'));
        assert_eq!(stream.read_and_next(), Ch('b'));
    }

    #[test]
    fn test_character() {
        let ch = Ch('a');
        assert_eq!(char::from(&ch), 'a');
        assert_eq!(char::from(ch), 'a');
        assert_eq!(format!("{ch}"), "a");

        let ch = Surrogate(0xDFA9);
        assert_eq!(format!("{ch}"), "U+DFA9");
        assert!(!ch.is_numeric());
        assert!(!ch.is_whitespace());

        let ch = Ch('0');
        assert!(ch.is_numeric());
        let ch = Ch('b');
        assert!(!ch.is_numeric());
        let ch = Ch(' ');
        assert!(ch.is_whitespace());
        let ch = Ch('\n');
        assert!(ch.is_whitespace());
        let ch = Ch('\t');
        assert!(ch.is_whitespace());
    }

    #[test]
    fn test_slice() {
        let v = vec![Ch('a'), Ch('b'), Ch('c'), Ch('d'), Ch('e')];

        assert_eq!(Character::slice_to_string(v), "abcde");
    }

    #[test]
    fn test_utf16le() {
        let mut stream = ByteStream::new(
            Encoding::UTF16BE,
            Some(Config {
                cr_lf_as_one: true,
                replace_cr_as_lf: false,
                replace_high_ascii: false,
            }),
        );

        // Quizdeltagerne spiste jordbær med fløde, mens cirkusklovnen Wolther spillede på xylofon.
        let _ = stream.read_from_bytes(&[
            0x00, 0x51, 0x00, 0x75, 0x00, 0x69, 0x00, 0x7a, 0x00, 0x64, 0x00, 0x65, 0x00, 0x6c, 0x00, 0x74, 0x00, 0x61,
            0x00, 0x67, 0x00, 0x65, 0x00, 0x72, 0x00, 0x6e, 0x00, 0x65, 0x00, 0x20, 0x00, 0x73, 0x00, 0x70, 0x00, 0x69,
            0x00, 0x73, 0x00, 0x74, 0x00, 0x65, 0x00, 0x20, 0x00, 0x6a, 0x00, 0x6f, 0x00, 0x72, 0x00, 0x64, 0x00, 0x62,
            0x00, 0xe6, 0x00, 0x72, 0x00, 0x20, 0x00, 0x6d, 0x00, 0x65, 0x00, 0x64, 0x00, 0x20, 0x00, 0x66, 0x00, 0x6c,
            0x00, 0xf8, 0x00, 0x64, 0x00, 0x65, 0x00, 0x2c, 0x00, 0x20, 0x00, 0x6d, 0x00, 0x65, 0x00, 0x6e, 0x00, 0x73,
            0x00, 0x20, 0x00, 0x63, 0x00, 0x69, 0x00, 0x72, 0x00, 0x6b, 0x00, 0x75, 0x00, 0x73, 0x00, 0x6b, 0x00, 0x6c,
            0x00, 0x6f, 0x00, 0x76, 0x00, 0x6e, 0x00, 0x65, 0x00, 0x6e, 0x00, 0x20, 0x00, 0x57, 0x00, 0x6f, 0x00, 0x6c,
            0x00, 0x74, 0x00, 0x68, 0x00, 0x65, 0x00, 0x72, 0x00, 0x20, 0x00, 0x73, 0x00, 0x70, 0x00, 0x69, 0x00, 0x6c,
            0x00, 0x6c, 0x00, 0x65, 0x00, 0x64, 0x00, 0x65, 0x00, 0x20, 0x00, 0x70, 0x00, 0xe5, 0x00, 0x20, 0x00, 0x78,
            0x00, 0x79, 0x00, 0x6c, 0x00, 0x6f, 0x00, 0x66, 0x00, 0x6f, 0x00, 0x6e, 0x00, 0x2e,
        ]);
        stream.close();

        assert_eq!(stream.read_and_next(), Ch('Q'));
        assert_eq!(stream.read_and_next(), Ch('u'));
        assert_eq!(stream.read_and_next(), Ch('i'));
        assert_eq!(stream.read_and_next(), Ch('z'));

        // Byte offsets address the decoded text (UTF-8 space), not the raw UTF-16
        // input: "Quizdeltagerne spiste jor" is 25 one-byte chars, so 'd' starts at 25.
        stream.seek_bytes(25);
        assert_eq!(stream.read_and_next(), Ch('d'));
        assert_eq!(stream.read_and_next(), Ch('b'));
        assert_eq!(stream.read_and_next(), Ch('æ'));
        assert_eq!(stream.read_and_next(), Ch('r'));
        assert_eq!(stream.read_and_next(), Ch(' '));

        stream.prev_n(4);
        assert_eq!(stream.read_and_next(), Ch('b'));
        assert_eq!(stream.read_and_next(), Ch('æ'));
        assert_eq!(stream.read_and_next(), Ch('r'));

        // Now do UTF on the same bytestream
        stream.reset_stream();
        stream.set_encoding(Encoding::UTF8);
        assert_eq!(stream.read_and_next(), Ch('\0'));
        assert_eq!(stream.read_and_next(), Ch('Q'));
        assert_eq!(stream.read_and_next(), Ch('\0'));
        assert_eq!(stream.read_and_next(), Ch('u'));
    }

    #[test]
    fn test_crlf() {
        let mut stream = ByteStream::new(
            Encoding::UTF8,
            Some(Config {
                cr_lf_as_one: true,
                replace_cr_as_lf: false,
                replace_high_ascii: false,
            }),
        );
        stream.read_from_str("a\r\nb\nc\r\nd\r\r\n\ne", Some(Encoding::UTF8));
        stream.close();

        assert_eq!(stream.read_and_next(), Ch('a'));
        assert_eq!(stream.read_and_next(), Ch('\n'));
        assert_eq!(stream.read_and_next(), Ch('b'));
        assert_eq!(stream.read_and_next(), Ch('\n'));
        assert_eq!(stream.read_and_next(), Ch('c'));

        stream.prev_n(2);
        assert_eq!(stream.read_and_next(), Ch('\n'));
        assert_eq!(stream.read_and_next(), Ch('c'));

        stream.prev_n(4);
        assert_eq!(stream.read_and_next(), Ch('\n'));
        assert_eq!(stream.read_and_next(), Ch('b'));
        assert_eq!(stream.read_and_next(), Ch('\n'));
        assert_eq!(stream.read_and_next(), Ch('c'));

        assert_eq!(stream.read_and_next(), Ch('\n'));
        assert_eq!(stream.read_and_next(), Ch('d'));
        assert_eq!(stream.read_and_next(), Ch('\r'));
        assert_eq!(stream.read_and_next(), Ch('\n'));
        assert_eq!(stream.read_and_next(), Ch('\n'));
        assert_eq!(stream.read_and_next(), Ch('e'));
        assert!(matches!(stream.read_and_next(), StreamEnd));

        stream.prev_n(4);
        assert_eq!(stream.read_and_next(), Ch('\r'));
        stream.prev_n(2);
        assert_eq!(stream.read_and_next(), Ch('d'));
        assert_eq!(stream.read_and_next(), Ch('\r'));
        stream.prev_n(4);
        assert_eq!(stream.read_and_next(), Ch('c'));
    }

    #[test]
    fn test_prev_across_crlf_pair() {
        // read_and_next() on \r\n (with cr_lf_as_one) advances char_pos by 2.
        // prev() must step back over both so the next read_and_next returns the
        // same LF again, not get stuck on the bare \n.
        let mut stream = ByteStream::new(
            Encoding::UTF8,
            Some(Config {
                cr_lf_as_one: true,
                replace_cr_as_lf: false,
                replace_high_ascii: false,
            }),
        );
        stream.read_from_str("a\r\nb", Some(Encoding::UTF8));
        stream.close();

        assert_eq!(stream.read_and_next(), Ch('a'));
        assert_eq!(stream.read_and_next(), Ch('\n')); // CR+LF collapsed
        stream.prev(); // back over the newline
        assert_eq!(stream.read_and_next(), Ch('\n')); // must get the same newline again
        assert_eq!(stream.read_and_next(), Ch('b'));
    }

    // ── regression tests ─────────────────────────────────────────────────────

    #[test]
    fn test_read_from_file_replaces_buffer() {
        use std::io::Cursor;
        let mut stream = ByteStream::new(Encoding::UTF8, None);
        stream.read_from_str("hello", Some(Encoding::UTF8));
        stream.read_from_file(Cursor::new(b"world")).unwrap();
        stream.reset_stream();
        assert_eq!(stream.read_and_next(), Ch('w'));
    }

    #[test]
    fn test_next_updates_location() {
        let mut stream = ByteStream::from_str("abc", Encoding::UTF8);
        stream.next(); // skip 'a'
        stream.next(); // skip 'b'
        assert_eq!(stream.location(), Location::new(1, 3, 2));
    }

    #[test]
    fn test_location_offset_is_byte_count() {
        // 'é' = U+00E9 = 2 bytes in UTF-8; after reading it offset should be 2.
        let mut stream = ByteStream::from_str("é", Encoding::UTF8);
        stream.read_and_next();
        assert_eq!(stream.location().offset, 2);
    }
}
