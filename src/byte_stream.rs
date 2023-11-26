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
    /// Start of stream
    StreamStart,
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
            StreamEmpty | StreamEnd | StreamStart => 0x0000 as char,
        }
    }
}

impl fmt::Display for Character {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Ch(ch) => write!(f, "{ch}"),
            Surrogate(surrogate) => write!(f, "U+{surrogate:04X}"),
            StreamStart => write!(f, "StreamStart"),
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
    /// Current position in the stream
    buffer_pos: usize,
    /// Reference to the actual buffer stream in u8 bytes
    u8_buffer: Vec<u8>,
    // True when the buffer is empty and not yet have a closed stream
    closed: bool,
}


/// Generic stream trait (not yet used)
trait Stream {
    /// Resets the stream back to the start position
    fn reset_stream(&mut self);
    /// Closes the stream (no more data can be added)
    fn close_stream(&mut self);
    /// Returns true when the stream is closed
    fn closed(&self) -> bool;
    /// Returns true when the stream is empty (but still open)
    fn empty(&self) -> bool;
    /// REturns true when the stream is closed and empty
    fn eof(&self) -> bool;
    /// Skip amount of characters in the stream
    fn skip(&mut self, offset: usize);
    /// Returns the current offset in the stream
    fn tell(&self) -> usize;
    /// Returns the length of the stream
    fn length(&self) -> usize;
    /// Returns the number of characters left in the stream
    fn chars_left(&self) -> usize;
    /// Reads a character and increases the current pointer
    fn read_char(&mut self) -> Character;
    /// Reads the current character
    fn current_char(&self) -> Character;
    /// Reads the next character
    fn next_char(&self) -> Character;
    /// Looks ahead in the stream
    fn look_ahead(&self, offset: usize) -> Character;
    /// Looks ahead in the stream and returns len characters
    fn look_ahead_slice(&self, len: usize) -> String;
    /// Unreads the current character
    fn unread(&mut self);
}

impl Default for ByteStream {
    fn default() -> Self {
        Self::new()
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

    /// Closes the stream so no more data can be added
    pub fn close_stream(&mut self) {
        self.closed = true;
    }

    /// Returns true when the stream is closed and no more input can be read after this buffer
    /// is emptied
    pub fn closed(&self) -> bool {
        self.closed
    }

    /// Returns true when the buffer is empty and there is no more input to read
    pub fn empty(&self) -> bool {
        self.buffer_pos >= self.buffer.len()
    }

    /// Returns true when the stream is closed and all the bytes have been read
    pub fn eof(&self) -> bool {
        self.closed() && self.empty()
    }

    /// Skip offset characters in the stream (based on chars)
    pub fn skip(&mut self, offset: usize) {
        let mut skip_len = offset;
        if self.buffer_pos + offset >= self.buffer.len() {
            skip_len = self.buffer.len() - self.buffer_pos;
        }

        for _ in 0..skip_len {
            self.read_char();
        }
    }

    /// Returns the current offset in the stream
    pub fn tell(&self) -> usize {
        self.buffer_pos
    }

    /// Returns the length of the buffer
    pub fn length(&self) -> usize {
        self.buffer.len()
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
        self.close_stream();
        self.force_set_encoding(e.unwrap_or(Encoding::UTF8));
        self.reset_stream();
        Ok(())
    }

    pub fn reset_stream(&mut self) {
        self.buffer_pos = 0;
    }

    /// Populates the current buffer with the contents of given file f
    pub fn read_from_file(&mut self, mut f: impl Read, e: Option<Encoding>) -> io::Result<()> {
        // First we read the u8 bytes into a buffer
        f.read_to_end(&mut self.u8_buffer).expect("uh oh");
        self.close_stream();
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

    /// Returns the number of characters left in the buffer
    #[cfg(test)]
    fn chars_left(&self) -> usize {
        self.buffer.len() - self.buffer_pos
    }

    /// Reads a character and increases the current pointer, or read EOF as None
    pub(crate) fn read_char(&mut self) -> Character {
        // If we still can move forward in the stream, move forwards
        if self.buffer_pos < self.buffer.len() {
            let c = self.buffer[self.buffer_pos];
            self.buffer_pos += 1;
            return c;
        }

        // Return none if we already have read EOF
        if self.eof() {
            return StreamEnd;
        }

        StreamEmpty
    }

    /// Reads the current character
    pub(crate) fn current_char(&self) -> Character {
        self.look_ahead(0)
    }

    /// Reads the next character
    pub(crate) fn next_char(&self) -> Character {
        self.look_ahead(1)
    }

    pub(crate) fn unread(&mut self) {
        if self.buffer_pos > 0 {
            self.buffer_pos -= 1;
        }
    }

    /// Looks ahead in the stream and returns len characters
    pub(crate) fn look_ahead_slice(&self, len: usize) -> String {
        let end_pos = std::cmp::min(self.buffer.len(), self.buffer_pos + len);

        let slice = &self.buffer[self.buffer_pos..end_pos];
        slice.iter().map(ToString::to_string).collect()
    }

    /// Looks ahead in the stream, can use an optional index if we want to seek further
    /// (or back) in the stream.
    pub(crate) fn look_ahead(&self, offset: usize) -> Character {
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
        let mut chars = ByteStream::new();
        assert!(chars.eof());

        chars.read_from_str("foo", Some(Encoding::ASCII));
        assert_eq!(chars.length(), 3);
        assert!(!chars.eof());
        assert_eq!(chars.chars_left(), 3);

        chars.read_from_str("f👽f", Some(Encoding::UTF8));
        assert_eq!(chars.length(), 3);
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

        chars.reset_stream();
        chars.set_encoding(Encoding::ASCII);
        assert_eq!(chars.length(), 6);
        assert_eq!(chars.read_char(), Ch('f'));
        assert_eq!(chars.read_char(), Ch('?'));
        assert_eq!(chars.read_char(), Ch('?'));
        assert_eq!(chars.read_char(), Ch('?'));
        assert_eq!(chars.read_char(), Ch('?'));
        assert_eq!(chars.read_char(), Ch('f'));
        assert!(matches!(chars.read_char(), StreamEnd));

        chars.unread(); // unread eof
        chars.unread(); // unread 'f'
        chars.unread(); // Unread '?'
        assert_eq!(chars.chars_left(), 2);
        chars.unread();
        assert_eq!(chars.chars_left(), 3);

        chars.reset_stream();
        assert_eq!(chars.chars_left(), 6);
        chars.unread();
        assert_eq!(chars.chars_left(), 6);

        chars.read_from_str("abc", Some(Encoding::UTF8));
        chars.reset_stream();
        assert_eq!(chars.read_char(), Ch('a'));
        chars.unread();
        assert_eq!(chars.read_char(), Ch('a'));
        assert_eq!(chars.read_char(), Ch('b'));
        chars.unread();
        assert_eq!(chars.read_char(), Ch('b'));
        assert_eq!(chars.read_char(), Ch('c'));
        chars.unread();
        assert_eq!(chars.read_char(), Ch('c'));
        assert!(matches!(chars.read_char(), StreamEnd));
        chars.unread();
        assert!(matches!(chars.read_char(), StreamEnd));
    }

    #[test]
    fn test_certainty() {
        let mut chars = ByteStream::new();
        assert!(!chars.is_certain_encoding());

        chars.set_confidence(Confidence::Certain);
        assert!(chars.is_certain_encoding());

        chars.set_confidence(Confidence::Tentative);
        assert!(!chars.is_certain_encoding());
    }

    #[test]
    fn test_eof() {
        let mut chars = ByteStream::new();
        chars.read_from_str("abc", Some(Encoding::UTF8));
        assert_eq!(chars.length(), 3);
        assert_eq!(chars.chars_left(), 3);
        assert_eq!(chars.read_char(), Ch('a'));
        assert_eq!(chars.read_char(), Ch('b'));
        assert_eq!(chars.read_char(), Ch('c'));
        assert!(matches!(chars.read_char(), StreamEnd));
        assert!(matches!(chars.read_char(), StreamEnd));
        assert!(matches!(chars.read_char(), StreamEnd));
        assert!(matches!(chars.read_char(), StreamEnd));
        chars.unread();
        assert!(matches!(chars.read_char(), StreamEnd));
        chars.unread();
        chars.unread();
        assert!(!matches!(chars.read_char(), StreamEnd));
        assert!(matches!(chars.read_char(), StreamEnd));
        chars.unread();
        chars.unread();
        assert!(!matches!(chars.read_char(), StreamEnd));
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
        assert!(matches!(chars.read_char(), StreamEnd));
        chars.unread();
        chars.unread();
        assert_eq!(chars.read_char(), Ch('c'));
        assert!(matches!(chars.read_char(), StreamEnd));
        chars.unread();
        assert!(matches!(chars.read_char(), StreamEnd));
    }


    #[test]
    fn stream_closing() {
        let mut chars = ByteStream::new();
        chars.read_from_str("abc", Some(Encoding::UTF8));
        assert_eq!(chars.length(), 3);
        assert_eq!(chars.chars_left(), 3);
        assert_eq!(chars.read_char(), Ch('a'));
        assert_eq!(chars.read_char(), Ch('b'));
        assert_eq!(chars.read_char(), Ch('c'));
        assert!(matches!(chars.read_char(), StreamEmpty));
        assert!(matches!(chars.read_char(), StreamEmpty));

        chars.append_str("def", Some(Encoding::UTF8));
        assert_eq!(chars.length(), 6);
        assert_eq!(chars.chars_left(), 3);
        assert_eq!(chars.read_char(), Ch('d'));
        assert_eq!(chars.read_char(), Ch('e'));
        assert_eq!(chars.read_char(), Ch('f'));
        assert!(matches!(chars.read_char(), StreamEmpty));

        chars.append_str("ghi", Some(Encoding::UTF8));
        chars.close_stream();
        assert_eq!(chars.length(), 9);
        assert_eq!(chars.chars_left(), 3);
        assert_eq!(chars.read_char(), Ch('g'));
        assert_eq!(chars.read_char(), Ch('h'));
        assert_eq!(chars.read_char(), Ch('i'));
        assert!(matches!(chars.read_char(), StreamEnd));
    }
}
