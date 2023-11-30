use crate::html5::element_class::ElementClass;
use crate::html5::node::NodeId;
use crate::html5::parser::document::DocumentFragment;
use core::fmt::{Debug, Formatter};

use std::collections::HashMap;
use std::fmt;

#[derive(PartialEq, Clone)]
/// Data structure for element nodes
pub struct ElementData {
    /// Numerical ID of the node this data is attached to
    pub(crate) node_id: NodeId,
    /// Name of the element (e.g., div)
    pub(crate) name: String,
    /// Element's attributes stored as key-value pairs.
    /// Note that it is NOT RECOMMENDED to modify this
    /// attribute map directly and instead use TreeBuilder.insert_attribute
    /// to keep attributes in sync with the DOM.
    pub(crate) attributes: HashMap<String, String>,
    /// CSS classes
    pub(crate) classes: ElementClass,
    // Only used for <script> elements
    pub(crate) force_async: bool,
    // Template contents (when it's a template element)
    pub(crate) template_contents: Option<DocumentFragment>,
}

impl Debug for ElementData {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let mut debug = f.debug_struct("ElementData");
        debug.finish()
    }
}

impl ElementData {
    pub(crate) fn new(node_id: NodeId) -> Self {
        Self {
            node_id,
            name: "".to_string(),
            attributes: HashMap::new(),
            classes: ElementClass::new(),
            force_async: false,
            template_contents: None,
        }
    }

    pub(crate) fn with_name_and_attributes(
        node_id: NodeId,
        name: &str,
        attributes: HashMap<String, String>,
    ) -> Self {
        Self {
            node_id,
            name: name.into(),
            attributes,
            classes: ElementClass::new(),
            force_async: false,
            template_contents: None,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn set_id(&mut self, node_id: NodeId) {
        self.node_id = node_id;
    }
}
