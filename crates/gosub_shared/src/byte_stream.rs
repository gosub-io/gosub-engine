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
    /// Raw bytes of the source data
    buffer: Vec<u8>,
    /// Pre-decoded characters (filled by decode_buffer)
    chars: Vec<Character>,
    /// Byte offset of each decoded character in `buffer`
    char_byte_offsets: Vec<usize>,
    /// Current position in the decoded chars array
    char_pos: usize,
    /// True when the stream is closed (no more data will be added)
    closed: bool,
    /// Current encoding
    encoding: Encoding,
    /// Configuration for the stream
    config: Config,
    /// Current line/column/offset position
    cur_location: Location,
    /// Column widths of previous lines, used to restore column on dec()
    column_stack: Vec<usize>,
}

/// Opaque snapshot of stream position and location for mark/reset.
pub struct StreamMark {
    char_pos: usize,
    location: Location,
    column_stack: Vec<usize>,
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
    fn read(&self) -> Character {
        if self.char_pos < self.chars.len() {
            self.chars[self.char_pos]
        } else {
            StreamEnd
        }
    }

    fn read_and_next(&mut self) -> Character {
        let ch = self.read();
        if matches!(ch, StreamEnd) {
            return ch;
        }
        self.char_pos += 1;

        let returned = if self.config.cr_lf_as_one && ch == Ch(CHAR_CR) && self.read() == Ch(CHAR_LF) {
            self.char_pos += 1;
            Ch(CHAR_LF)
        } else if self.config.replace_cr_as_lf && ch == Ch(CHAR_CR) && self.read() != Ch(CHAR_LF) {
            Ch(CHAR_LF)
        } else {
            ch
        };

        self.loc_inc(returned);
        returned
    }

    fn look_ahead(&self, offset: usize) -> Character {
        let pos = self.char_pos + offset;
        if pos < self.chars.len() {
            self.chars[pos]
        } else {
            StreamEnd
        }
    }

    fn next(&mut self) {
        self.next_n(1);
    }

    fn next_n(&mut self, offset: usize) {
        self.char_pos = (self.char_pos + offset).min(self.chars.len());
    }

    fn prev(&mut self) {
        self.prev_n(1);
    }

    fn prev_n(&mut self, n: usize) {
        for _ in 0..n {
            self.char_pos = self.char_pos.saturating_sub(1);
            if self.config.cr_lf_as_one && self.read() == Ch(CHAR_CR) && self.look_ahead(1) == Ch(CHAR_LF) {
                self.char_pos = self.char_pos.saturating_sub(1);
            }
            self.loc_dec();
        }
    }

    fn seek_bytes(&mut self, offset: usize) {
        let pos = self.char_byte_offsets.partition_point(|&b| b < offset);
        self.char_pos = pos.min(self.chars.len());
    }

    fn tell_bytes(&self) -> usize {
        self.char_byte_offsets.get(self.char_pos).copied().unwrap_or(self.buffer.len())
    }

    fn get_slice(&mut self, len: usize) -> Vec<Character> {
        let mark = self.mark();
        let mut slice = Vec::with_capacity(len);
        for _ in 0..len {
            slice.push(self.read_and_next());
        }
        self.reset_to_mark(mark);
        slice
    }

    fn reset_stream(&mut self) {
        self.char_pos = 0;
        self.cur_location = Location::default();
        self.column_stack.clear();
    }

    fn close(&mut self) {
        self.closed = true;
    }

    fn closed(&self) -> bool {
        self.closed
    }

    fn exhausted(&self) -> bool {
        self.char_pos >= self.chars.len()
    }

    fn eof(&self) -> bool {
        self.closed() && self.exhausted()
    }

    fn location(&self) -> Location {
        self.cur_location
    }
}

impl ByteStream {
    #[must_use]
    pub fn new(encoding: Encoding, config: Option<Config>) -> Self {
        Self {
            config: config.unwrap_or_default(),
            char_pos: 0,
            buffer: Vec::new(),
            chars: Vec::new(),
            char_byte_offsets: Vec::new(),
            closed: false,
            encoding,
            cur_location: Location::default(),
            column_stack: Vec::new(),
        }
    }

    /// Create a stream from a string, fully decoded and closed, ready for parsing.
    pub fn from_str(s: &str, encoding: Encoding) -> Self {
        let mut stream = Self::new(encoding, None);
        stream.read_from_str(s, None);
        stream.close();
        stream
    }

    /// Take a snapshot of the current position and location for later restoration.
    pub fn mark(&self) -> StreamMark {
        StreamMark {
            char_pos: self.char_pos,
            location: self.cur_location,
            column_stack: self.column_stack.clone(),
        }
    }

    /// Restore position and location to a previously saved mark.
    pub fn reset_to_mark(&mut self, mark: StreamMark) {
        self.char_pos = mark.char_pos;
        self.cur_location = mark.location;
        self.column_stack = mark.column_stack;
    }

    fn loc_inc(&mut self, ch: Character) {
        match ch {
            Ch(CHAR_LF) => {
                self.column_stack.push(self.cur_location.column);
                self.cur_location.line += 1;
                self.cur_location.column = 1;
                self.cur_location.offset += 1;
            }
            Ch(_) => {
                self.cur_location.column += 1;
                self.cur_location.offset += 1;
            }
            _ => {}
        }
    }

    fn loc_dec(&mut self) {
        let loc = self.cur_location;
        if loc.column > 1 {
            self.cur_location = Location::new(loc.line, loc.column - 1, loc.offset - 1);
        } else if loc.line > 1 {
            let prev_col = self.column_stack.pop().unwrap_or(1);
            self.cur_location = Location::new(loc.line - 1, prev_col, loc.offset - 1);
        }
    }

    /// Decode `self.buffer` into `self.chars` and `self.char_byte_offsets` using the current encoding.
    fn decode_buffer(&mut self) {
        self.chars.clear();
        self.char_byte_offsets.clear();

        match self.encoding {
            Encoding::Unknown => {}
            Encoding::Latin1 => {
                for (i, &byte) in self.buffer.iter().enumerate() {
                    self.char_byte_offsets.push(i);
                    if self.config.replace_high_ascii && byte > 127 {
                        self.chars.push(Ch('?'));
                    } else {
                        self.chars.push(Ch(byte as char));
                    }
                }
            }
            Encoding::UTF8 => {
                let mut byte_pos = 0;
                while byte_pos < self.buffer.len() {
                    let (ch, len) = decode_one_utf8(&self.buffer[byte_pos..], self.closed);
                    if len == 0 {
                        break; // incomplete sequence at end of open stream — stop
                    }
                    self.char_byte_offsets.push(byte_pos);
                    self.chars.push(ch);
                    byte_pos += len;
                }
            }
            Encoding::UTF16LE => {
                let mut byte_pos = 0;
                while byte_pos + 2 <= self.buffer.len() {
                    let cu = u16::from_le_bytes([self.buffer[byte_pos], self.buffer[byte_pos + 1]]);
                    let next = if byte_pos + 4 <= self.buffer.len() {
                        Some(u16::from_le_bytes([self.buffer[byte_pos + 2], self.buffer[byte_pos + 3]]))
                    } else {
                        None
                    };
                    let (ch, len) = decode_utf16_char(cu, || next);
                    self.char_byte_offsets.push(byte_pos);
                    self.chars.push(ch);
                    byte_pos += len;
                }
            }
            Encoding::UTF16BE => {
                let mut byte_pos = 0;
                while byte_pos + 2 <= self.buffer.len() {
                    let cu = u16::from_be_bytes([self.buffer[byte_pos], self.buffer[byte_pos + 1]]);
                    let next = if byte_pos + 4 <= self.buffer.len() {
                        Some(u16::from_be_bytes([self.buffer[byte_pos + 2], self.buffer[byte_pos + 3]]))
                    } else {
                        None
                    };
                    let (ch, len) = decode_utf16_char(cu, || next);
                    self.char_byte_offsets.push(byte_pos);
                    self.chars.push(ch);
                    byte_pos += len;
                }
            }
        }
    }

    pub fn read_from_file(&mut self, mut f: impl Read) -> io::Result<()> {
        f.read_to_end(&mut self.buffer)?;
        self.close();
        self.decode_buffer();
        self.reset_stream();
        Ok(())
    }

    pub fn read_from_str(&mut self, s: &str, encoding: Option<Encoding>) {
        self.buffer = Vec::from(s.as_bytes());
        if let Some(enc) = encoding {
            self.encoding = enc;
        }
        self.decode_buffer();
        self.reset_stream();
    }

    pub fn append_str(&mut self, s: &str) {
        let saved = self.char_pos;
        self.buffer.extend_from_slice(s.as_bytes());
        self.decode_buffer();
        self.char_pos = saved.min(self.chars.len());
    }

    pub fn close(&mut self) {
        self.closed = true;
        // Re-decode so any trailing incomplete sequence is resolved
        self.decode_buffer();
    }

    pub fn read_from_bytes(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.buffer = bytes.to_vec();
        self.close();
        self.reset_stream();
        Ok(())
    }

    #[cfg(test)]
    fn chars_left(&self) -> usize {
        self.chars.len() - self.char_pos
    }
}

impl ByteStream {
    /// Detect the given encoding from stream analysis
    pub fn detect_encoding(&self) -> Encoding {
        let mut buf = self.buffer.as_slice();

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

        let mut encoding_detector = chardetng::EncodingDetector::new();
        encoding_detector.feed(buf, complete);

        let encoding = encoding_detector.guess(None, true);
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
        let current_byte_offset = self.tell_bytes();
        self.encoding = e;
        self.decode_buffer();
        // Remap char_pos to the same byte offset in the newly-decoded buffer
        let new_pos = self.char_byte_offsets.partition_point(|&b| b < current_byte_offset);
        self.char_pos = new_pos.min(self.chars.len());
    }
}

/// Location holds the start position of the given element in the data source
#[derive(Clone, PartialEq, Copy)]
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


/// Decode one UTF-8 character from the start of `buf`. Returns `(Character, bytes_consumed)`.
/// Returns `(StreamEnd, 0)` for an incomplete sequence at the end of an open stream (signals loop stop).
fn decode_one_utf8(buf: &[u8], closed: bool) -> (Character, usize) {
    match std::str::from_utf8(buf) {
        Ok(s) => {
            let ch = s.chars().next().expect("non-empty buf");
            (Ch(ch), ch.len_utf8())
        }
        Err(e) => {
            if e.valid_up_to() > 0 {
                let s = unsafe { std::str::from_utf8_unchecked(&buf[..e.valid_up_to()]) };
                let ch = s.chars().next().expect("valid_up_to > 0");
                (Ch(ch), ch.len_utf8())
            } else {
                match e.error_len() {
                    Some(n) => (Ch(REPLACEMENT_CHARACTER), n),
                    None => {
                        if closed {
                            (Ch(REPLACEMENT_CHARACTER), buf.len())
                        } else {
                            (StreamEnd, 0)
                        }
                    }
                }
            }
        }
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
            Some(Config { cr_lf_as_one: false, replace_cr_as_lf: true, replace_high_ascii: false }),
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
            Some(Config { cr_lf_as_one: false, replace_cr_as_lf: false, replace_high_ascii: false }),
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
            Some(Config { cr_lf_as_one: false, replace_cr_as_lf: false, replace_high_ascii: true }),
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

        stream.seek_bytes(50);
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

}
