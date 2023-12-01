use std::collections::HashMap;

/// For now an AST does not contain different node kinds, but a single node that is distinguished
/// by the name. This will change in the near future.

#[derive(Debug, Clone)]
pub struct Node {
    pub name: String,
    pub attributes: HashMap<String, String>,
    pub children: Vec<Node>,
}

impl Node {
    pub(crate) fn with_attribute(&self, key: &str, val: String) -> Self {
        let mut node = self.clone();
        node.attributes.insert(key.to_string(), val);
        node
    }
}

impl Node {
    pub fn new(name: &str) -> Node {
        // trace!("Creating node: {}", name);
        Node {
            name: name.to_string(),
            attributes: HashMap::new(),
            children: Vec::new(),
        }
    }
}
