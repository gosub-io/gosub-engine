use crate::html5::tokenizer::{CHAR_CR, CHAR_LF};
use std::io::Read;
use std::{fmt, io};

/// Encoding defines the way the buffer stream is read, as what defines a "character".
#[derive(PartialEq)]
pub enum Encoding {
    /// Stream is of UTF8 characters
    UTF8,
    /// Stream consists of 8-bit ASCII characters
    ASCII,
}

/// The confidence decides how confident we are that the input stream is of this encoding
#[derive(PartialEq)]
pub enum Confidence {
    /// This encoding might be the one we need
    Tentative,
    /// We are certain to use this encoding
    Certain,
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
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Ch(ch) => write!(f, "{ch}"),
            Surrogate(surrogate) => write!(f, "U+{surrogate:04X}"),
            StreamEnd => write!(f, "StreamEnd"),
            StreamEmpty => write!(f, "StreamEmpty"),
        }
    }
}

impl Character {
    pub fn is_whitespace(&self) -> bool {
        matches!(self, Self::Ch(c) if c.is_whitespace())
    }
    pub fn is_numeric(&self) -> bool {
        matches!(self, Self::Ch(c) if c.is_numeric())
    }
}

pub struct ByteStream {
    /// Current encoding
    pub encoding: Encoding,
    /// How confident are we that this is the correct encoding?
    pub confidence: Confidence,
    /// Reference to the actual buffer stream in characters
    buffer: Vec<Character>,
    /// Current position in the stream, when it is the same as buffer length, we are at the end and no more can be read
    buffer_pos: usize,
    /// Reference to the actual buffer stream in u8 bytes
    u8_buffer: Vec<u8>,
    // True when the buffer is empty and not yet have a closed stream
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

    // Returns a slice
    fn get_slice(&self, start: usize, end: usize) -> &[Character];

    /// Resets the stream back to the start position
    fn reset_stream(&mut self);
    /// Closes the stream (no more data can be added)
    fn close(&mut self);
    /// Returns true when the stream is closed
    fn closed(&self) -> bool;
    /// Returns true when the stream is empty (but still open)
    fn empty(&self) -> bool;
    /// REturns true when the stream is closed and empty
    fn eof(&self) -> bool;
    /// Returns the current offset in the stream
    fn tell(&self) -> usize;
    /// Returns the length of the stream
    fn length(&self) -> usize;
    /// Returns the number of characters left in the stream
    fn chars_left(&self) -> usize;
}

impl Default for ByteStream {
    fn default() -> Self {
        Self::new()
    }
}

impl Stream for ByteStream {
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
    fn empty(&self) -> bool {
        self.buffer_pos >= self.buffer.len()
    }

    /// Returns true when the stream is closed and all the bytes have been read
    fn eof(&self) -> bool {
        self.closed() && self.empty()
    }

    fn next(&mut self) {
        self.next_n(1);
    }

    fn next_n(&mut self, offset: usize) {
        if self.buffer.is_empty() {
            return;
        }

        self.buffer_pos += offset;
        if self.buffer_pos >= self.buffer.len() {
            self.buffer_pos = self.buffer.len();
        }
    }

    /// Returns the current offset in the stream
    fn tell(&self) -> usize {
        self.buffer_pos
    }

    /// Returns the length of the buffer
    fn length(&self) -> usize {
        self.buffer.len()
    }

    fn reset_stream(&mut self) {
        self.buffer_pos = 0;
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

    fn read_and_next(&mut self) -> Character {
        let c = self.read();

        self.next();
        c
    }

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

    fn chars_left(&self) -> usize {
        if self.buffer_pos >= self.buffer.len() {
            return 0;
        }

        self.buffer.len() - self.buffer_pos
    }

    fn prev(&mut self) {
        self.prev_n(1);
    }

    fn prev_n(&mut self, n: usize) {
        if self.buffer_pos < n {
            self.buffer_pos = 0;
        } else {
            self.buffer_pos -= n;
        }
    }

    /// Retrieves a slice of the buffer
    fn get_slice(&self, start: usize, end: usize) -> &[Character] {
        &self.buffer[start..end]
    }
}

impl ByteStream {
    /// Create a new default empty input stream
    #[must_use]
    pub fn new() -> Self {
        Self {
            encoding: Encoding::UTF8,
            confidence: Confidence::Tentative,
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
    /// Returns true when the encoding encountered is defined as certain
    pub fn is_certain_encoding(&self) -> bool {
        self.confidence == Confidence::Certain
    }

    /// Detect the given encoding from stream analysis
    pub fn detect_encoding(&self) {
        todo!()
    }

    /// Set the given confidence of the input stream encoding
    pub fn set_confidence(&mut self, c: Confidence) {
        self.confidence = c;
    }

    /// Changes the encoding and if necessary, decodes the u8 buffer into the correct encoding
    pub fn set_encoding(&mut self, e: Encoding) {
        // Don't convert if the encoding is the same as it already is
        if self.encoding == e {
            return;
        }

        self.force_set_encoding(e);
    }

    /// Sets the encoding for this stream, and decodes the u8_buffer into the buffer with the
    /// correct encoding.
    pub fn force_set_encoding(&mut self, e: Encoding) {
        match e {
            Encoding::UTF8 => {
                let str_buf = unsafe {
                    std::str::from_utf8_unchecked(&self.u8_buffer)
                        .replace("\u{000D}\u{000A}", "\u{000A}")
                        .replace('\u{000D}', "\u{000A}")
                };

                // Convert the utf8 string into characters so we can use easy indexing
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
                            Character::Surrogate(c as u16)
                        } else {
                            Ch(c)
                        }
                    })
                    .collect::<Vec<_>>();
            }
            Encoding::ASCII => {
                // Convert the string into characters so we can use easy indexing. Any non-ascii chars (> 0x7F) are converted to '?'
                self.buffer = self.normalize_newlines_and_ascii(&self.u8_buffer);
            }
        }

        self.encoding = e;
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_stream() {
        let mut stream = ByteStream::new();
        assert!(stream.empty());
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
    fn test_certainty() {
        let mut stream = ByteStream::new();
        assert!(!stream.is_certain_encoding());

        stream.set_confidence(Confidence::Certain);
        assert!(stream.is_certain_encoding());

        stream.set_confidence(Confidence::Tentative);
        assert!(!stream.is_certain_encoding());
    }

    #[test]
    fn test_eof() {
        let mut stream = ByteStream::new();
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
        let mut stream = ByteStream::new();
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
    }

    #[test]
    fn advance() {
        let mut stream = ByteStream::new();
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
