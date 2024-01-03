use crate::html5::tokenizer::{CHAR_CR, CHAR_LF};
use std::collections::HashMap;
use std::io::Read;
use std::iter::Iterator;
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

/// This struct defines a position in the stream. POsition itself is 0-based, but line and col are
/// 1-based and are calculated from the line_offsets vector.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Position {
    /// Offset in the stream
    pub offset: usize,
    /// Line number (1-based)
    pub line: usize,
    /// Column number (1-based)
    pub col: usize,
}

impl Position {
    /// Create a new position
    #[must_use]
    pub fn new(offset: usize, line: usize, col: usize) -> Self {
        Self { offset, line, col }
    }
}

impl fmt::Display for Position {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}:{}", self.offset, self.line, self.col)
    }
}

/// Defines a single character/element in the stream. This is either a UTF8 character, or
/// a surrogate characters since these cannot be stored in a single char.
/// Eof is denoted as a separate element.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Bytes {
    /// Standard UTF character
    Ch(char),
    /// Surrogate character (since they cannot be stored in char)
    Surrogate(u16),
    /// End of stream
    Eof,
}

use Bytes::*;

/// Converts the given character to a char. This is only valid for UTF8 characters. Surrogate
/// and EOF characters are converted to 0x0000
impl From<Bytes> for char {
    fn from(c: Bytes) -> Self {
        match c {
            Ch(c) => c,
            Bytes::Surrogate(..) | Eof => 0x0000 as char,
        }
    }
}

impl fmt::Display for Bytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Ch(ch) => write!(f, "{ch}"),
            Bytes::Surrogate(surrogate) => write!(f, "U+{surrogate:04X}"),
            Eof => write!(f, "EOF"),
        }
    }
}

impl Bytes {
    pub fn is_whitespace(&self) -> bool {
        matches!(self, Self::Ch(c) if c.is_whitespace())
    }

    pub fn is_numeric(&self) -> bool {
        matches!(self, Self::Ch(c) if c.is_numeric())
    }
}

/// Buffered UTF-8 iterator
pub struct CharIterator {
    /// Current encoding
    pub encoding: Encoding,
    /// How confident are we that this is the correct encoding?
    pub confidence: Confidence,
    /// Current positions
    pub position: Position,
    /// Length (in chars) of the buffer
    pub length: usize,
    /// Offsets of the given lines
    line_columns: HashMap<usize, usize>,
    /// Reference to the actual buffer stream in characters
    buffer: Vec<Bytes>,
    /// Reference to the actual buffer stream in u8 bytes
    u8_buffer: Vec<u8>,
    /// If all things are ok, both buffer and u8_buffer should refer to the same memory location (?)
    pub has_read_eof: bool, // True when we just read an EOF
}

impl Default for CharIterator {
    fn default() -> Self {
        Self::new()
    }
}

impl Iterator for CharIterator {
    type Item = char;

    fn next(&mut self) -> Option<Self::Item> {
        if self.eof() || self.position.offset >= self.length {
            return None;
        }

        // SAFETY: self.buffer and self.u8_buffer have the same length
        let c = self.u8_buffer[self.position.offset] as char;

        if c == '\n' {
            // Store line offset for the given line
            self.line_columns
                .insert(self.position.line, self.position.col);
            // And continue position on the next line
            self.position.line += 1;
            self.position.col = 1;
        } else {
            self.position.col += 1;
        }

        self.position.offset += 1;
        Some(c)
    }
}

impl CharIterator {
    /// Create a new default empty input stream
    #[must_use]
    pub fn new() -> Self {
        Self {
            encoding: Encoding::UTF8,
            confidence: Confidence::Tentative,
            position: Position {
                offset: 0,
                line: 1,
                col: 1,
            },
            length: 0,
            line_columns: HashMap::new(),
            buffer: Vec::new(),
            u8_buffer: Vec::new(),
            has_read_eof: false,
        }
    }
    /// Returns true when the encoding encountered is defined as certain
    pub fn is_certain_encoding(&self) -> bool {
        self.confidence == Confidence::Certain
    }

    /// Detect the given encoding from stream analysis
    pub fn detect_encoding(&self) {
        todo!()
    }

    /// Returns true when the stream pointer is at the end of the stream
    pub fn eof(&self) -> bool {
        self.has_read_eof || self.position.offset >= self.length
    }

    /// Reset the stream reader back to the start
    pub fn reset(&mut self) {
        self.position.offset = 0;
        self.position.line = 1;
        self.position.col = 1;
    }

    /// Skip offset characters in the stream (based on chars)
    pub fn skip(&mut self, offset: usize) {
        let mut skip_len = offset;
        if self.position.offset + offset >= self.length {
            skip_len = self.length - self.position.offset;
        }

        for _ in 0..skip_len {
            self.read_char();
        }
    }

    /// Returns the previous position based on the current position
    pub fn get_previous_position(&mut self) -> Position {
        // if we are at the beginning or the end of the stream, we just return the current position
        if self.position.offset == 0 || self.has_read_eof {
            return self.position;
        }

        self.unread();
        let pos = self.position;
        self.skip(1);

        pos
    }

    /// Returns the current offset in the stream
    pub fn tell(&self) -> usize {
        self.position.offset
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
                            Bytes::Surrogate(c as u16)
                        } else {
                            Ch(c)
                        }
                    })
                    .collect::<Vec<_>>();
                self.length = self.buffer.len();
            }
            Encoding::ASCII => {
                // Convert the string into characters so we can use easy indexing. Any non-ascii chars (> 0x7F) are converted to '?'
                self.buffer = self.normalize_newlines_and_ascii(&self.u8_buffer);
                self.length = self.buffer.len();
            }
        }

        self.encoding = e;
    }

    /// Normalizes newlines (CRLF/CR => LF) and converts high ascii to '?'
    fn normalize_newlines_and_ascii(&self, buffer: &[u8]) -> Vec<Bytes> {
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
        self.force_set_encoding(e.unwrap_or(Encoding::UTF8));
        self.reset();
        Ok(())
    }

    /// Populates the current buffer with the contents of given file f
    pub fn read_from_file(&mut self, mut f: impl Read, e: Option<Encoding>) -> io::Result<()> {
        // First we read the u8 bytes into a buffer
        f.read_to_end(&mut self.u8_buffer).expect("uh oh");
        self.force_set_encoding(e.unwrap_or(Encoding::UTF8));
        self.reset();
        Ok(())
    }

    /// Populates the current buffer with the contents of the given string s
    pub fn read_from_str(&mut self, s: &str, e: Option<Encoding>) {
        self.u8_buffer = Vec::from(s.as_bytes());
        self.force_set_encoding(e.unwrap_or(Encoding::UTF8));
        self.reset();
    }

    /// Returns the number of characters left in the buffer
    #[cfg(test)]
    fn chars_left(&self) -> usize {
        self.length - self.position.offset
    }

    /// Reads a character and increases the current pointer, or read EOF as None
    pub(crate) fn read_char(&mut self) -> Bytes {
        // Return none if we already have read EOF
        if self.has_read_eof {
            return Eof;
        }

        // If we still can move forward in the stream, move forwards
        if self.position.offset < self.length {
            let c = self.buffer[self.position.offset];
            if c == Ch('\n') {
                // Store line offset for the given line
                self.line_columns
                    .insert(self.position.line, self.position.col);
                // And continue position on the next line
                self.position.line += 1;
                self.position.col = 1;
            } else {
                self.position.col += 1;
            }
            self.position.offset += 1;
            return c;
        }

        // otherwise, we have reached the end of the stream
        self.has_read_eof = true;

        Eof
    }

    pub(crate) fn unread(&mut self) {
        // We already read eof, so "unread" the eof by unsetting the flag
        if self.has_read_eof {
            self.has_read_eof = false;
            return;
        }

        // If we can track back from the offset, we can do so
        if self.position.offset > 0 {
            self.position.offset -= 1;

            if self.position.col == 1 {
                self.position.line -= 1;
                let key = self.position.line;
                self.position.col = *self.line_columns.get(&key).unwrap_or(&1);
            } else {
                self.position.col -= 1;
            }
        }
    }

    /// Looks ahead in the stream and returns len characters
    pub(crate) fn look_ahead_slice(&self, len: usize) -> String {
        let end_pos = std::cmp::min(self.length, self.position.offset + len);

        let slice = &self.buffer[self.position.offset..end_pos];
        slice.iter().map(ToString::to_string).collect()
    }

    /// Looks ahead in the stream, can use an optional index if we want to seek further
    /// (or back) in the stream.
    pub(crate) fn look_ahead(&self, offset: usize) -> Bytes {
        // Trying to look after the stream
        if self.position.offset + offset >= self.length {
            return Eof;
        }

        self.buffer[self.position.offset + offset]
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_stream() {
        let mut chars = CharIterator::new();
        assert!(chars.eof());

        chars.read_from_str("foo", Some(Encoding::ASCII));
        assert_eq!(chars.length, 3);
        assert!(!chars.eof());
        assert_eq!(chars.chars_left(), 3);

        chars.read_from_str("f👽f", Some(Encoding::UTF8));
        assert_eq!(chars.length, 3);
        assert!(!chars.eof());
        assert_eq!(chars.chars_left(), 3);
        assert_eq!(chars.read_char(), Ch('f'));
        assert_eq!(chars.chars_left(), 2);
        assert!(!chars.eof());
        assert_eq!(chars.read_char(), Ch('👽'));
        assert!(!chars.eof());
        assert_eq!(chars.chars_left(), 1);
        assert_eq!(chars.read_char(), Ch('f'));
        assert!(chars.eof());
        assert_eq!(chars.chars_left(), 0);

        chars.reset();
        chars.set_encoding(Encoding::ASCII);
        assert_eq!(chars.length, 6);
        assert_eq!(chars.read_char(), Ch('f'));
        assert_eq!(chars.read_char(), Ch('?'));
        assert_eq!(chars.read_char(), Ch('?'));
        assert_eq!(chars.read_char(), Ch('?'));
        assert_eq!(chars.read_char(), Ch('?'));
        assert_eq!(chars.read_char(), Ch('f'));
        assert!(matches!(chars.read_char(), Eof));

        chars.unread(); // unread eof
        chars.unread(); // unread 'f'
        chars.unread(); // Unread '?'
        assert_eq!(chars.chars_left(), 2);
        chars.unread();
        assert_eq!(chars.chars_left(), 3);

        chars.reset();
        assert_eq!(chars.chars_left(), 6);
        chars.unread();
        assert_eq!(chars.chars_left(), 6);

        chars.read_from_str("abc", Some(Encoding::UTF8));
        chars.reset();
        assert_eq!(chars.read_char(), Ch('a'));
        chars.unread();
        assert_eq!(chars.read_char(), Ch('a'));
        assert_eq!(chars.read_char(), Ch('b'));
        chars.unread();
        assert_eq!(chars.read_char(), Ch('b'));
        assert_eq!(chars.read_char(), Ch('c'));
        chars.unread();
        assert_eq!(chars.read_char(), Ch('c'));
        assert!(matches!(chars.read_char(), Eof));
        chars.unread();
        assert!(matches!(chars.read_char(), Eof));
    }

    #[test]
    fn test_certainty() {
        let mut chars = CharIterator::new();
        assert!(!chars.is_certain_encoding());

        chars.set_confidence(Confidence::Certain);
        assert!(chars.is_certain_encoding());

        chars.set_confidence(Confidence::Tentative);
        assert!(!chars.is_certain_encoding());
    }

    #[test]
    fn test_eof() {
        let mut chars = CharIterator::new();
        chars.read_from_str("abc", Some(Encoding::UTF8));
        assert_eq!(chars.length, 3);
        assert_eq!(chars.chars_left(), 3);
        assert_eq!(chars.read_char(), Ch('a'));
        assert_eq!(chars.read_char(), Ch('b'));
        assert_eq!(chars.read_char(), Ch('c'));
        assert!(matches!(chars.read_char(), Eof));
        assert!(matches!(chars.read_char(), Eof));
        assert!(matches!(chars.read_char(), Eof));
        assert!(matches!(chars.read_char(), Eof));
        chars.unread();
        assert!(matches!(chars.read_char(), Eof));
        chars.unread();
        chars.unread();
        assert!(!matches!(chars.read_char(), Eof));
        assert!(matches!(chars.read_char(), Eof));
        chars.unread();
        chars.unread();
        assert!(!matches!(chars.read_char(), Eof));
        chars.unread();
        chars.unread();
        chars.unread();
        assert_eq!(chars.read_char(), Ch('a'));
        chars.unread();
        assert_eq!(chars.read_char(), Ch('a'));
        chars.unread();
        chars.unread();
        assert_eq!(chars.read_char(), Ch('a'));
        chars.unread();
        chars.unread();
        chars.unread();
        chars.unread();
        chars.unread();
        chars.unread();
        assert_eq!(chars.read_char(), Ch('a'));
        assert_eq!(chars.read_char(), Ch('b'));
        assert_eq!(chars.read_char(), Ch('c'));
        assert!(matches!(chars.read_char(), Eof));
        chars.unread();
        chars.unread();
        assert_eq!(chars.read_char(), Ch('c'));
        assert!(matches!(chars.read_char(), Eof));
        chars.unread();
        assert!(matches!(chars.read_char(), Eof));
    }

    #[test]
    fn test_iter() {
        let mut chars = CharIterator::new();
        chars.read_from_str("abc", Some(Encoding::UTF8));
        assert_eq!(chars.next(), Some('a'));
        assert_eq!(chars.next(), Some('b'));
        assert_eq!(chars.next(), Some('c'));
        assert_eq!(chars.next(), None);
        assert!(chars.eof());
    }

    #[test]
    fn test_peekable() {
        let mut chars = CharIterator::new();
        chars.read_from_str("abc", Some(Encoding::UTF8));
        let mut peekable = chars.peekable();
        assert_eq!(peekable.peek(), Some(&'a'));
        assert_eq!(peekable.next(), Some('a'));
        assert_eq!(peekable.peek(), Some(&'b'));
        assert_eq!(peekable.next(), Some('b'));
        let nxt = peekable.peek_mut().unwrap();
        *nxt = 'd';
        assert_eq!(peekable.peek(), Some(&'d'));
        assert_eq!(peekable.next(), Some('d'));
        assert_eq!(peekable.next(), None);
    }
}
