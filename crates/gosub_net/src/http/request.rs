use crate::http::headers::Headers;
use cookie::CookieJar;
use core::fmt::{Display, Formatter};

#[derive(Debug, Default, Clone)]
pub struct Request {
    pub method: String,
    pub uri: String,
    pub version: String,
    pub headers: Headers,
    pub cookies: CookieJar,
    pub body: Vec<u8>,
}

impl Request {
    pub fn new(method: &str, uri: &str, version: &str) -> Self {
        Self {
            method: method.to_string(),
            uri: uri.to_string(),
            version: version.to_string(),
            headers: Headers::default(),
            cookies: CookieJar::default(),
            body: vec![],
        }
    }

    pub fn headers(&mut self, headers: Headers) {
        self.headers = headers;
    }

    pub fn cookies(&mut self, cookies: CookieJar) {
        self.cookies = cookies;
    }
}

impl Display for Request {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        writeln!(f, "{} {} {}", self.method, self.uri, self.version)?;
        writeln!(f, "Headers:")?;
        for (key, value) in self.headers.sorted() {
            writeln!(f, "  {}: {}", key, value)?;
        }
        writeln!(f, "Cookies:")?;
        let mut sorted_cookies = self.cookies.iter().collect::<Vec<_>>();
        sorted_cookies.sort_by(|a, b| a.name().cmp(b.name()));
        for cookie in sorted_cookies {
            writeln!(f, "  {}", cookie)?;
        }
        writeln!(f, "Body: {} bytes", self.body.len())?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cookie::Cookie;

    #[test]
    fn test_request() {
        let mut req = Request::new("GET", "/", "HTTP/1.1");
        req.headers(Headers::new());
        req.cookies(CookieJar::new());

        req.headers.set_str("Content-Type", "application/json");
        req.cookies.add(Cookie::new("qux", "wok"));
        req.cookies.add(Cookie::new("foo", "bar"));
        req.headers.set_str("Accept", "text/html");
        req.headers.set_str("Accept-Encoding", "gzip, deflate, br");

        assert_eq!(req.method, "GET");
        assert_eq!(req.uri, "/");
        assert_eq!(req.version, "HTTP/1.1");
        assert_eq!(req.headers.all().len(), 3);
        assert_eq!(req.cookies.iter().count(), 2);
    }

    #[test]
    fn test_request_display() {
        let mut req = Request::new("GET", "/", "HTTP/1.1");
        req.headers(Headers::new());
        req.cookies(CookieJar::new());

        req.cookies.add(Cookie::new("foo", "bar"));
        req.cookies.add(Cookie::new("qux", "wok"));
        req.headers.set_str("Content-Type", "application/json");
        req.headers.set_str("Accept", "text/html");
        req.headers.set_str("Accept-Encoding", "gzip, deflate, br");

        let s = format!("{}", req);
        assert_eq!(s, "GET / HTTP/1.1\nHeaders:\n  Accept: text/html\n  Accept-Encoding: gzip, deflate, br\n  Content-Type: application/json\nCookies:\n  foo=bar\n  qux=wok\nBody: 0 bytes\n");
    }
}
