//! `Headers` as described by <https://fetch.spec.whatwg.org/#headers-class>:
//! an ordered header list with byte-case-insensitive names, value
//! normalization, and sort-and-combine iteration.

use cow_utils::CowUtils;
use std::error::Error;
use std::fmt;

/// Validation failure. `Display` carries the `TypeError:` prefix — the same
/// rethrow protocol as the other jsapi modules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeadersError {
    InvalidName(String),
    InvalidValue(String),
    /// The guard forbids this mutation
    Immutable,
}

impl fmt::Display for HeadersError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidName(name) => write!(f, "TypeError: {name:?} is not a valid header name"),
            Self::InvalidValue(value) => write!(f, "TypeError: {value:?} is not a valid header value"),
            Self::Immutable => f.write_str("TypeError: headers are immutable"),
        }
    }
}

impl Error for HeadersError {}

/// The headers guard. Constructed `Headers` objects always get `None`; the
/// other variants exist for future Request/Response integration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HeadersGuard {
    #[default]
    None,
    Immutable,
    Request,
    RequestNoCors,
    Response,
}

/// An HTTP token per RFC 9110: the only characters legal in a header name
fn is_token(name: &str) -> bool {
    !name.is_empty()
        && name.bytes().all(|b| {
            matches!(
                b,
                b'!' | b'#' | b'$' | b'%' | b'&' | b'\'' | b'*' | b'+' | b'-' | b'.' | b'^' | b'_' | b'`' | b'|' | b'~'
            ) || b.is_ascii_alphanumeric()
        })
}

/// Strip leading/trailing HTTP whitespace (the spec's "normalize")
fn normalize_value(value: &str) -> &str {
    value.trim_matches(|c| matches!(c, '\t' | '\n' | '\r' | ' '))
}

/// A normalized value must not contain NUL, CR or LF
fn is_valid_value(value: &str) -> bool {
    !value.chars().any(|c| matches!(c, '\0' | '\r' | '\n'))
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Headers {
    guard: HeadersGuard,
    /// Names stored byte-lowercased; nothing observable exposes original casing
    list: Vec<(String, String)>,
}

impl Headers {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_guard(guard: HeadersGuard) -> Self {
        Self {
            guard,
            list: Vec::new(),
        }
    }

    fn check_mutable(&self) -> Result<(), HeadersError> {
        // Request/response guards additionally filter forbidden header names;
        // only the immutable guard matters until Request/Response exist
        if self.guard == HeadersGuard::Immutable {
            return Err(HeadersError::Immutable);
        }
        Ok(())
    }

    fn validate(name: &str, value: &str) -> Result<(String, String), HeadersError> {
        if !is_token(name) {
            return Err(HeadersError::InvalidName(name.to_owned()));
        }
        let value = normalize_value(value);
        if !is_valid_value(value) {
            return Err(HeadersError::InvalidValue(value.to_owned()));
        }
        Ok((name.cow_to_ascii_lowercase().into_owned(), value.to_owned()))
    }

    pub fn append(&mut self, name: &str, value: &str) -> Result<(), HeadersError> {
        let (name, value) = Self::validate(name, value)?;
        self.check_mutable()?;
        self.list.push((name, value));
        Ok(())
    }

    pub fn delete(&mut self, name: &str) -> Result<(), HeadersError> {
        if !is_token(name) {
            return Err(HeadersError::InvalidName(name.to_owned()));
        }
        self.check_mutable()?;
        let name = name.cow_to_ascii_lowercase();
        self.list.retain(|(n, _)| *n != name);
        Ok(())
    }

    /// The combined value: all values for the name joined with ", "
    pub fn get(&self, name: &str) -> Result<Option<String>, HeadersError> {
        if !is_token(name) {
            return Err(HeadersError::InvalidName(name.to_owned()));
        }
        let name = name.cow_to_ascii_lowercase();
        let values: Vec<&str> = self
            .list
            .iter()
            .filter(|(n, _)| *n == name)
            .map(|(_, v)| v.as_str())
            .collect();
        if values.is_empty() {
            return Ok(None);
        }
        Ok(Some(values.join(", ")))
    }

    #[must_use]
    pub fn get_set_cookie(&self) -> Vec<&str> {
        self.list
            .iter()
            .filter(|(n, _)| n == "set-cookie")
            .map(|(_, v)| v.as_str())
            .collect()
    }

    pub fn has(&self, name: &str) -> Result<bool, HeadersError> {
        if !is_token(name) {
            return Err(HeadersError::InvalidName(name.to_owned()));
        }
        let name = name.cow_to_ascii_lowercase();
        Ok(self.list.iter().any(|(n, _)| *n == name))
    }

    /// Replace the first matching header's value and drop other matches, or
    /// append if the name is absent
    pub fn set(&mut self, name: &str, value: &str) -> Result<(), HeadersError> {
        let (name, value) = Self::validate(name, value)?;
        self.check_mutable()?;

        let mut found = false;
        self.list.retain_mut(|(n, v)| {
            if *n != name {
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
            self.list.push((name, value));
        }
        Ok(())
    }

    /// The iteration view: names sorted, values per name combined with ", " —
    /// except set-cookie, which yields one entry per value
    #[must_use]
    pub fn sorted_entries(&self) -> Vec<(String, String)> {
        let mut names: Vec<&str> = self.list.iter().map(|(n, _)| n.as_str()).collect();
        names.sort_unstable();
        names.dedup();

        let mut out = Vec::with_capacity(names.len());
        for name in names {
            if name == "set-cookie" {
                for value in self.get_set_cookie() {
                    out.push((name.to_owned(), value.to_owned()));
                }
            } else {
                let combined = self
                    .list
                    .iter()
                    .filter(|(n, _)| n == name)
                    .map(|(_, v)| v.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                out.push((name.to_owned(), combined));
            }
        }
        out
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.list.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_get_case_insensitive() {
        let mut h = Headers::new();
        h.append("Content-Type", "text/html").unwrap();
        assert_eq!(h.get("content-type").unwrap(), Some("text/html".to_owned()));
        assert_eq!(h.get("CONTENT-TYPE").unwrap(), Some("text/html".to_owned()));
        assert!(h.has("Content-type").unwrap());
        assert_eq!(h.get("absent").unwrap(), None);
    }

    #[test]
    fn values_are_normalized_and_combined() {
        let mut h = Headers::new();
        h.append("a", "  one\t").unwrap();
        h.append("A", "two").unwrap();
        assert_eq!(h.get("a").unwrap(), Some("one, two".to_owned()));

        // Empty value stays; combining preserves it
        h.append("b", "").unwrap();
        assert_eq!(h.get("b").unwrap(), Some(String::new()));
    }

    #[test]
    fn invalid_names_and_values_throw() {
        let mut h = Headers::new();
        for bad in ["", "invalid name", "und:erscore", "(comment)", "é"] {
            let err = h.append(bad, "v").unwrap_err();
            assert!(err.to_string().starts_with("TypeError:"), "{bad}");
        }
        for bad in ["a\0b", "line\nfeed", "carriage\rreturn"] {
            assert!(h.append("name", bad).is_err(), "{bad}");
        }
        // Leading/trailing CR/LF are stripped by normalization, so legal
        assert!(h.append("name", "\nok\r\n").is_ok());
        assert_eq!(h.get("name").unwrap(), Some("ok".to_owned()));
    }

    #[test]
    fn set_replaces_first_and_drops_rest() {
        let mut h = Headers::new();
        h.append("a", "1").unwrap();
        h.append("b", "2").unwrap();
        h.append("A", "3").unwrap();
        h.set("a", "9").unwrap();
        assert_eq!(
            h.sorted_entries(),
            vec![("a".into(), "9".into()), ("b".into(), "2".into())]
        );
    }

    #[test]
    fn delete_removes_all_matches() {
        let mut h = Headers::new();
        h.append("a", "1").unwrap();
        h.append("A", "2").unwrap();
        h.delete("a").unwrap();
        assert!(h.is_empty());
    }

    #[test]
    fn iteration_is_sorted_and_combined_with_set_cookie_exception() {
        let mut h = Headers::new();
        h.append("Set-Cookie", "a=1").unwrap();
        h.append("Accept", "text/html").unwrap();
        h.append("set-cookie", "b=2").unwrap();
        h.append("accept", "text/plain").unwrap();
        assert_eq!(
            h.sorted_entries(),
            vec![
                ("accept".into(), "text/html, text/plain".into()),
                ("set-cookie".into(), "a=1".into()),
                ("set-cookie".into(), "b=2".into()),
            ]
        );
        assert_eq!(h.get_set_cookie(), vec!["a=1", "b=2"]);
    }

    #[test]
    fn immutable_guard_blocks_mutation_after_validation() {
        let mut h = Headers::with_guard(HeadersGuard::Immutable);
        assert_eq!(h.append("a", "1").unwrap_err(), HeadersError::Immutable);
        // Validation errors take precedence over the guard
        assert!(matches!(
            h.append("bad name", "1").unwrap_err(),
            HeadersError::InvalidName(_)
        ));
    }
}
