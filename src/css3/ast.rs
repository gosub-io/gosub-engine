use core::fmt::{Debug, Formatter};
use std::collections::HashMap;
use serde_derive::Serialize;
use serde_json::json;

/// For now an AST does not contain different node kinds, but a single node that is distinguished
/// by the name. This will change in the near future.

#[derive(Clone, Serialize)]
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

impl Debug for Node {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let obj = json!(self);
        write!(f, "{}", serde_json::to_string_pretty(&obj).unwrap())
    }
}
