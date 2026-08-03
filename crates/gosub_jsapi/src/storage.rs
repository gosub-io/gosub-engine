//! The Web Storage `Storage` interface per
//! <https://html.spec.whatwg.org/multipage/webstorage.html>: an ordered
//! key/value map with a byte quota, backing `localStorage`/`sessionStorage`.
//!
//! Keys keep their insertion position when overwritten (the spec leaves order
//! implementation-defined but requires it to stay stable between mutations,
//! which `key(n)` exposes).

use std::error::Error;
use std::fmt;

/// The quota browsers conventionally give each origin: 5 MiB, measured in
/// UTF-16 code units over all keys and values.
pub const DEFAULT_QUOTA: usize = 5 * 1024 * 1024;

/// `Display` carries the error name prefix — the same rethrow protocol as the
/// other jsapi modules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageError {
    QuotaExceeded,
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::QuotaExceeded => f.write_str("QuotaExceededError: the storage quota has been exceeded"),
        }
    }
}

impl Error for StorageError {}

/// UTF-16 code units — the unit the quota is measured in, since that is what
/// JS strings are made of.
fn utf16_units(s: &str) -> usize {
    s.encode_utf16().count()
}

#[derive(Debug, Clone)]
pub struct Storage {
    entries: Vec<(String, String)>,
    quota: usize,
    used: usize,
}

impl Default for Storage {
    fn default() -> Self {
        Self::new()
    }
}

impl Storage {
    #[must_use]
    pub fn new() -> Self {
        Self::with_quota(DEFAULT_QUOTA)
    }

    #[must_use]
    pub fn with_quota(quota: usize) -> Self {
        Self {
            entries: Vec::new(),
            quota,
            used: 0,
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The name of the nth key, or `None` when out of range
    #[must_use]
    pub fn key(&self, index: usize) -> Option<&str> {
        self.entries.get(index).map(|(k, _)| k.as_str())
    }

    #[must_use]
    pub fn get_item(&self, key: &str) -> Option<&str> {
        self.entries.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    /// Insert or overwrite. An overwritten key keeps its position; the quota
    /// is checked against the total after the change, so shrinking a value
    /// always succeeds.
    pub fn set_item(&mut self, key: &str, value: &str) -> Result<(), StorageError> {
        let added = utf16_units(key) + utf16_units(value);

        if let Some(pos) = self.entries.iter().position(|(k, _)| k == key) {
            let removed = utf16_units(key) + utf16_units(&self.entries[pos].1);
            let used = self.used - removed + added;
            if used > self.quota {
                return Err(StorageError::QuotaExceeded);
            }
            self.used = used;
            value.clone_into(&mut self.entries[pos].1);
        } else {
            let used = self.used + added;
            if used > self.quota {
                return Err(StorageError::QuotaExceeded);
            }
            self.used = used;
            self.entries.push((key.to_owned(), value.to_owned()));
        }
        Ok(())
    }

    pub fn remove_item(&mut self, key: &str) {
        if let Some(pos) = self.entries.iter().position(|(k, _)| k == key) {
            let (k, v) = self.entries.remove(pos);
            self.used -= utf16_units(&k) + utf16_units(&v);
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.used = 0;
    }

    #[must_use]
    pub fn keys(&self) -> Vec<&str> {
        self.entries.iter().map(|(k, _)| k.as_str()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_get_remove_roundtrip() {
        let mut s = Storage::new();
        assert_eq!(s.get_item("a"), None);
        s.set_item("a", "1").unwrap();
        assert_eq!(s.get_item("a"), Some("1"));
        assert_eq!(s.len(), 1);
        s.remove_item("a");
        s.remove_item("missing");
        assert_eq!(s.get_item("a"), None);
        assert!(s.is_empty());
    }

    #[test]
    fn empty_key_and_value_are_legal() {
        let mut s = Storage::new();
        s.set_item("", "empty key").unwrap();
        s.set_item("empty value", "").unwrap();
        assert_eq!(s.get_item(""), Some("empty key"));
        assert_eq!(s.get_item("empty value"), Some(""));
    }

    #[test]
    fn overwrite_keeps_position() {
        let mut s = Storage::new();
        s.set_item("a", "1").unwrap();
        s.set_item("b", "2").unwrap();
        s.set_item("a", "9").unwrap();
        assert_eq!(s.key(0), Some("a"));
        assert_eq!(s.key(1), Some("b"));
        assert_eq!(s.key(2), None);
        assert_eq!(s.get_item("a"), Some("9"));
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn keys_in_insertion_order() {
        let mut s = Storage::new();
        s.set_item("z", "1").unwrap();
        s.set_item("a", "2").unwrap();
        assert_eq!(s.keys(), vec!["z", "a"]);
    }

    #[test]
    fn quota_is_enforced_in_utf16_units() {
        // "ab" + "cd" = 4 units fits a quota of 4; one unit more does not
        let mut s = Storage::with_quota(4);
        s.set_item("ab", "cd").unwrap();
        assert_eq!(s.set_item("e", "").unwrap_err(), StorageError::QuotaExceeded);

        // Overwriting with something smaller succeeds and frees room
        s.set_item("ab", "c").unwrap();
        s.set_item("e", "").unwrap();

        // Non-BMP chars count as two units: one astral char = 2 units
        let mut s = Storage::with_quota(2);
        assert!(s.set_item("💥", "x").is_err());
        s.set_item("💥", "").unwrap();
    }

    #[test]
    fn failed_set_leaves_state_untouched() {
        let mut s = Storage::with_quota(4);
        s.set_item("ab", "cd").unwrap();
        assert!(s.set_item("ab", "cde").is_err());
        assert_eq!(s.get_item("ab"), Some("cd"));
        s.clear();
        assert_eq!(s.len(), 0);
        s.set_item("ab", "cd").unwrap();
    }
}
