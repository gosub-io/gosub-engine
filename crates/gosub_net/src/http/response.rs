use crate::http::headers::Headers;
use core::fmt::{Display, Formatter};
use std::collections::HashMap;
use std::io::Read;

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

impl TryFrom<ureq::Response> for Response {
    type Error = anyhow::Error;

    fn try_from(value: ureq::Response) -> std::result::Result<Self, Self::Error> {
        let body = Vec::with_capacity(
            value
                .header("Content-Length")
                .map(|s| s.parse().unwrap_or(0))
                .unwrap_or(0),
        );

        let mut this = Self {
            status: value.status(),
            status_text: value.status_text().to_string(),
            version: value.http_version().to_string(),
            headers: get_headers(&value),
            body,
            cookies: Default::default(),
        };

        value.into_reader().read_to_end(&mut this.body)?;

        Ok(this)
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

fn get_headers(response: &ureq::Response) -> Headers {
    let names = response.headers_names();

    let mut headers = Headers::with_capacity(names.len());

    for name in names {
        let header = response.header(&name).unwrap_or_default().to_string();

        headers.set(name, header);
    }

    headers
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
        response
            .cookies
            .insert("session".to_string(), "1234567890".to_string());
        response.body = b"Hello, world!".to_vec();

        let s = format!("{}", response);
        assert_eq!(s, "HTTP/1.1 200\nHeaders:\n  Content-Type: application/json\nCookies:\n  session: 1234567890\nBody: 13 bytes\n");
    }
}
