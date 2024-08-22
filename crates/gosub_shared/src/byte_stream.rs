use std::fmt::{Debug, Formatter};
use std::io::Read;
use std::{fmt, io};

pub const CHAR_LF: char = '\u{000A}';
pub const CHAR_CR: char = '\u{000D}';

/// Encoding defines the way the buffer stream is read, as what defines a "character".
#[derive(PartialEq)]
pub enum Encoding {
    /// Stream is of UTF8 characters
    UTF8,
    /// Stream consists of 8-bit ASCII characters
    ASCII,
}

/// Defines a single character/element in the stream. This is either a UTF8 character, or
/// a surrogate characters since these cannot be stored in a single char.
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
    pub fn slice_to_string(v: &[Character]) -> String {
        v.iter().map(char::from).collect()
    }
}

pub struct Config {
    /// Current encoding
    pub encoding: Encoding,
    /// Treat any CRLF pairs as a single LF
    pub cr_lf_as_one: bool
}

impl Default for Config {
    fn default() -> Self {
        Self {
            encoding: Encoding::UTF8,
            cr_lf_as_one: true,
        }
    }
}

pub struct ByteStream {
    config: Config,
    /// Reference to the actual buffer stream in characters
    buffer: Vec<Character>,
    /// Current position in the stream, when it is the same as buffer length, we are at the end and no more can be read
    buffer_pos: usize,
    /// Reference to the actual buffer stream in u8 bytes
    u8_buffer: Vec<u8>,
    /// True when the buffer is empty and not yet have a closed stream
    closed: bool,
}

/// Generic stream trait
pub trait Stream {
    /// Read current character
    fn read(&self) -> Character;
    /// Read current character and advance to next
    fn read_and_next(&mut self) -> Character;
    /// Look ahead in the stream
    fn look_ahead(&self, offset: usize) -> Character;
    /// Advance with 1 character
    fn next(&mut self);
    /// Advance with offset characters
    fn next_n(&mut self, offset: usize);
    /// Unread the current character
    fn prev(&mut self);
    /// Unread n characters
    fn prev_n(&mut self, n: usize);
    // Seek to a specific position
    fn seek(&mut self, pos: usize);
    // Returns a slice
    fn get_slice(&self, len: usize) -> &[Character];
    /// Resets the stream back to the start position
    fn reset_stream(&mut self);
    /// Closes the stream (no more data can be added)
    fn close(&mut self);
    /// Returns true when the stream is closed
    fn closed(&self) -> bool;
    /// Returns true when the stream is empty (but still open)
    fn exhausted(&self) -> bool;
    /// REturns true when the stream is closed and empty
    fn eof(&self) -> bool;
    /// Returns the current offset in the stream
    fn offset(&self) -> usize;
    /// Returns the length of the stream
    fn length(&self) -> usize;
    /// Returns the number of characters left in the stream
    fn chars_left(&self) -> usize;
}

impl Default for ByteStream {
    fn default() -> Self {
        Self::new(None)
    }
}

impl Stream for ByteStream {
    /// Read the current character
    fn read(&self) -> Character {
        // Return none if we already have read EOF
        if self.eof() {
            return StreamEnd;
        }

        if self.buffer.is_empty() || self.buffer_pos >= self.buffer.len() {
            return StreamEmpty;
        }

        self.buffer[self.buffer_pos]
    }

    /// Read a character and advance to the next
    fn read_and_next(&mut self) -> Character {
        let c = self.read();

        self.next();
        c
    }

    /// Looks ahead in the stream, can use an optional index if we want to seek further
    /// (or back) in the stream.
    fn look_ahead(&self, offset: usize) -> Character {
        if self.buffer.is_empty() {
            return StreamEnd;
        }

        // Trying to look after the stream
        if self.buffer_pos + offset >= self.buffer.len() {
            return if self.closed() {
                StreamEnd
            } else {
                StreamEmpty
            };
        }

        self.buffer[self.buffer_pos + offset]
    }

    /// Returns the next character in the stream
    fn next(&mut self) {
        self.next_n(1);
    }

    /// Returns the n'th character in the stream
    fn next_n(&mut self, offset: usize) {
        if self.buffer.is_empty() {
            return;
        }

        self.buffer_pos += offset;
        if self.buffer_pos >= self.buffer.len() {
            self.buffer_pos = self.buffer.len();
        }
    }

    /// Unread the current character
    fn prev(&mut self) {
        self.prev_n(1);
    }

    /// Unread n characters
    fn prev_n(&mut self, n: usize) {
        if self.buffer_pos < n {
            self.buffer_pos = 0;
        } else {
            self.buffer_pos -= n;
        }
    }

    /// Seeks to a specific position in the stream
    fn seek(&mut self, pos: usize) {
        if pos >= self.buffer.len() {
            self.buffer_pos = self.buffer.len();
            return;
        }

        self.buffer_pos = pos;
    }

    /// Retrieves a slice of the buffer
    fn get_slice(&self, len: usize) -> &[Character] {
        if self.buffer_pos + len > self.buffer.len() {
            return &self.buffer[self.buffer_pos..];
        }

        &self.buffer[self.buffer_pos..self.buffer_pos + len]
    }

    /// Resets the stream to the first character of the stream
    fn reset_stream(&mut self) {
        self.buffer_pos = 0;
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
        self.buffer_pos >= self.buffer.len()
    }

    /// Returns true when the stream is closed and all the bytes have been read
    fn eof(&self) -> bool {
        self.closed() && self.exhausted()
    }

    /// Returns the current offset in the stream
    fn offset(&self) -> usize {
        self.buffer_pos
    }

    /// Returns the length of the buffer
    fn length(&self) -> usize {
        self.buffer.len()
    }

    /// Returns the number of characters left in the buffer
    fn chars_left(&self) -> usize {
        if self.buffer_pos >= self.buffer.len() {
            return 0;
        }

        self.buffer.len() - self.buffer_pos
    }
}

impl ByteStream {
    /// Create a new default empty input stream
    #[must_use]
    pub fn new(config: Option<Config>) -> Self {
        Self {
            config: config.unwrap_or(Config::default()),
            buffer: Vec::new(),
            buffer_pos: 0,
            u8_buffer: Vec::new(),
            closed: false,
        }
    }

    /// Populates the current buffer with the contents of given file f
    pub fn read_from_file(&mut self, mut f: impl Read, e: Option<Encoding>) -> io::Result<()> {
        // First we read the u8 bytes into a buffer
        f.read_to_end(&mut self.u8_buffer).expect("uh oh");
        self.close();
        self.force_set_encoding(e.unwrap_or(Encoding::UTF8));
        self.reset_stream();
        self.close();
        Ok(())
    }

    /// Populates the current buffer with the contents of the given string s
    pub fn read_from_str(&mut self, s: &str, e: Option<Encoding>) {
        self.u8_buffer = Vec::from(s.as_bytes());
        self.force_set_encoding(e.unwrap_or(Encoding::UTF8));
        self.reset_stream();
    }

    pub fn append_str(&mut self, s: &str, e: Option<Encoding>) {
        // @todo: this is not very efficient
        self.u8_buffer.extend_from_slice(s.as_bytes());
        self.force_set_encoding(e.unwrap_or(Encoding::UTF8));
    }

    pub fn close(&mut self) {
        self.closed = true;
    }

    /// Normalizes newlines (CRLF/CR => LF) and converts high ascii to '?'
    fn normalize_newlines_and_ascii(&self, buffer: &[u8]) -> Vec<Character> {
        let mut result = Vec::with_capacity(buffer.len());

        for i in 0..buffer.len() {
            if buffer[i] == CHAR_CR as u8 {
                // convert CR to LF, or CRLF to LF
                if i + 1 < buffer.len() && buffer[i + 1] == CHAR_LF as u8 {
                    continue;
                }
                result.push(Ch(CHAR_LF));
            } else if buffer[i] >= 0x80 {
                // Convert high ascii to ?
                result.push(Ch('?'));
            } else {
                // everything else is ok
                result.push(Ch(buffer[i] as char));
            }
        }

        result
    }

    /// Read directly from bytes
    pub fn read_from_bytes(&mut self, bytes: &[u8], e: Option<Encoding>) -> io::Result<()> {
        self.u8_buffer = bytes.to_vec();
        self.close();
        self.force_set_encoding(e.unwrap_or(Encoding::UTF8));
        self.reset_stream();
        Ok(())
    }

    /// Returns the number of characters left in the buffer
    #[cfg(test)]
    fn chars_left(&self) -> usize {
        self.buffer.len() - self.buffer_pos
    }
}

impl ByteStream {
    /// Detect the given encoding from stream analysis
    pub fn detect_encoding(&self) {
        panic!("Not implemented");
    }

    /// Changes the encoding and if necessary, decodes the u8 buffer into the correct encoding
    pub fn set_encoding(&mut self, e: Encoding) {
        // Don't convert if the encoding is the same as it already is
        if self.config.encoding == e {
            return;
        }

        self.force_set_encoding(e);
    }

    /// Sets the encoding for this stream, and decodes the u8_buffer into the buffer with the
    /// correct encoding.
    ///
    /// @TODO: I think we should not set an encoding and completely convert a stream. Instead,
    /// we should set an encoding, and try to use that encoding. If we find that we have a different
    /// encoding, we can notify the user, or try to convert the stream to the correct encoding.
    pub fn force_set_encoding(&mut self, e: Encoding) {
        match e {
            Encoding::UTF8 => {
                let str_buf = unsafe {
                    std::str::from_utf8_unchecked(&self.u8_buffer)
                        .replace("\u{000D}\u{000A}", "\u{000A}")
                        .replace('\u{000D}', "\u{000A}")
                };

                // Convert the utf8 string into characters, so we can use easy indexing
                self.buffer = str_buf
                    .chars()
                    .map(|c| {
                        // // Check if we have a non-bmp character. This means it's above 0x10000
                        // let cp = c as u32;
                        // if cp > 0x10000 && cp <= 0x10FFFF {
                        //     let adjusted = cp - 0x10000;
                        //     let lead = ((adjusted >> 10) & 0x3FF) as u16 + 0xD800;
                        //     let trail = (adjusted & 0x3FF) as u16 + 0xDC00;
                        //     self.buffer.push(Element::Surrogate(lead));
                        //     self.buffer.push(Element::Surrogate(trail));
                        //     continue;
                        // }

                        if (0xD800..=0xDFFF).contains(&(c as u32)) {
                            Surrogate(c as u16)
                        } else {
                            Ch(c)
                        }
                    })
                    .collect::<Vec<_>>();
            }
            Encoding::ASCII => {
                // Convert the string into characters, so we can use easy indexing. Any non-ascii chars (> 0x7F) are converted to '?'
                self.buffer = self.normalize_newlines_and_ascii(&self.u8_buffer);
            }
        }

        self.config.encoding = e;
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
        write!(f, "({}:{}:{})", self.line, self.column, self.offset)
    }
}

/// LocationHandler is a wrapper that will deal with line/column locations in the stream
pub struct LocationHandler {
    /// The start offset of the location. Normally this is 0:0, but can be different in case of inline streams
    pub start_location: Location,
    /// The current location of the stream
    pub cur_location: Location,
}

impl LocationHandler {
    /// Create a new LocationHandler. Start_location can be set in case the stream is
    /// not starting at 1:1
    pub fn new(start_location: Location) -> Self {
        Self {
            start_location,
            cur_location: Location::default(),
        }
    }

    /// Sets the current location to the given location. This is useful when we want to
    /// return back into the stream to a certain location.
    pub fn set(&mut self, loc: Location) {
        self.cur_location = loc;
    }

    /// Will increase the current location based on the given character
    pub fn inc(&mut self, ch: Character) {
        match ch {
            Ch(CHAR_LF) => {
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

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_stream() {
        let mut stream = ByteStream::new(None);
        assert!(stream.exhausted());
        assert!(!stream.eof());

        stream.read_from_str("foo", Some(Encoding::ASCII));
        stream.close();
        assert_eq!(stream.length(), 3);
        assert!(!stream.eof());
        assert_eq!(stream.chars_left(), 3);

        stream.read_from_str("f👽f", Some(Encoding::UTF8));
        stream.close();
        assert_eq!(stream.length(), 3);
        assert!(!stream.eof());
        assert_eq!(stream.chars_left(), 3);
        assert_eq!(stream.read_and_next(), Ch('f'));
        assert_eq!(stream.chars_left(), 2);
        assert!(!stream.eof());
        assert_eq!(stream.read_and_next(), Ch('👽'));
        assert!(!stream.eof());
        assert_eq!(stream.chars_left(), 1);
        assert_eq!(stream.read_and_next(), Ch('f'));
        assert!(stream.eof());
        assert_eq!(stream.chars_left(), 0);

        stream.reset_stream();
        stream.set_encoding(Encoding::ASCII);
        assert_eq!(stream.length(), 6);
        assert_eq!(stream.read_and_next(), Ch('f'));
        assert_eq!(stream.read_and_next(), Ch('?'));
        assert_eq!(stream.read_and_next(), Ch('?'));
        assert_eq!(stream.read_and_next(), Ch('?'));
        assert_eq!(stream.read_and_next(), Ch('?'));
        assert_eq!(stream.read_and_next(), Ch('f'));
        assert!(matches!(stream.read_and_next(), StreamEnd));

        stream.prev(); // unread 'f'
        stream.prev(); // Unread '?'
        stream.prev(); // Unread '?'
        assert_eq!(stream.chars_left(), 3);
        stream.prev();
        assert_eq!(stream.chars_left(), 4);

        stream.reset_stream();
        assert_eq!(stream.chars_left(), 6);
        stream.prev();
        assert_eq!(stream.chars_left(), 6);

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
        let mut stream = ByteStream::new(None);
        stream.read_from_str("abc", Some(Encoding::UTF8));
        stream.close();
        assert_eq!(stream.length(), 3);
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
        let mut stream = ByteStream::new(None);
        stream.read_from_str("abc", Some(Encoding::UTF8));
        assert_eq!(stream.length(), 3);
        assert_eq!(stream.chars_left(), 3);
        assert_eq!(stream.read_and_next(), Ch('a'));
        assert_eq!(stream.read_and_next(), Ch('b'));
        assert_eq!(stream.read_and_next(), Ch('c'));
        assert!(matches!(stream.read_and_next(), StreamEmpty));
        assert!(matches!(stream.read_and_next(), StreamEmpty));

        stream.append_str("def", Some(Encoding::UTF8));
        assert_eq!(stream.length(), 6);
        assert_eq!(stream.chars_left(), 3);
        assert_eq!(stream.read_and_next(), Ch('d'));
        assert_eq!(stream.read_and_next(), Ch('e'));
        assert_eq!(stream.read_and_next(), Ch('f'));
        assert!(matches!(stream.read_and_next(), StreamEmpty));

        stream.append_str("ghi", Some(Encoding::UTF8));
        stream.close();
        assert_eq!(stream.length(), 9);
        assert_eq!(stream.chars_left(), 3);
        assert_eq!(stream.read_and_next(), Ch('g'));
        assert_eq!(stream.read_and_next(), Ch('h'));
        assert_eq!(stream.read_and_next(), Ch('i'));
        assert!(matches!(stream.read_and_next(), StreamEnd));
        assert!(matches!(stream.read_and_next(), StreamEnd));
    }

    #[test]
    fn advance() {
        let mut stream = ByteStream::new(None);
        stream.read_from_str("abc", Some(Encoding::UTF8));
        stream.close();
        assert_eq!(stream.length(), 3);
        assert_eq!(stream.chars_left(), 3);
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
}
