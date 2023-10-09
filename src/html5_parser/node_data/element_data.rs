use crate::html5_parser::element_class::ElementClass;
use std::collections::hash_map::Iter;
use std::collections::HashMap;

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct ElementAttributes {
    pub(crate) attributes: HashMap<String, String>,
}

impl Default for ElementAttributes {
    fn default() -> Self {
        Self::new()
    }
}

/// This is a very thin wrapper around a HashMap.
/// Most of the methods are the same besides contains/insert.
/// This "controls" what you're allowed to do with an element's attributes
/// so there are no unexpected modifications.
impl ElementAttributes {
    pub(crate) fn new() -> Self {
        Self {
            attributes: HashMap::new(),
        }
    }

    pub(crate) fn with_attributes(attributes: HashMap<String, String>) -> Self {
        Self {
            attributes: attributes.clone(),
        }
    }

    pub(crate) fn contains(&self, name: &str) -> bool {
        self.attributes.contains_key(name)
    }

    pub(crate) fn insert(&mut self, name: &str, value: &str) {
        self.attributes.insert(name.to_owned(), value.to_owned());
    }

    pub(crate) fn remove(&mut self, name: &str) {
        self.attributes.remove(name);
    }

    pub(crate) fn get(&self, name: &str) -> Option<&String> {
        self.attributes.get(name)
    }

    pub(crate) fn get_mut(&mut self, name: &str) -> Option<&mut String> {
        self.attributes.get_mut(name)
    }

    pub(crate) fn clear(&mut self) {
        self.attributes.clear();
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.attributes.is_empty()
    }

    pub(crate) fn iter(&self) -> Iter<'_, String, String> {
        self.attributes.iter()
    }

    pub(crate) fn copy_from(&mut self, attribute_map: HashMap<String, String>) {
        for (key, value) in attribute_map.iter() {
            self.insert(key, value);
        }
    }
}

#[derive(Debug, PartialEq, Clone)]
pub struct ElementData {
    /// Name of the element (e.g., div)
    pub(crate) name: String,
    /// Element's attributes stored as key-value pairs
    pub(crate) attributes: ElementAttributes,
    /// CSS classes (only relevant for NodeType::Element, otherwise None)
    pub(crate) classes: ElementClass,
}

impl Default for ElementData {
    fn default() -> Self {
        Self::new()
    }
}

impl ElementData {
    pub(crate) fn new() -> Self {
        Self {
            name: "".to_string(),
            attributes: ElementAttributes::new(),
            classes: ElementClass::new(),
        }
    }

    pub(crate) fn with_name_and_attributes(
        name: &str,
        attributes: HashMap<String, String>,
    ) -> Self {
        Self {
            name: name.into(),
            attributes: ElementAttributes::with_attributes(attributes),
            classes: ElementClass::new(),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}
