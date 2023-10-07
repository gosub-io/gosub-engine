use std::collections::HashMap;
use std::collections::hash_map::Iter;

#[derive(Debug, PartialEq, Clone)]
pub struct ElementAttributes {
    attributes: HashMap<String, String>,
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
    pub fn new() -> Self {
        ElementAttributes {
            attributes: HashMap::new(),
        }
    }

    pub fn contains(&self, name: &str) -> bool {
        self.attributes.contains_key(name)
    }

    pub fn insert(&mut self, name: &str, value: &str) {
        self.attributes.insert(name.to_owned(), value.to_owned());
    }

    pub fn remove(&mut self, name: &str) {
        self.attributes.remove(name);
    }

    pub fn get(&self, name: &str) -> Option<&String> {
        self.attributes.get(name)
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut String> {
        self.attributes.get_mut(name)
    }

    pub fn clear(&mut self) {
        self.attributes.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.attributes.is_empty()
    }

    pub fn iter(&self) -> Iter<'_, String, String> {
        self.attributes.iter()
    }

    pub fn clone(&self) -> HashMap<String, String> {
        self.attributes.clone()
    }

    pub fn copy_from(&mut self, attribute_map: HashMap<String, String>) {
        for (key, value) in attribute_map.iter() {
            self.insert(key, value);
        }
    }
}

#[derive(Debug, PartialEq, Clone)]
pub struct ElementData {
    name: String,
    pub attributes: ElementAttributes,
}

impl Default for ElementData {
    fn default() -> Self {
        Self::new()
    }
}

impl ElementData {
    pub fn new() -> Self {
        ElementData {
            name: "".to_string(),
            attributes: ElementAttributes::new(),
        }
    }

    pub fn new_with_name_and_attributes(name: &str, attributes: HashMap<String, String>) -> Self {
        let mut element_data = ElementData::new();
        element_data.set_name(name);
        element_data.attributes.copy_from(attributes);

        element_data
    }

    pub fn get_name(&self) -> String {
        self.name.clone()
    }

    pub fn set_name(&mut self, new_name: &str) {
        self.name = new_name.to_owned();
    }
}
