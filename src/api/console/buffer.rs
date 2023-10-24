use crate::types::Result;
use std::io::Write;

/// A buffer that can be written to and then converted to a string
pub struct Buffer {
    /// Internal buffer
    buf: Vec<u8>,
}

impl Buffer {
    /// Creates a new buffer
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    /// Converts the buffer to a String
    pub fn to_string(&self) -> Result<String> {
        Ok(String::from_utf8(self.buf.clone())?)
    }
}

impl Write for Buffer {
    /// Writes the given data to the buffer
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.buf.extend(buf);
        Ok(buf.len())
    }

    /// Flushes the buffer by clearing it
    fn flush(&mut self) -> std::io::Result<()> {
        self.buf.clear();
        Ok(())
    }
}
