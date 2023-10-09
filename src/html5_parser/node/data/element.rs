use crate::html5_parser::element_class::ElementClass;
use crate::html5_parser::parser::document::DocumentFragment;
use std::collections::hash_map::Iter;
use std::collections::HashMap;

#[derive(Debug, PartialEq, Clone)]
/// Data structure for storing element attributes (ie: class="foo")
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

    /// Returns true when the attribute map contains the given name.
    pub(crate) fn contains(&self, name: &str) -> bool {
        self.attributes.contains_key(name)
    }

    /// Inserts a new attribute into the map.
    pub(crate) fn insert(&mut self, name: &str, value: &str) {
        self.attributes.insert(name.to_owned(), value.to_owned());
    }

    /// Removes an attribute from the map.
    pub(crate) fn remove(&mut self, name: &str) {
        self.attributes.remove(name);
    }

    /// Returns the value of the attribute with the given name.
    pub(crate) fn get(&self, name: &str) -> Option<&String> {
        self.attributes.get(name)
    }

    /// Returns a mutable reference to the value of the attribute with the given name.
    pub(crate) fn get_mut(&mut self, name: &str) -> Option<&mut String> {
        self.attributes.get_mut(name)
    }

    /// Clears the attribute map.
    pub(crate) fn clear(&mut self) {
        self.attributes.clear();
    }

    /// Returns true if the attribute map is empty.
    pub(crate) fn is_empty(&self) -> bool {
        self.attributes.is_empty()
    }

    /// Returns an iterator over the attribute map.
    pub(crate) fn iter(&self) -> Iter<'_, String, String> {
        self.attributes.iter()
    }

    /// Adds the given attributes to the attribute map.
    pub(crate) fn copy_from(&mut self, attribute_map: HashMap<String, String>) {
        for (key, value) in attribute_map.iter() {
            self.insert(key, value);
        }
    }
}

#[derive(Debug, PartialEq, Clone)]
/// Data structure for element nodes
pub struct ElementData {
    /// Name of the element (e.g., div)
    pub(crate) name: String,
    /// Element's attributes stored as key-value pairs
    pub(crate) attributes: ElementAttributes,
    /// CSS classes
    pub(crate) classes: ElementClass,
    // Only used for <script> elements
    pub(crate) force_async: bool,
    // Template contents (when it's a template element)
    pub(crate) template_contents: Option<DocumentFragment>,
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
            force_async: false,
            template_contents: None,
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
            force_async: false,
            template_contents: None,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}
