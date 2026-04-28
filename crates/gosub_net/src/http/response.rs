use core::fmt::{Display, Formatter};
use std::collections::HashMap;

#[derive(Debug, Default)]
pub struct Response {
    pub status: u16,
    pub status_text: String,
    pub version: String,
    pub headers: HashMap<String, String>,
    pub cookies: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl Response {
    #[must_use]
    pub fn new() -> Response {
        Self {
            version: "HTTP/1.1".to_string(),
            ..Default::default()
        }
    }

    #[must_use]
    pub fn is_ok(&self) -> bool {
        self.status >= 200 && self.status < 300
    }
}

impl From<Vec<u8>> for Response {
    fn from(body: Vec<u8>) -> Self {
        Self {
            status: 200,
            status_text: "OK".to_string(),
            version: "HTTP/1.1".to_string(),
            body,
            ..Default::default()
        }
    }
}

impl Display for Response {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        writeln!(f, "HTTP/1.1 {}", self.status)?;
        writeln!(f, "Headers:")?;
        for (key, value) in &self.headers {
            writeln!(f, "  {key}: {value}")?;
        }
        writeln!(f, "Cookies:")?;
        for (key, value) in &self.cookies {
            writeln!(f, "  {key}: {value}")?;
        }
        writeln!(f, "Body: {} bytes", self.body.len())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response() {
        let response = Response::new();
        let s = format!("{response}");
        assert_eq!(s, "HTTP/1.1 0\nHeaders:\nCookies:\nBody: 0 bytes\n");
    }
}
