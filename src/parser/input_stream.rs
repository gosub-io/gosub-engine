use std::fs::File;
use std::io::Read;

// Encoding defines the way the buffer stream is read, as what defines a "character".
#[derive(PartialEq)]
pub enum Encoding {
    UTF8,           // Stream is of UTF8 characters
    ASCII,          // Stream is of 8bit ASCII
    // Iso88591        // Stream is of iso_8859_1
    // More
}

// The confidence decides how confident we are that the input stream is of this encoding
#[derive(PartialEq)]
pub enum Confidence {
    Tentative,          // This encoding might be the one we need
    Certain,            // We are certain to use this encoding
    // Irrelevant          // There is no content encoding for this stream
}
pub struct InputStream {
    encoding: Encoding,                 // Current encoding
    pub(crate) confidence: Confidence,  // How confident are we that this is the correct encoding?
    current: usize,                     // Current offset of the reader
    length: usize,                      // Length (in bytes) of the buffer
    buffer: Vec<char>,                  // Reference to the actual buffer stream in characters
    u8_buffer: Vec<u8>                  // Reference to the actual buffer stream in u8 bytes
}

impl InputStream {
    pub fn new() -> Self {
        InputStream {
            encoding: Encoding::UTF8,
            confidence: Confidence::Tentative,
            current: 0,
            length: 0,
            buffer: Vec::new(),
            u8_buffer: Vec::new(),
        }
    }

    pub fn is_certain_encoding(&self) -> bool {
        self.confidence == Confidence::Certain
    }

    pub fn detect_encoding(&self) {
        todo!()
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

    pub fn set_confidence(&mut self, c: Confidence)
    {
        self.confidence = c;
    }

    // Changes the encoding and if necessary, decodes the u8 buffer into the correct encoding
    pub fn set_encoding(&mut self, e: Encoding)
    {
        // Don't convert if the encoding is the same as it already is
        if self.encoding == e {
            return
        }

        self.force_set_encoding(e)
    }

    // Sets the encoding for this stream, and decodes the u8_buffer into the buffer with the
    // correct encoding.
    pub fn force_set_encoding(&mut self, e: Encoding) {
        match e {
            Encoding::UTF8 => {
                // Convert the u8 buffer into utf8 string
                let str_buf = std::str::from_utf8(&self.u8_buffer).unwrap();

                // Convert the utf8 string into characters so we can use easy indexing
                self.buffer = str_buf.chars().collect();
                self.length = self.buffer.len();
            }
            Encoding::ASCII => {
                // Convert the string into characters so we can use easy indexing. Any non-ascii chars (> 0x7F) are converted to '?'
                self.buffer = self.u8_buffer.iter().map(|&byte| if byte.is_ascii() { byte as char } else { '?' }).collect();
                self.length = self.buffer.len();
            }
            _ => {
                // @TODO: we probably want to do something with the other encodings
            }
        }

        self.encoding = e;
    }

    // Populates the current buffer with the contents of given file f
    pub fn read_from_file(&mut self, mut f: File, e: Option<Encoding>) -> std::io::Result<()> {
        // First we read the u8 bytes into a buffer
        f.read_to_end(&mut self.u8_buffer).expect("uh oh");

        self.force_set_encoding(e.unwrap_or(Encoding::UTF8));
        Ok(())
    }

    // Reads a character and increases the current pointer
    pub fn read_char(&mut self) -> char
    {
        if self.eof() {
            return 0x0 as char;
        }

        let c = self.buffer[self.current];
        self.current+=1;
        c
    }

    // Returns the number of characters left in the buffer
    pub fn chars_left(&self) -> usize {
        self.length - self.current
    }

    // Looks ahead in the stream, can use an optional index if we want to seek further (or back) in the stream
    pub fn look_char(&self, idx: Option<i32>) -> char {
        if self.current < idx.unwrap_or(0) as usize {
            return 0x0 as char;
        }

        if self.current + idx.unwrap_or(0) as usize > self.length {
            return 0x0 as char;
        }

        self.buffer[self.current + idx.unwrap_or(0) as usize]
    }
}