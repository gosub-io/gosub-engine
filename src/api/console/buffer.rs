use crate::types::Result;
use std::io::Write;

pub struct Buffer {
    buf: Vec<u8>,
}

impl Buffer {
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    pub fn to_string(&self) -> Result<String> {
        Ok(String::from_utf8(self.buf.clone())?)
    }
}

impl Write for Buffer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.buf.extend(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.buf.clear();
        Ok(())
    }
}
