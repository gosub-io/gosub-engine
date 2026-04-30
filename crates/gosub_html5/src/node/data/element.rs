use crate::node::elements::{
    FORMATTING_HTML_ELEMENTS, SPECIAL_HTML_ELEMENTS, SPECIAL_MATHML_ELEMENTS, SPECIAL_SVG_ELEMENTS,
};
use crate::node::{HTML_NAMESPACE, MATHML_NAMESPACE, SVG_NAMESPACE};
use core::fmt::{Debug, Formatter};
use gosub_shared::node::NodeId;
use std::collections::hash_map::IntoIter;
use std::collections::HashMap;
use std::fmt;

#[derive(Debug)]
pub struct ClassListImpl {
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
    #[must_use]
    pub fn new() -> Self {
        Self {
            class_map: HashMap::new(),
        }
    }

    pub fn iter(&self) -> IntoIter<String, bool> {
        self.class_map.clone().into_iter()
    }

    pub fn len(&self) -> usize {
        self.class_map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.class_map.is_empty()
    }

    pub fn contains(&self, name: &str) -> bool {
        self.class_map.contains_key(name)
    }

    pub fn add(&mut self, name: &str) {
        if !self.contains(name) {
            self.class_map.insert(name.to_owned(), true);
        }
    }

    pub fn remove(&mut self, name: &str) {
        self.class_map.remove(name);
    }

    pub fn toggle(&mut self, name: &str) {
        if let Some(is_active) = self.class_map.get_mut(name) {
            *is_active = !*is_active;
        }
    }

    pub fn set_active(&mut self, name: &str, is_active: bool) {
        if let Some(is_active_item) = self.class_map.get_mut(name) {
            *is_active_item = is_active;
        }
    }

    pub fn is_active(&self, name: &str) -> bool {
        self.class_map.get(name).copied().unwrap_or(false)
    }

    pub fn replace(&mut self, old_class_name: &str, new_class_name: &str) {
        if let Some(is_active) = self.class_map.remove(old_class_name) {
            self.class_map.insert(new_class_name.to_owned(), is_active);
        }
    }

    pub fn length(&self) -> usize {
        self.class_map.len()
    }

    pub fn as_vec(&self) -> Vec<String> {
        self.class_map.keys().cloned().collect()
    }

    pub fn active_classes(&self) -> Vec<String> {
        self.class_map
            .iter()
            .filter(|(_, &active)| active)
            .map(|(name, _)| name.clone())
            .collect()
    }
}

impl From<&str> for ClassListImpl {
    fn from(class_string: &str) -> Self {
        let class_map = class_string
            .split_whitespace()
            .map(|class| (class.to_owned(), true))
            .collect();
        ClassListImpl { class_map }
    }
}

/// Data structure for element nodes
#[derive(PartialEq, Clone)]
pub struct ElementData {
    pub node_id: Option<NodeId>,
    pub name: String,
    pub namespace: Option<String>,
    pub attributes: HashMap<String, String>,
    pub class_list: ClassListImpl,
    pub force_async: bool,
    /// Template contents: NodeId of the fragment root in the same arena (template elements only)
    pub template_contents: Option<NodeId>,
}

impl Debug for ElementData {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let mut debug = f.debug_struct("ElementData");
        debug.field("name", &self.name);
        debug.field("attributes", &self.attributes);
        debug.field("classes", &self.class_list);
        debug.finish()
    }
}

impl ElementData {
    pub(crate) fn new(
        name: &str,
        namespace: Option<&str>,
        attributes: HashMap<String, String>,
        classlist: ClassListImpl,
    ) -> Self {
        Self {
            node_id: None,
            name: name.into(),
            namespace: Some(namespace.unwrap_or(HTML_NAMESPACE).into()),
            attributes,
            class_list: classlist,
            force_async: false,
            template_contents: None,
        }
    }

    pub fn name(&self) -> &str {
        self.name.as_str()
    }

    pub fn namespace(&self) -> &str {
        match self.namespace {
            Some(ref namespace) => namespace.as_str(),
            None => HTML_NAMESPACE,
        }
    }

    pub fn is_namespace(&self, namespace: &str) -> bool {
        if self.namespace.is_none() {
            return namespace == HTML_NAMESPACE;
        }
        self.namespace == Some(namespace.into())
    }

    pub fn classlist(&self) -> &ClassListImpl {
        &self.class_list
    }

    pub fn classlist_mut(&mut self) -> &mut ClassListImpl {
        &mut self.class_list
    }

    pub fn active_class_names(&self) -> Vec<String> {
        self.class_list
            .iter()
            .filter(|(_, active)| *active)
            .map(|(name, _)| name.clone())
            .collect()
    }

    pub fn attribute(&self, name: &str) -> Option<&String> {
        self.attributes.get(name)
    }

    pub fn attributes(&self) -> &HashMap<String, String> {
        &self.attributes
    }

    pub fn add_attribute(&mut self, name: &str, value: &str) {
        self.attributes.insert(name.into(), value.into());
        if name == "class" {
            self.class_list = ClassListImpl::new();
            for class in value.split_whitespace() {
                self.class_list.add(class);
            }
        }
    }

    pub fn remove_attribute(&mut self, name: &str) {
        if name == "class" {
            if let Some(value) = self.attributes.get(name) {
                self.class_list.remove(value);
            }
        }
        self.attributes.remove(name);
    }

    pub fn add_class(&mut self, class_name: &str) {
        self.class_list.add(class_name);
    }

    pub fn matches_tag_and_attrs_without_order(&self, other_data: &ElementData) -> bool {
        if self.name != other_data.name || self.namespace != other_data.namespace {
            return false;
        }
        self.attributes.eq(&other_data.attributes)
    }

    pub fn is_mathml_integration_point(&self) -> bool {
        let namespace = self.namespace.clone().unwrap_or_default();
        namespace == MATHML_NAMESPACE && ["mi", "mo", "mn", "ms", "mtext"].contains(&self.name.as_str())
    }

    pub fn is_html_integration_point(&self) -> bool {
        match self.namespace {
            Some(ref namespace) => {
                if namespace == MATHML_NAMESPACE && self.name == "annotation-xml" {
                    if let Some(value) = self.attributes.get("encoding") {
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

    pub fn is_special(&self) -> bool {
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

    pub fn template_contents(&self) -> Option<NodeId> {
        self.template_contents
    }

    pub fn is_formatting(&self) -> bool {
        self.namespace == Some(HTML_NAMESPACE.into()) && FORMATTING_HTML_ELEMENTS.contains(&self.name.as_str())
    }

    pub fn set_template_contents(&mut self, template_contents: NodeId) {
        self.template_contents = Some(template_contents);
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
