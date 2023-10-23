use crate::html5::element_class::ElementClass;
use crate::html5::node::NodeId;
use crate::html5::parser::document::{Document, DocumentFragment, DocumentHandle};
use core::fmt::{Debug, Formatter};
use std::collections::hash_map::Iter;
use std::collections::HashMap;
use std::fmt;

#[derive(Debug, PartialEq, Clone)]
/// Data structure for storing element attributes (ie: class="foo")
pub(crate) struct ElementAttributes {
    /// Numerical ID of the node these attributes are tied to
    pub(crate) node_id: NodeId,
    /// Pointer to the document that the node associated with these attributes are tied to
    pub(crate) document: DocumentHandle,
    /// Key-value pair of all attributes
    attributes: HashMap<String, String>,
}

/// This is a very thin wrapper around a HashMap.
/// Most of the methods are the same besides contains/insert.
/// This "controls" what you're allowed to do with an element's attributes
/// so there are no unexpected modifications.
impl ElementAttributes {
    pub(crate) fn new(node_id: NodeId, document: DocumentHandle) -> Self {
        Self {
            node_id,
            document,
            attributes: HashMap::new(),
        }
    }

    pub(crate) fn with_attributes(
        node_id: NodeId,
        document: DocumentHandle,
        attributes: HashMap<String, String>,
    ) -> Self {
        Self {
            node_id,
            document,
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

    /// Clones the internal map of attributes (NOT the attributes object itself)
    pub(crate) fn clone_map(&self) -> HashMap<String, String> {
        self.attributes.clone()
    }
}

#[derive(PartialEq, Clone)]
/// Data structure for element nodes
pub struct ElementData {
    /// Numerical ID of the node this data is attached to
    pub(crate) node_id: NodeId,
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
    /// Pointer to the document the node associated with this data is tied to
    pub(crate) document: DocumentHandle,
}

impl Debug for ElementData {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let mut debug = f.debug_struct("ElementData");
        debug.finish()
    }
}

impl ElementData {
    pub(crate) fn new(node_id: NodeId, document: DocumentHandle) -> Self {
        Self {
            node_id,
            name: "".to_string(),
            attributes: ElementAttributes::new(node_id, Document::clone(&document)),
            classes: ElementClass::new(),
            force_async: false,
            template_contents: None,
            document,
        }
    }

    pub(crate) fn with_name_and_attributes(
        node_id: NodeId,
        document: DocumentHandle,
        name: &str,
        attributes: HashMap<String, String>,
    ) -> Self {
        Self {
            node_id,
            name: name.into(),
            attributes: ElementAttributes::with_attributes(
                node_id,
                Document::clone(&document),
                attributes,
            ),
            classes: ElementClass::new(),
            force_async: false,
            template_contents: None,
            document,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn set_id(&mut self, node_id: NodeId) {
        self.node_id = node_id;
    }
}
