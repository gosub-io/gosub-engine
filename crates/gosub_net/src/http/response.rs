use core::fmt::{Display, Formatter};
use std::collections::HashMap;

use crate::http::headers::Headers;

#[derive(Debug)]
pub struct Response {
    pub status: u16,
    pub status_text: String,
    pub version: String,
    pub headers: Headers,
    pub cookies: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn new() -> Response {
        Self {
            status: 0,
            status_text: "".to_string(),
            version: "HTTP/1.1".to_string(),
            headers: Default::default(),
            cookies: Default::default(),
            body: vec![],
        }
    }

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
            headers: Default::default(),
            cookies: Default::default(),
            body,
        }
    }
}

impl Default for Response {
    fn default() -> Self {
        Self::new()
    }
}

impl Display for Response {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        writeln!(f, "HTTP/1.1 {}", self.status)?;
        writeln!(f, "Headers:")?;
        for (key, value) in self.headers.all() {
            writeln!(f, "  {}: {}", key, value)?;
        }
        writeln!(f, "Cookies:")?;
        for (key, value) in &self.cookies {
            writeln!(f, "  {}: {}", key, value)?;
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
        let mut response = Response::new();

        let s = format!("{}", response);
        assert_eq!(s, "HTTP/1.1 0\nHeaders:\nCookies:\nBody: 0 bytes\n");

        response.status = 200;
        response.headers.set_str("Content-Type", "application/json");
        response.cookies.insert("session".to_string(), "1234567890".to_string());
        response.body = b"Hello, world!".to_vec();

        let s = format!("{}", response);
        assert_eq!(s, "HTTP/1.1 200\nHeaders:\n  Content-Type: application/json\nCookies:\n  session: 1234567890\nBody: 13 bytes\n");
    }
}
