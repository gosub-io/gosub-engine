//! An element's attribute list.
//!
//! DOM keeps attributes as an ordered list: a new attribute is appended, a changed value stays
//! where it is, and removing one keeps the order of the rest. Serialization and enumeration
//! (`element.attributes`, `getAttributeNames()`) follow that order. Lookups are hashed: a tag
//! can carry any number of attributes, and the tokenizer looks each one up as it reads it.
//!
//! Equality ignores order: "the same attributes" in DOM (`isEqualNode`) and in the parser's
//! list of active formatting elements compares them as a set.

use crate::memory::{HeapSize, Walk};
use indexmap::IndexMap;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Attributes {
    entries: IndexMap<String, String>,
}

impl Attributes {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&String> {
        self.entries.get(name)
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut String> {
        self.entries.get_mut(name)
    }

    #[must_use]
    pub fn contains_key(&self, name: &str) -> bool {
        self.entries.contains_key(name)
    }

    /// Appends a new attribute, or replaces the value of an existing one in place. Returns the
    /// old value.
    pub fn insert(&mut self, name: String, value: String) -> Option<String> {
        self.entries.insert(name, value)
    }

    /// Removes an attribute, keeping the order of the rest. Returns its value.
    pub fn remove(&mut self, name: &str) -> Option<String> {
        self.entries.shift_remove(name)
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Attributes in list order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = (&String, &String)> {
        self.entries.iter()
    }

    pub fn keys(&self) -> impl ExactSizeIterator<Item = &String> {
        self.entries.keys()
    }

    pub fn values(&self) -> impl ExactSizeIterator<Item = &String> {
        self.entries.values()
    }

    pub fn values_mut(&mut self) -> impl ExactSizeIterator<Item = &mut String> {
        self.entries.values_mut()
    }

    /// Keeps the attributes for which `keep` returns true, in order.
    pub fn retain(&mut self, mut keep: impl FnMut(&String, &mut String) -> bool) {
        self.entries.retain(|n, v| keep(n, v));
    }
}

impl Extend<(String, String)> for Attributes {
    fn extend<I: IntoIterator<Item = (String, String)>>(&mut self, iter: I) {
        for (name, value) in iter {
            self.insert(name, value);
        }
    }
}

impl FromIterator<(String, String)> for Attributes {
    fn from_iter<I: IntoIterator<Item = (String, String)>>(iter: I) -> Self {
        let mut attributes = Self::new();
        attributes.extend(iter);
        attributes
    }
}

impl<const N: usize> From<[(String, String); N]> for Attributes {
    fn from(entries: [(String, String); N]) -> Self {
        entries.into_iter().collect()
    }
}

impl IntoIterator for Attributes {
    type Item = (String, String);
    type IntoIter = indexmap::map::IntoIter<String, String>;

    fn into_iter(self) -> Self::IntoIter {
        self.entries.into_iter()
    }
}

impl<'a> IntoIterator for &'a Attributes {
    type Item = (&'a String, &'a String);
    type IntoIter = indexmap::map::Iter<'a, String, String>;

    fn into_iter(self) -> Self::IntoIter {
        self.entries.iter()
    }
}

impl HeapSize for Attributes {
    fn heap_size(&self, walk: &mut Walk) {
        // Each entry also keeps its hash, and the table an index for it.
        walk.bytes(self.entries.capacity() * (size_of::<(String, String)>() + 2 * size_of::<usize>()));
        for (name, value) in &self.entries {
            name.heap_size(walk);
            value.heap_size(walk);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attrs(pairs: &[(&str, &str)]) -> Attributes {
        pairs.iter().map(|(n, v)| (n.to_string(), v.to_string())).collect()
    }

    fn names(a: &Attributes) -> Vec<&str> {
        a.keys().map(String::as_str).collect()
    }

    #[test]
    fn keeps_insertion_order() {
        let a = attrs(&[("id", "t"), ("width", "20"), ("height", "10")]);
        assert_eq!(names(&a), ["id", "width", "height"]);
    }

    #[test]
    fn a_changed_value_stays_in_place() {
        let mut a = attrs(&[("id", "t"), ("width", "20"), ("height", "10")]);
        assert_eq!(a.insert("width".into(), "30".into()), Some("20".into()));
        assert_eq!(names(&a), ["id", "width", "height"]);
        assert_eq!(a.get("width").map(String::as_str), Some("30"));
    }

    #[test]
    fn removing_keeps_the_order_of_the_rest() {
        let mut a = attrs(&[("id", "t"), ("width", "20"), ("height", "10")]);
        assert_eq!(a.remove("width"), Some("20".into()));
        assert_eq!(a.remove("width"), None);
        assert_eq!(names(&a), ["id", "height"]);
    }

    #[test]
    fn equality_ignores_order() {
        let a = attrs(&[("a", "1"), ("b", "2")]);
        assert_eq!(a, attrs(&[("b", "2"), ("a", "1")]));
        assert_ne!(a, attrs(&[("a", "1"), ("b", "3")]));
        assert_ne!(a, attrs(&[("a", "1")]));
    }
}
