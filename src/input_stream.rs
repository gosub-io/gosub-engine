use std::fs::File;
use std::io::Read;

// Encoding defines the way the buffer stream is read, as what defines a "character".
pub enum Encoding {
    UTF8,           // Stream is of UTF8 characters
    ASCII,          // Stream is of 8bit ASCII
    Iso88591        // Stream is of iso_8859_1
    // More
}

// The confidence decides how confident we are that the input stream is of this encoding
pub enum Confidence {
    Tentative,          // This encoding might be the one we need
    Certain,            // We are certain to use this encoding
    Irrelevant          // There is no content encoding for this stream
}
pub struct InputStream {
    encoding: Encoding,     // Current encoding
    confidence: Confidence, // How confident are we that this is the correct encoding?
    current: usize,         // Current offset of the reader
    length: usize,          // Length (in bytes) of the buffer
    buffer: Vec<char>       // Reference to the actual buffer stream
}

impl InputStream {
    pub fn new() -> Self {
        InputStream {
            encoding: Encoding::UTF8,
            confidence: Confidence::Tentative,
            current: 0,
            length: 0,
            buffer: Vec::new(),
        }
    }

    pub fn eof(&self) -> bool
    {
        self.current >= self.length
    }

    // Reset the reader back to the start
    pub fn reset(&mut self)
    {
        self.current = 0
    }

    // Seek explicit offset in the stream (based on chars)
    pub fn seek(&mut self, mut off: usize)
    {
        if off > self.length {
            off = self.length
        }

        self.current = off
    }

    // Sets the encoding and confidence for this stream. Doesn't do anything yet, as we always assume
    // UTF8 (or ASCII)
    pub fn set_encoding(&mut self, e: Encoding, c: Confidence)
    {
        self.encoding = e;
        self.confidence = c;
    }

    // Populates the current buffer with the contents of given file f
    pub fn read_from_file(&mut self, mut f: File) -> std::io::Result<()> {
        // First we read the u8 bytes into a buffer
        let mut u8_buf = Vec::new();
        let i = f.read_to_end(&mut u8_buf).expect("uh oh");

        // Convert the u8 buffer into utf8 string
        let str_buf = std::str::from_utf8(&u8_buf).map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, e)
        })?;

        // Convert the utf8 string into characters so we can use easy indexing
        self.buffer = str_buf.chars().collect();
        self.length = self.buffer.len();

        Ok(())
    }

    pub fn read_char(&mut self) -> char
    {
        if self.eof() {
            return 0x0 as char;
        }

        let c = self.buffer[self.current];
        self.current+=1;
        c
    }
}