//! `URL`/`URLSearchParams` as described by <https://url.spec.whatwg.org/>,
//! wrapping the `url` crate — rust-url implements the URL Standard, and its
//! `quirks` module exposes the exact JS-attribute semantics.

use std::cmp::Ordering;
use std::error::Error;
use std::fmt;

use ::url::quirks;

/// Parse failure. The `Display` text carries the JS error class prefix
/// (`TypeError: ...`) — the same rethrow protocol as `DomException` and
/// `EncodingError`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UrlError(String);

impl fmt::Display for UrlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TypeError: {}", self.0)
    }
}

impl Error for UrlError {}

/// A parsed URL with the JS-attribute accessors of the `URL` interface.
/// Setters other than `href` are infallible per spec — invalid input leaves
/// the URL unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Url {
    inner: ::url::Url,
}

impl Url {
    pub fn parse(input: &str, base: Option<&str>) -> Result<Self, UrlError> {
        let base_url = match base {
            Some(b) => Some(::url::Url::parse(b).map_err(|e| UrlError(format!("invalid base URL {b:?}: {e}")))?),
            None => None,
        };

        ::url::Url::options()
            .base_url(base_url.as_ref())
            .parse(input)
            .map(|inner| Self { inner })
            .map_err(|e| UrlError(format!("invalid URL {input:?}: {e}")))
    }

    #[must_use]
    pub fn can_parse(input: &str, base: Option<&str>) -> bool {
        Self::parse(input, base).is_ok()
    }

    #[must_use]
    pub fn href(&self) -> &str {
        quirks::href(&self.inner)
    }

    /// The one throwing setter: assigning an unparsable href is a TypeError
    pub fn set_href(&mut self, value: &str) -> Result<(), UrlError> {
        quirks::set_href(&mut self.inner, value).map_err(|e| UrlError(format!("invalid URL {value:?}: {e}")))
    }

    #[must_use]
    pub fn origin(&self) -> String {
        quirks::origin(&self.inner)
    }

    #[must_use]
    pub fn protocol(&self) -> &str {
        quirks::protocol(&self.inner)
    }

    pub fn set_protocol(&mut self, value: &str) {
        let _ = quirks::set_protocol(&mut self.inner, value);
    }

    #[must_use]
    pub fn username(&self) -> &str {
        quirks::username(&self.inner)
    }

    pub fn set_username(&mut self, value: &str) {
        let _ = quirks::set_username(&mut self.inner, value);
    }

    #[must_use]
    pub fn password(&self) -> &str {
        quirks::password(&self.inner)
    }

    pub fn set_password(&mut self, value: &str) {
        let _ = quirks::set_password(&mut self.inner, value);
    }

    #[must_use]
    pub fn host(&self) -> &str {
        quirks::host(&self.inner)
    }

    pub fn set_host(&mut self, value: &str) {
        let _ = quirks::set_host(&mut self.inner, value);
    }

    #[must_use]
    pub fn hostname(&self) -> &str {
        quirks::hostname(&self.inner)
    }

    pub fn set_hostname(&mut self, value: &str) {
        let _ = quirks::set_hostname(&mut self.inner, value);
    }

    #[must_use]
    pub fn port(&self) -> &str {
        quirks::port(&self.inner)
    }

    pub fn set_port(&mut self, value: &str) {
        let _ = quirks::set_port(&mut self.inner, value);
    }

    #[must_use]
    pub fn pathname(&self) -> &str {
        quirks::pathname(&self.inner)
    }

    pub fn set_pathname(&mut self, value: &str) {
        quirks::set_pathname(&mut self.inner, value);
    }

    #[must_use]
    pub fn search(&self) -> &str {
        quirks::search(&self.inner)
    }

    pub fn set_search(&mut self, value: &str) {
        quirks::set_search(&mut self.inner, value);
    }

    #[must_use]
    pub fn hash(&self) -> &str {
        quirks::hash(&self.inner)
    }

    pub fn set_hash(&mut self, value: &str) {
        quirks::set_hash(&mut self.inner, value);
    }

    /// The raw query component (no leading `?`), for `URLSearchParams` linkage
    #[must_use]
    pub fn query(&self) -> Option<&str> {
        self.inner.query()
    }

    /// The `URLSearchParams` update steps' empty-serialization case: null the
    /// query, percent-encoding a lone trailing space of an opaque path first
    /// so the serialization still round-trips (the `search` setter's
    /// strip-all-spaces rule must not apply here).
    pub fn clear_query_for_params(&mut self) {
        if self.inner.cannot_be_a_base() {
            let path = self.inner.path();
            if let Some(stripped) = path.strip_suffix(' ') {
                let fixed = format!("{stripped}%20");
                self.inner.set_path(&fixed);
            }
        }
        self.inner.set_query(None);
    }
}

impl fmt::Display for Url {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.href())
    }
}

/// The `URLSearchParams` list: ordered name/value pairs with
/// application/x-www-form-urlencoded parsing and serialization.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct UrlSearchParams {
    list: Vec<(String, String)>,
}

impl UrlSearchParams {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse a query string (without leading `?` — the JS constructor strips it)
    #[must_use]
    pub fn parse_query(query: &str) -> Self {
        Self {
            list: ::url::form_urlencoded::parse(query.as_bytes())
                .map(|(name, value)| (name.into_owned(), value.into_owned()))
                .collect(),
        }
    }

    pub fn reset_from_query(&mut self, query: &str) {
        *self = Self::parse_query(query);
    }

    pub fn append(&mut self, name: &str, value: &str) {
        self.list.push((name.to_owned(), value.to_owned()));
    }

    /// Remove entries matching the name — and the value too, if given
    pub fn delete(&mut self, name: &str, value: Option<&str>) {
        self.list
            .retain(|(n, v)| !(n == name && value.is_none_or(|value| v == value)));
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&str> {
        self.list.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }

    #[must_use]
    pub fn get_all(&self, name: &str) -> Vec<&str> {
        self.list
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
            .collect()
    }

    #[must_use]
    pub fn has(&self, name: &str, value: Option<&str>) -> bool {
        self.list
            .iter()
            .any(|(n, v)| n == name && value.is_none_or(|value| v == value))
    }

    /// Set the first matching entry's value and drop the other matches, or
    /// append if the name is absent
    pub fn set(&mut self, name: &str, value: &str) {
        let mut found = false;
        self.list.retain_mut(|(n, v)| {
            if n != name {
                return true;
            }
            if found {
                return false;
            }
            found = true;
            value.clone_into(v);
            true
        });

        if !found {
            self.append(name, value);
        }
    }

    /// Stable sort by name, comparing UTF-16 code units per spec
    pub fn sort(&mut self) {
        self.list.sort_by(|a, b| utf16_cmp(&a.0, &b.0));
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.list.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    #[must_use]
    pub fn entries(&self) -> &[(String, String)] {
        &self.list
    }

    #[must_use]
    pub fn to_query_string(&self) -> String {
        let mut serializer = ::url::form_urlencoded::Serializer::new(String::new());
        for (name, value) in &self.list {
            serializer.append_pair(name, value);
        }
        serializer.finish()
    }
}

fn utf16_cmp(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_accessors() {
        let u = Url::parse("https://user:pw@example.com:8080/p/q?a=1#frag", None).unwrap();
        assert_eq!(u.href(), "https://user:pw@example.com:8080/p/q?a=1#frag");
        assert_eq!(u.protocol(), "https:");
        assert_eq!(u.username(), "user");
        assert_eq!(u.password(), "pw");
        assert_eq!(u.host(), "example.com:8080");
        assert_eq!(u.hostname(), "example.com");
        assert_eq!(u.port(), "8080");
        assert_eq!(u.pathname(), "/p/q");
        assert_eq!(u.search(), "?a=1");
        assert_eq!(u.hash(), "#frag");
        assert_eq!(u.origin(), "https://example.com:8080");
    }

    #[test]
    fn base_resolution() {
        let u = Url::parse("/path", Some("http://example.org/dir/file")).unwrap();
        assert_eq!(u.href(), "http://example.org/path");
        assert!(Url::parse("/path", None).is_err());
        assert!(Url::can_parse("/path", Some("http://example.org")));
        assert!(!Url::can_parse("/path", None));
    }

    #[test]
    fn errors_carry_typeerror_prefix() {
        let err = Url::parse("http://exa mple.org", None).unwrap_err();
        assert!(err.to_string().starts_with("TypeError:"));
    }

    #[test]
    fn invalid_setter_input_is_ignored() {
        let mut u = Url::parse("https://example.com/", None).unwrap();
        u.set_port("bogus");
        assert_eq!(u.port(), "");
        u.set_port("81");
        assert_eq!(u.port(), "81");
    }

    #[test]
    fn search_params_parse_and_serialize() {
        let sp = UrlSearchParams::parse_query("a=1&b=2&a=3");
        assert_eq!(sp.get("a"), Some("1"));
        assert_eq!(sp.get_all("a"), vec!["1", "3"]);
        assert_eq!(sp.len(), 3);
        assert_eq!(sp.to_query_string(), "a=1&b=2&a=3");

        // Space/plus and percent-decoding round-trip
        let sp = UrlSearchParams::parse_query("a+b=c%20d&e=%26");
        assert_eq!(sp.get("a b"), Some("c d"));
        assert_eq!(sp.get("e"), Some("&"));
        assert_eq!(sp.to_query_string(), "a+b=c+d&e=%26");
    }

    #[test]
    fn set_replaces_first_and_drops_rest() {
        let mut sp = UrlSearchParams::parse_query("a=1&b=2&a=3");
        sp.set("a", "9");
        assert_eq!(sp.to_query_string(), "a=9&b=2");
        sp.set("c", "1");
        assert_eq!(sp.to_query_string(), "a=9&b=2&c=1");
    }

    #[test]
    fn delete_with_and_without_value() {
        let mut sp = UrlSearchParams::parse_query("a=1&a=2&b=3");
        sp.delete("a", Some("1"));
        assert_eq!(sp.to_query_string(), "a=2&b=3");
        sp.delete("a", None);
        assert_eq!(sp.to_query_string(), "b=3");
    }

    #[test]
    fn sort_uses_utf16_code_units() {
        // U+1D306 (surrogate pair, first unit 0xD834) sorts after U+FFFD in
        // code-point order but before it in UTF-16 code-unit order
        let mut sp = UrlSearchParams::new();
        sp.append("\u{FFFD}", "1");
        sp.append("\u{1D306}", "2");
        sp.append("a", "3");
        sp.sort();
        let names: Vec<&str> = sp.entries().iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["a", "\u{1D306}", "\u{FFFD}"]);
    }
}
