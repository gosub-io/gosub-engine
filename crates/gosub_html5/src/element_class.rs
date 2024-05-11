use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElementClass {
    /// a map of classes applied to an HTML element.
    /// key = name, value = `is_active`
    /// the `is_active` is used to toggle a class (JavaScript API)
    class_map: HashMap<String, bool>,
}

impl Default for ElementClass {
    fn default() -> Self {
        Self::new()
    }
}

impl ElementClass {
    /// Initialise a new (empty) `ElementClass`
    #[must_use]
    pub fn new() -> Self {
        Self {
            class_map: HashMap::new(),
        }
    }

    /// Count the number of classes (active or inactive)
    /// assigned to an element
    #[must_use]
    pub fn len(&self) -> usize {
        self.class_map.len()
    }

    /// Check if any classes are present
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.class_map.is_empty()
    }

    /// Check if class name exists
    #[must_use]
    pub fn contains(&self, name: &str) -> bool {
        self.class_map.contains_key(name)
    }

    /// Add a new class (if already exists, does nothing)
    pub fn add(&mut self, name: &str) {
        // by default, adding a new class will be active.
        // however, map.insert will update a key if it exists
        // and we don't want to overwrite an inactive class to make it active unintentionally
        // so we ignore this operation if the class already exists
        if !self.contains(name) {
            self.class_map.insert(name.to_owned(), true);
        }
    }

    /// Remove a class (does nothing if not exists)
    pub fn remove(&mut self, name: &str) {
        self.class_map.remove(name);
    }

    /// Toggle a class active/inactive. Does nothing if class doesn't exist
    pub fn toggle(&mut self, name: &str) {
        if let Some(is_active) = self.class_map.get_mut(name) {
            *is_active = !*is_active;
        }
    }

    /// Set explicitly if a class is active or not. Does nothing if class doesn't exist
    pub fn set_active(&mut self, name: &str, is_active: bool) {
        if let Some(is_active_item) = self.class_map.get_mut(name) {
            *is_active_item = is_active;
        }
    }

    /// Check if a class is active. Returns false if class doesn't exist
    #[must_use]
    pub fn is_active(&self, name: &str) -> bool {
        if let Some(is_active) = self.class_map.get(name) {
            return *is_active;
        }

        false
    }
}

/// Initialize a class from a class string
/// with space-delimited class names
impl From<&str> for ElementClass {
    fn from(class_string: &str) -> Self {
        let class_map_local = class_string
            .split_whitespace()
            .map(|class| (class.to_owned(), true))
            .collect::<HashMap<String, bool>>();

        Self {
            class_map: class_map_local,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_empty() {
        let mut classes = ElementClass::new();
        assert!(classes.is_empty());
        classes.add("one");
        assert!(!classes.is_empty());
    }

    #[test]
    fn count_classes() {
        let mut classes = ElementClass::new();
        classes.add("one");
        classes.add("two");
        assert_eq!(classes.len(), 2);
    }

    #[test]
    fn contains_nonexistant_class() {
        let classes = ElementClass::new();
        assert!(!classes.contains("nope"));
    }

    #[test]
    fn contains_valid_class() {
        let mut classes = ElementClass::new();
        classes.add("yep");
        assert!(classes.contains("yep"));
    }

    #[test]
    fn add_class() {
        let mut classes = ElementClass::new();
        classes.add("yep");
        assert!(classes.is_active("yep"));

        classes.set_active("yep", false);
        classes.add("yep"); // should be ignored
        assert!(!classes.is_active("yep"));
    }

    #[test]
    fn remove_class() {
        let mut classes = ElementClass::new();
        classes.add("yep");
        classes.remove("yep");
        assert!(!classes.contains("yep"));
    }

    #[test]
    fn toggle_class() {
        let mut classes = ElementClass::new();
        classes.add("yep");
        assert!(classes.is_active("yep"));
        classes.toggle("yep");
        assert!(!classes.is_active("yep"));
        classes.toggle("yep");
        assert!(classes.is_active("yep"));
    }
}
