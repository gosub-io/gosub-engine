use std::cell::RefCell;
use std::char::REPLACEMENT_CHARACTER;
use std::collections::HashMap;
use std::fmt::{Debug, Formatter};
use std::io::Read;
use std::{fmt, io};

pub const CHAR_LF: char = '\u{000A}';
pub const CHAR_CR: char = '\u{000D}';

/// Encoding defines the way the buffer stream is read, as what defines a "character".
#[derive(PartialEq)]
pub enum Encoding {
    /// Unknown encoding. Won't read anything from the stream until the encoding is set
    UNKNOWN,
    /// Stream is of single byte ASCII chars (0-255)
    ASCII,
    /// Stream is of UTF8 characters
    UTF8,
    // Stream consists of 16-bit UTF characters (Little Endian)
    UTF16LE,
    // Stream consists of 16-bit UTF characters (Big Endian)
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
    /// Stream buffer empty and closed
    StreamEnd,
    /// Stream buffer empty (but not closed)
    StreamEmpty,
}

use Character::*;

/// Converts the given character to a char. This is only valid for UTF8 characters. Surrogate
/// and EOF characters are converted to 0x0000
impl From<&Character> for char {
    fn from(c: &Character) -> Self {
        match c {
            Ch(c) => *c,
            Surrogate(..) => 0x0000 as char,
            StreamEmpty | StreamEnd => 0x0000 as char,
        }
    }
}

impl From<Character> for char {
    fn from(c: Character) -> Self {
        match c {
            Ch(c) => c,
            Surrogate(..) => 0x0000 as char,
            StreamEmpty | StreamEnd => 0x0000 as char,
        }
    }
}

impl fmt::Display for Character {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Ch(ch) => write!(f, "{ch}"),
            Surrogate(surrogate) => write!(f, "U+{surrogate:04X}"),
            StreamEnd => write!(f, "StreamEnd"),
            StreamEmpty => write!(f, "StreamEmpty"),
        }
    }
}

impl Character {
    /// Returns true when the character is a whitespace
    pub fn is_whitespace(&self) -> bool {
        matches!(self, Ch(c) if c.is_whitespace())
    }

    /// Returns true when the character is a numerical
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
    /// Actual buffer stream in u8 bytes
    buffer: Vec<u8>,
    /// Current position in the stream
    buffer_pos: RefCell<usize>,
    /// True when the buffer is empty and not yet have a closed stream
    closed: bool,
    /// Current encoding
    encoding: Encoding,
    // Configuration for the stream
    config: Config,
}

/// Generic stream trait
pub trait Stream {
    /// Read current character
    fn read(&self) -> Character;
    /// Read current character and advance to next
    fn read_and_next(&self) -> Character;
    /// Look ahead in the stream
    fn look_ahead(&self, offset: usize) -> Character;
    /// Advance with 1 character
    fn next(&self);
    /// Advance with offset characters
    fn next_n(&self, offset: usize);
    /// Unread the current character
    fn prev(&self);
    /// Unread n characters
    fn prev_n(&self, n: usize);
    // Seek to a specific position in bytes!
    fn seek_bytes(&self, offset: usize);
    // Tell the current position in bytes
    fn tell_bytes(&self) -> usize;
    /// Retrieves a slice of the buffer
    fn get_slice(&self, len: usize) -> Vec<Character>;
    /// Resets the stream back to the start position
    fn reset_stream(&self);
    /// Closes the stream (no more data can be added)
    fn close(&mut self);
    /// Returns true when the stream is closed
    fn closed(&self) -> bool;
    /// Returns true when the stream is empty (but still open)
    fn exhausted(&self) -> bool;
    /// Returns true when the stream is closed and empty
    fn eof(&self) -> bool;
}

impl Default for ByteStream {
    fn default() -> Self {
        Self::new(Encoding::UNKNOWN, None)
    }
}

impl Stream for ByteStream {
    /// Read the current character
    fn read(&self) -> Character {
        let (ch, _) = self.read_with_length();
        ch
    }

    /// Read a character and advance to the next
    fn read_and_next(&self) -> Character {
        let (ch, len) = self.read_with_length();

        {
            let mut pos = self.buffer_pos.borrow_mut();
            *pos += len;
        }

        // Make sure we skip the CR if it is followed by a LF
        if self.config.cr_lf_as_one && ch == Ch(CHAR_CR) && self.read() == Ch(CHAR_LF) {
            self.next();
            return Ch(CHAR_LF);
        }

        // Replace CR with LF if it is not followed by a LF
        if self.config.replace_cr_as_lf && ch == Ch(CHAR_CR) && self.read() != Ch(CHAR_LF) {
            return Ch(CHAR_LF);
        }

        ch
    }

    /// Looks ahead in the stream, can use an optional index if we want to seek further
    /// (or back) in the stream.
    fn look_ahead(&self, offset: usize) -> Character {
        if self.buffer.is_empty() {
            return StreamEnd;
        }

        let original_pos = *self.buffer_pos.borrow();

        self.next_n(offset);
        let ch = self.read();

        let mut pos = self.buffer_pos.borrow_mut();
        *pos = original_pos;

        ch
    }

    /// Returns the next character in the stream
    fn next(&self) {
        self.next_n(1);
    }

    /// Returns the n'th character in the stream
    fn next_n(&self, offset: usize) {
        for _ in 0..offset {
            let (_, len) = self.read_with_length();
            if len == 0 {
                return;
            }

            let mut pos = self.buffer_pos.borrow_mut();
            *pos += len;
        }
    }

    /// Unread the current character
    fn prev(&self) {
        self.prev_n(1);
    }

    /// Unread n characters
    fn prev_n(&self, n: usize) {
        // No need for extra checks, so we can just move back n characters
        if !self.config.cr_lf_as_one {
            self.move_back(n);
            return;
        }

        // We need to loop n times, as we might encounter CR/LF pairs we need to take into account
        for _ in 0..n {
            self.move_back(1);

            if self.config.cr_lf_as_one
                && self.read() == Ch(CHAR_CR)
                && self.look_ahead(1) == Ch(CHAR_LF)
            {
                self.move_back(1);
            }
        }
    }

    /// Seeks to a specific position in the stream
    fn seek_bytes(&self, offset: usize) {
        let mut pos = self.buffer_pos.borrow_mut();
        *pos = offset;
    }

    fn tell_bytes(&self) -> usize {
        *self.buffer_pos.borrow()
    }

    /// Retrieves a slice of the buffer
    fn get_slice(&self, len: usize) -> Vec<Character> {
        let current_pos = self.tell_bytes();

        let mut slice = Vec::with_capacity(len);
        for _ in 0..len {
            slice.push(self.read_and_next());
        }

        self.seek_bytes(current_pos);

        slice.clone()
    }

    /// Resets the stream to the first character of the stream
    fn reset_stream(&self) {
        let mut pos = self.buffer_pos.borrow_mut();
        *pos = 0;
    }

    /// Closes the stream so no more data can be added
    fn close(&mut self) {
        self.closed = true;
    }

    /// Returns true when the stream is closed and no more input can be read after this buffer
    /// is emptied
    fn closed(&self) -> bool {
        self.closed
    }

    /// Returns true when the buffer is empty and there is no more input to read
    /// Note that it does not check if the stream is closed. Use `closed` for that.
    fn exhausted(&self) -> bool {
        *self.buffer_pos.borrow() >= self.buffer.len()
    }

    /// Returns true when the stream is closed and all the bytes have been read
    fn eof(&self) -> bool {
        self.closed() && self.exhausted()
    }
}

impl ByteStream {
    /// Create a new default empty input stream
    #[must_use]
    pub fn new(encoding: Encoding, config: Option<Config>) -> Self {
        Self {
            config: config.unwrap_or_default(),
            buffer_pos: RefCell::new(0),
            buffer: Vec::new(),
            closed: false,
            encoding,
        }
    }

    // Read the character and return it together with the number of bytes the character took
    fn read_with_length(&self) -> (Character, usize) {
        if self.eof() || self.buffer.is_empty() || *self.buffer_pos.borrow() >= self.buffer.len() {
            if self.closed {
                return (StreamEnd, 0);
            }
            return (StreamEmpty, 0);
        }

        let buf_pos = self.buffer_pos.borrow();

        match self.encoding {
            Encoding::UNKNOWN => {
                todo!("Unknown encoding. Please detect encoding first");
            }
            Encoding::ASCII => {
                if *buf_pos >= self.buffer.len() {
                    if self.closed {
                        return (StreamEnd, 0);
                    }
                    return (StreamEmpty, 0);
                }

                if self.config.replace_high_ascii && self.buffer[*buf_pos] > 127 {
                    (Ch('?'), 1)
                } else {
                    (Ch(self.buffer[*buf_pos] as char), 1)
                }
            }
            Encoding::UTF8 => {
                let first_byte = self.buffer[*buf_pos];
                let width = utf8_char_width(first_byte);

                if *buf_pos + width > self.buffer.len() {
                    return (StreamEmpty, self.buffer.len() - *buf_pos);
                }

                let ch = match width {
                    1 => first_byte as u32,
                    2 => {
                        ((first_byte as u32 & 0x1F) << 6)
                            | (self.buffer[*buf_pos + 1] as u32 & 0x3F)
                    }
                    3 => {
                        ((first_byte as u32 & 0x0F) << 12)
                            | ((self.buffer[*buf_pos + 1] as u32 & 0x3F) << 6)
                            | (self.buffer[*buf_pos + 2] as u32 & 0x3F)
                    }
                    4 => {
                        ((first_byte as u32 & 0x07) << 18)
                            | ((self.buffer[*buf_pos + 1] as u32 & 0x3F) << 12)
                            | ((self.buffer[*buf_pos + 2] as u32 & 0x3F) << 6)
                            | (self.buffer[*buf_pos + 3] as u32 & 0x3F)
                    }
                    _ => 0xFFFD, // Invalid UTF-8 byte sequence
                };

                if ch > 0x10FFFF || (ch > 0xD800 && ch <= 0xDFFF) {
                    (Surrogate(ch as u16), width)
                } else {
                    (
                        char::from_u32(ch).map_or(Ch(REPLACEMENT_CHARACTER), Ch),
                        width,
                    )
                }
            }
            Encoding::UTF16LE => {
                if *buf_pos + 1 < self.buffer.len() {
                    let code_unit =
                        u16::from_le_bytes([self.buffer[*buf_pos], self.buffer[*buf_pos + 1]]);
                    (
                        char::from_u32(u32::from(code_unit)).map_or(Ch(REPLACEMENT_CHARACTER), Ch),
                        2,
                    )
                } else {
                    (StreamEmpty, 1)
                }
            }
            Encoding::UTF16BE => {
                if *buf_pos + 1 < self.buffer.len() {
                    let code_unit =
                        u16::from_be_bytes([self.buffer[*buf_pos], self.buffer[*buf_pos + 1]]);
                    (
                        char::from_u32(u32::from(code_unit)).map_or(Ch(REPLACEMENT_CHARACTER), Ch),
                        2,
                    )
                } else {
                    (StreamEmpty, 1)
                }
            }
        }
    }

    /// Populates the current buffer with the contents of given file f
    pub fn read_from_file(&mut self, mut f: impl Read) -> io::Result<()> {
        // First we read the u8 bytes into a buffer
        f.read_to_end(&mut self.buffer).expect("uh oh");
        self.close();
        self.reset_stream();
        self.close();
        Ok(())
    }

    /// Populates the current buffer with the contents of the given string s
    pub fn read_from_str(&mut self, s: &str, _encoding: Option<Encoding>) {
        self.buffer = Vec::from(s.as_bytes());
        self.reset_stream();
    }

    pub fn append_str(&mut self, s: &str) {
        self.buffer.extend_from_slice(s.as_bytes());
    }

    pub fn close(&mut self) {
        self.closed = true;
    }

    /// Read directly from bytes
    pub fn read_from_bytes(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.buffer = bytes.to_vec();
        self.close();
        self.reset_stream();
        Ok(())
    }

    /// Returns the number of characters left in the buffer
    #[cfg(test)]
    fn chars_left(&self) -> usize {
        self.buffer.len() - *self.buffer_pos.borrow()
    }

    // Moves back n characters in the stream
    fn move_back(&self, n: usize) {
        let mut pos = self.buffer_pos.borrow_mut();

        match self.encoding {
            Encoding::ASCII => {
                if *pos > n {
                    *pos -= n;
                } else {
                    *pos = 0;
                }
            }
            Encoding::UTF8 => {
                let mut n = n;
                while n > 0 && *pos > 0 {
                    *pos -= 1;

                    if self.buffer[*pos] & 0b1100_0000 != 0b1000_0000 {
                        n -= 1;
                    }
                }
            }
            Encoding::UTF16LE => {
                if *pos > n * 2 {
                    *pos -= n * 2;
                } else {
                    *pos = 0;
                }
            }
            Encoding::UTF16BE => {
                if *pos > n * 2 {
                    *pos -= n * 2;
                } else {
                    *pos = 0;
                }
            }
            _ => {}
        }
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
        if encoding == encoding_rs::UTF_8 {
            Encoding::UTF8
        } else if encoding == encoding_rs::UTF_16BE {
            Encoding::UTF16BE
        } else if encoding == encoding_rs::UTF_16LE {
            Encoding::UTF16LE
        } else {
            panic!("Unsupported encoding");
        }
    }

    /// Changes the encoding that the decoder uses to read the buffer. Note that this does not reset
    /// the buffer, so it might start on a non-valid character.
    pub fn set_encoding(&mut self, e: Encoding) {
        self.encoding = e;
    }
}

/// Location holds the start position of the given element in the data source
#[derive(Clone, PartialEq)]
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
    pub fn new(line: usize, column: usize, offset: usize) -> Self {
        Self {
            line,
            column,
            offset,
        }
    }
}

impl Debug for Location {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "({}:{})", self.line, self.column)
    }
}

/// LocationHandler is a wrapper that will deal with line/column locations in the stream
pub struct LocationHandler {
    /// The start offset of the location. Normally this is 0:0, but can be different in case of inline streams
    pub start_location: Location,
    /// The current location of the stream
    pub cur_location: Location,
    /// List of all line number -> col size mappings
    line_endings: HashMap<usize, usize>,
}

impl LocationHandler {
    /// Create a new LocationHandler. Start_location can be set in case the stream is
    /// not starting at 1:1
    pub fn new(start_location: Location) -> Self {
        Self {
            start_location,
            cur_location: Location::default(),
            line_endings: HashMap::new(),
        }
    }

    /// Sets the current location to the given location. This is useful when we want to
    /// return back into the stream to a certain location.
    pub fn set(&mut self, loc: Location) {
        self.cur_location = loc;
    }

    /// Will decrease the current location based on the current character
    pub fn dec(&mut self) {
        if self.cur_location.column > 1 {
            self.cur_location.column -= 1;
            self.cur_location.offset -= 1;
            return;
        }

        if self.cur_location.line > 1 {
            self.cur_location.line -= 1;
            self.cur_location.column = self.line_endings[&self.cur_location.line];
            self.cur_location.offset -= 1;
        }
    }

    /// Will increase the current location based on the given character
    pub fn inc(&mut self, ch: Character) {
        match ch {
            Ch(CHAR_LF) => {
                self.line_endings
                    .insert(self.cur_location.line, self.cur_location.column);

                self.cur_location.line += 1;
                self.cur_location.column = 1;
                self.cur_location.offset += 1;
            }
            Ch(_) => {
                self.cur_location.column += 1;
                self.cur_location.offset += 1;
            }
            StreamEnd | StreamEmpty => {}
            _ => {}
        }
    }
}

/// Returns the width of the given UTF8 character, which is based on the first byte
#[inline]
fn utf8_char_width(first_byte: u8) -> usize {
    if first_byte < 0x80 {
        1
    } else {
        2 + (first_byte >= 0xE0) as usize + (first_byte >= 0xF0) as usize
    }
    // match first_byte {
    //     0..=0x7F => 1,
    //     0xC2..=0xDF => 2,
    //     0xE0..=0xEF => 3,
    //     0xF0..=0xF4 => 4,
    //     _ => 1,
    // }
}

#[cfg(test)]
mod test {
    use super::*;

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

        stream.read_from_str("foo", Some(Encoding::ASCII));
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
        stream.set_encoding(Encoding::ASCII);
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
        assert!(matches!(stream.read_and_next(), StreamEmpty));
        assert!(matches!(stream.read_and_next(), StreamEmpty));

        stream.append_str("def");
        assert_eq!(stream.read_and_next(), Ch('d'));
        assert_eq!(stream.read_and_next(), Ch('e'));
        assert_eq!(stream.read_and_next(), Ch('f'));
        assert!(matches!(stream.read_and_next(), StreamEmpty));

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

        stream.set_encoding(Encoding::ASCII);
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
        assert_eq!(format!("{}", ch), "a");

        let ch = Surrogate(0xDFA9);
        assert_eq!(format!("{}", ch), "U+DFA9");
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
            0x00, 0x51, 0x00, 0x75, 0x00, 0x69, 0x00, 0x7a, 0x00, 0x64, 0x00, 0x65, 0x00, 0x6c,
            0x00, 0x74, 0x00, 0x61, 0x00, 0x67, 0x00, 0x65, 0x00, 0x72, 0x00, 0x6e, 0x00, 0x65,
            0x00, 0x20, 0x00, 0x73, 0x00, 0x70, 0x00, 0x69, 0x00, 0x73, 0x00, 0x74, 0x00, 0x65,
            0x00, 0x20, 0x00, 0x6a, 0x00, 0x6f, 0x00, 0x72, 0x00, 0x64, 0x00, 0x62, 0x00, 0xe6,
            0x00, 0x72, 0x00, 0x20, 0x00, 0x6d, 0x00, 0x65, 0x00, 0x64, 0x00, 0x20, 0x00, 0x66,
            0x00, 0x6c, 0x00, 0xf8, 0x00, 0x64, 0x00, 0x65, 0x00, 0x2c, 0x00, 0x20, 0x00, 0x6d,
            0x00, 0x65, 0x00, 0x6e, 0x00, 0x73, 0x00, 0x20, 0x00, 0x63, 0x00, 0x69, 0x00, 0x72,
            0x00, 0x6b, 0x00, 0x75, 0x00, 0x73, 0x00, 0x6b, 0x00, 0x6c, 0x00, 0x6f, 0x00, 0x76,
            0x00, 0x6e, 0x00, 0x65, 0x00, 0x6e, 0x00, 0x20, 0x00, 0x57, 0x00, 0x6f, 0x00, 0x6c,
            0x00, 0x74, 0x00, 0x68, 0x00, 0x65, 0x00, 0x72, 0x00, 0x20, 0x00, 0x73, 0x00, 0x70,
            0x00, 0x69, 0x00, 0x6c, 0x00, 0x6c, 0x00, 0x65, 0x00, 0x64, 0x00, 0x65, 0x00, 0x20,
            0x00, 0x70, 0x00, 0xe5, 0x00, 0x20, 0x00, 0x78, 0x00, 0x79, 0x00, 0x6c, 0x00, 0x6f,
            0x00, 0x66, 0x00, 0x6f, 0x00, 0x6e, 0x00, 0x2e,
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
