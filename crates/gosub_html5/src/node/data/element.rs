use crate::element_class::ElementClass;
use crate::node::NodeId;
use crate::parser::document::DocumentFragment;
use core::fmt::{Debug, Formatter};

use std::collections::HashMap;
use std::fmt;

#[derive(PartialEq, Clone)]
/// Data structure for element nodes
pub struct ElementData {
    /// Numerical ID of the node this data is attached to
    pub node_id: NodeId,
    /// Name of the element (e.g., div)
    pub name: String,
    /// Element's attributes stored as key-value pairs.
    /// Note that it is NOT RECOMMENDED to modify this
    /// attribute map directly and instead use `TreeBuilder.insert_attribute`
    /// to keep attributes in sync with the DOM.
    pub attributes: HashMap<String, String>,
    /// CSS classes
    pub classes: ElementClass,
    // Only used for <script> elements
    pub force_async: bool,
    // Template contents (when it's a template element)
    pub template_contents: Option<DocumentFragment>,
}

impl Debug for ElementData {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let mut debug = f.debug_struct("ElementData");
        debug.field("node_id", &self.node_id);
        debug.field("name", &self.name);
        debug.field("attributes", &self.attributes);
        debug.field("classes", &self.classes);
        debug.finish()
    }
}

impl ElementData {
    #[allow(dead_code)]
    pub(crate) fn new(node_id: NodeId) -> Self {
        let (name, attributes, classes, force_async, template_contents) = <_>::default();
        Self {
            node_id,
            name,
            attributes,
            classes,
            force_async,
            template_contents,
        }
    }

    pub(crate) fn with_name_and_attributes(
        node_id: NodeId,
        name: &str,
        attributes: HashMap<String, String>,
    ) -> Self {
        let (classes, force_async, template_contents) = <_>::default();
        Self {
            node_id,
            name: name.into(),
            attributes,
            classes,
            force_async,
            template_contents,
        }
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn set_id(&mut self, node_id: NodeId) {
        self.node_id = node_id;
    }
}
