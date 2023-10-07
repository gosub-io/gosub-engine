use std::collections::HashMap;

#[derive(Debug, PartialEq, Clone)]
struct ElementAttributes {
    attributes: HashMap<String, String>,
}

impl Default for ElementAttributes {
    fn default() -> Self {
        Self::new()
    }
}

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
}
