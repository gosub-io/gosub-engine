use crate::node::elements::{
    FORMATTING_HTML_ELEMENTS, SPECIAL_HTML_ELEMENTS, SPECIAL_MATHML_ELEMENTS, SPECIAL_SVG_ELEMENTS,
};
use crate::node::{HTML_NAMESPACE, MATHML_NAMESPACE, SVG_NAMESPACE};
use core::fmt::{Debug, Formatter};
use gosub_shared::document::DocumentHandle;
use gosub_shared::node::NodeId;
use gosub_shared::traits::config::HasDocument;
use gosub_shared::traits::node::{ClassList, ElementDataType};
use std::collections::hash_map::IntoIter;
use std::collections::HashMap;
use std::fmt;

#[derive(Debug)]
pub struct ClassListImpl {
    /// a map of classes applied to an HTML element.
    /// key = name, value = is_active
    /// the is_active is used to toggle a class (JavaScript API)
    class_map: HashMap<String, bool>,
}

impl Clone for ClassListImpl {
    fn clone(&self) -> Self {
        Self {
            class_map: self.class_map.clone(),
        }
    }
}

impl PartialEq for ClassListImpl {
    fn eq(&self, other: &Self) -> bool {
        self.class_map == other.class_map
    }
}

impl Default for ClassListImpl {
    fn default() -> Self {
        Self::new()
    }
}

impl ClassListImpl {
    /// Initialise a new (empty) ClassList
    #[must_use]
    pub fn new() -> Self {
        Self {
            class_map: HashMap::new(),
        }
    }
}

impl ClassList for ClassListImpl {
    fn iter(&self) -> IntoIter<String, bool> {
        self.class_map.clone().into_iter()
    }

    /// Count the number of classes (active or inactive)
    /// assigned to an element
    fn len(&self) -> usize {
        self.class_map.len()
    }

    /// Check if any classes are present
    fn is_empty(&self) -> bool {
        self.class_map.is_empty()
    }

    /// Check if class name exists
    fn contains(&self, name: &str) -> bool {
        self.class_map.contains_key(name)
    }

    /// Add a new class (if already exists, does nothing)
    fn add(&mut self, name: &str) {
        // by default, adding a new class will be active.
        // however, map.insert will update a key if it exists,
        // and we don't want to overwrite an inactive class to make it active unintentionally,
        // so we ignore this operation if the class already exists
        if !self.contains(name) {
            self.class_map.insert(name.to_owned(), true);
        }
    }

    /// Remove a class (does nothing if not exists)
    fn remove(&mut self, name: &str) {
        self.class_map.remove(name);
    }

    /// Toggle a class active/inactive. Does nothing if class doesn't exist
    fn toggle(&mut self, name: &str) {
        if let Some(is_active) = self.class_map.get_mut(name) {
            *is_active = !*is_active;
        }
    }

    /// Set explicitly if a class is active or not. Does nothing if class doesn't exist
    fn set_active(&mut self, name: &str, is_active: bool) {
        if let Some(is_active_item) = self.class_map.get_mut(name) {
            *is_active_item = is_active;
        }
    }

    /// Check if a class is active. Returns false if class doesn't exist
    fn is_active(&self, name: &str) -> bool {
        if let Some(is_active) = self.class_map.get(name) {
            return *is_active;
        }

        false
    }

    fn replace(&mut self, old_class_name: &str, new_class_name: &str) {
        if let Some(is_active) = self.class_map.remove(old_class_name) {
            self.class_map.insert(new_class_name.to_owned(), is_active);
        }
    }

    fn length(&self) -> usize {
        self.class_map.len()
    }

    fn as_vec(&self) -> Vec<String> {
        self.class_map.keys().cloned().collect()
    }

    fn active_classes(&self) -> Vec<String> {
        self.class_map
            .iter()
            .filter(|(_, &active)| active)
            .map(|(name, _)| name.clone())
            .collect()
    }
}

/// Initialize a class from a class string
/// with space-delimited class names
impl From<&str> for ClassListImpl {
    fn from(class_string: &str) -> Self {
        let class_map_local = class_string
            .split_whitespace()
            .map(|class| (class.to_owned(), true))
            .collect::<HashMap<String, bool>>();

        ClassListImpl {
            class_map: class_map_local,
        }
    }
}

/// Data structure for element nodes
#[derive(PartialEq, Clone)]
pub struct ElementData<C: HasDocument> {
    pub doc_handle: DocumentHandle<C>,
    pub node_id: Option<NodeId>,
    /// Name of the element (e.g., div)
    pub name: String,
    /// Namespace of the element
    pub namespace: Option<String>,
    /// Element's attributes stored as key-value pairs.
    /// Note that it is NOT RECOMMENDED to modify this
    /// attribute map directly and instead use TreeBuilder.insert_attribute
    /// to keep attributes in sync with the DOM.
    pub attributes: HashMap<String, String>,
    /// CSS list of classes
    pub class_list: ClassListImpl,
    // Only used for <script> elements
    pub force_async: bool,
    // Template contents (when it's a template element)
    pub template_contents: Option<C::DocumentFragment>,
}

impl<C: HasDocument> Debug for ElementData<C> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let mut debug = f.debug_struct("ElementData");
        debug.field("name", &self.name);
        debug.field("attributes", &self.attributes);
        debug.field("classes", &self.class_list);
        debug.finish()
    }
}

impl<C: HasDocument> ElementDataType<C> for ElementData<C> {
    fn name(&self) -> &str {
        self.name.as_str()
    }

    fn namespace(&self) -> &str {
        match self.namespace {
            Some(ref namespace) => namespace.as_str(),
            None => HTML_NAMESPACE,
        }
    }

    fn is_namespace(&self, namespace: &str) -> bool {
        if self.namespace.is_none() {
            return namespace == HTML_NAMESPACE;
        }

        self.namespace == Some(namespace.into())
    }

    fn classlist(&self) -> &impl ClassList {
        &self.class_list
    }

    fn classlist_mut(&mut self) -> &mut impl ClassList {
        &mut self.class_list
    }

    fn active_class_names(&self) -> Vec<String> {
        self.class_list
            .iter()
            .filter(|(_, active)| *active)
            .map(|(name, _)| name.clone())
            .collect()
    }

    fn attribute(&self, name: &str) -> Option<&String> {
        self.attributes.get(name)
    }

    fn attributes(&self) -> &HashMap<String, String> {
        &self.attributes
    }

    /// Note that adding attributes should not be done directly, but rather through the document.insert_attribute
    /// function. This way, the document can keep track of ID attributes.
    fn add_attribute(&mut self, name: &str, value: &str) {
        self.attributes.insert(name.into(), value.into());

        // Classes are treated as a special attributes as they are stored separately
        if name == "class" {
            self.class_list = ClassListImpl::new();
            for class in value.split_whitespace() {
                self.class_list.add(class);
            }
        }
    }

    // removes attribute from element
    fn remove_attribute(&mut self, name: &str) {
        if name == "class" {
            if let Some(value) = self.attributes.get(name) {
                self.class_list.remove(value);
            }
        }

        self.attributes.remove(name);
    }

    fn add_class(&mut self, class_name: &str) {
        self.class_list.add(class_name);
    }

    /// This will only compare against the tag, namespace and data same except element data.
    /// for element data compare against the tag, namespace and attributes without order.
    /// Both nodes could still have other parents and children.
    fn matches_tag_and_attrs_without_order(&self, other_data: &ElementData<C>) -> bool {
        if self.name != other_data.name || self.namespace != other_data.namespace {
            return false;
        }

        if self.name != other_data.name {
            return false;
        }

        if self.namespace != other_data.namespace {
            return false;
        }

        self.attributes.eq(&other_data.attributes)
    }

    /// Returns true if the given node is a mathml integration point
    /// See: https://html.spec.whatwg.org/multipage/parsing.html#mathml-text-integration-point
    fn is_mathml_integration_point(&self) -> bool {
        let namespace = self.namespace.clone().unwrap_or_default();

        namespace == MATHML_NAMESPACE && ["mi", "mo", "mn", "ms", "mtext"].contains(&self.name.as_str())
    }

    /// Returns true if the given node is a html integration point
    /// See: https://html.spec.whatwg.org/multipage/parsing.html#html-integration-point
    fn is_html_integration_point(&self) -> bool {
        match self.namespace {
            Some(ref namespace) => {
                if namespace == MATHML_NAMESPACE && self.name == "annotation-xml" {
                    if let Some(value) = self.attributes().get("encoding") {
                        if value.eq_ignore_ascii_case("text/html") {
                            return true;
                        }
                        if value.eq_ignore_ascii_case("application/xhtml+xml") {
                            return true;
                        }
                    }

                    return false;
                }

                namespace == SVG_NAMESPACE && ["foreignObject", "desc", "title"].contains(&self.name.as_str())
            }
            None => false,
        }
    }

    /// Returns true if the given node is "special" node based on the namespace and name
    fn is_special(&self) -> bool {
        if self.namespace == Some(HTML_NAMESPACE.into()) && SPECIAL_HTML_ELEMENTS.contains(&self.name()) {
            return true;
        }
        if self.namespace == Some(MATHML_NAMESPACE.into()) && SPECIAL_MATHML_ELEMENTS.contains(&self.name()) {
            return true;
        }
        if self.namespace == Some(SVG_NAMESPACE.into()) && SPECIAL_SVG_ELEMENTS.contains(&self.name()) {
            return true;
        }

        false
    }

    fn template_contents(&self) -> Option<&C::DocumentFragment> {
        match &self.template_contents {
            Some(fragment) => Some(fragment),
            None => None,
        }
    }

    /// Returns true if the given node is a "formatting" node
    fn is_formatting(&self) -> bool {
        self.namespace == Some(HTML_NAMESPACE.into()) && FORMATTING_HTML_ELEMENTS.contains(&self.name.as_str())
    }

    fn set_template_contents(&mut self, template_contents: C::DocumentFragment) {
        self.template_contents = Some(template_contents);
    }
}

impl<C: HasDocument> ElementData<C> {
    pub(crate) fn new(
        doc_handle: DocumentHandle<C>,
        name: &str,
        namespace: Option<&str>,
        attributes: HashMap<String, String>,
        classlist: ClassListImpl,
    ) -> Self {
        let (force_async, template_contents) = <_>::default();
        Self {
            doc_handle: doc_handle.clone(),
            node_id: None, // We are not yet registered in the document, so we have no node-id
            name: name.into(),
            namespace: Some(namespace.unwrap_or(HTML_NAMESPACE).into()),
            attributes,
            class_list: classlist,
            force_async,
            template_contents,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_empty() {
        let mut classes = ClassListImpl::new();
        assert!(classes.is_empty());
        classes.add("one");
        assert!(!classes.is_empty());
    }

    #[test]
    fn count_classes() {
        let mut classes = ClassListImpl::new();
        classes.add("one");
        classes.add("two");
        assert_eq!(classes.len(), 2);
    }

    #[test]
    fn contains_nonexistent_class() {
        let classes = ClassListImpl::new();
        assert!(!classes.contains("nope"));
    }

    #[test]
    fn contains_valid_class() {
        let mut classes = ClassListImpl::new();
        classes.add("yep");
        assert!(classes.contains("yep"));
    }

    #[test]
    fn add_class() {
        let mut classlist = ClassListImpl::new();
        classlist.add("yep");
        assert!(classlist.is_active("yep"));

        classlist.set_active("yep", false);
        classlist.add("yep"); // should be ignored
        assert!(!classlist.is_active("yep"));
    }

    #[test]
    fn remove_class() {
        let mut classlist = ClassListImpl::new();
        classlist.add("yep");
        classlist.remove("yep");
        assert!(!classlist.contains("yep"));
    }

    #[test]
    fn toggle_class() {
        let mut classlist = ClassListImpl::new();
        classlist.add("yep");
        assert!(classlist.is_active("yep"));
        classlist.toggle("yep");
        assert!(!classlist.is_active("yep"));
        classlist.toggle("yep");
        assert!(classlist.is_active("yep"));
    }
}
