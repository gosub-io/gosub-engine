use gosub_shared::types::Result;
use std::io::Write;

/// A buffer that can be written to and then converted to a string
#[derive(Default)]
pub struct Buffer {
    /// Internal buffer
    buf: Vec<u8>,
}

impl Buffer {
    /// Creates a new buffer
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self::default()
    }

    /// Converts the buffer to a String
    #[allow(dead_code)]
    pub fn try_to_string(&self) -> Result<String> {
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
