use crate::node::elements::{
    FORMATTING_HTML_ELEMENTS, SPECIAL_HTML_ELEMENTS, SPECIAL_MATHML_ELEMENTS, SPECIAL_SVG_ELEMENTS,
};
use crate::node::{HTML_NAMESPACE, MATHML_NAMESPACE, SVG_NAMESPACE};
use core::fmt::{Debug, Formatter};
use gosub_shared::attributes::Attributes;
use gosub_shared::node::NodeId;
use indexmap::IndexSet;
use std::fmt;

/// The classes of an element: the `class` attribute parsed as an ordered set of tokens.
///
/// The attribute is the source of truth. `ElementData` rebuilds this list on every write to
/// `class`, and its class methods write the attribute back, so the two cannot drift apart.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ClassListImpl {
    classes: IndexSet<String>,
}

impl gosub_shared::memory::HeapSize for ClassListImpl {
    fn heap_size(&self, walk: &mut gosub_shared::memory::Walk) {
        // Each entry also keeps its hash, and the table an index for it.
        walk.bytes(self.classes.capacity() * (size_of::<String>() + 2 * size_of::<usize>()));
        for class in &self.classes {
            class.heap_size(walk);
        }
    }
}

impl ClassListImpl {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.classes.iter().map(String::as_str)
    }

    pub fn len(&self) -> usize {
        self.classes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.classes.is_empty()
    }

    pub fn contains(&self, name: &str) -> bool {
        self.classes.contains(name)
    }

    pub fn as_vec(&self) -> Vec<String> {
        self.classes.iter().cloned().collect()
    }

    /// The ordered set serializer: the tokens joined by a single space.
    fn serialize(&self) -> String {
        self.iter().collect::<Vec<_>>().join(" ")
    }
}

impl From<&str> for ClassListImpl {
    /// The ordered set parser: split on ASCII whitespace, drop duplicates, keep first order.
    fn from(class_string: &str) -> Self {
        let classes = class_string.split_ascii_whitespace().map(str::to_owned).collect();
        ClassListImpl { classes }
    }
}

/// A token `classList` accepts. The DOM throws on an empty token or one with whitespace in
/// it; here such a token is ignored.
fn is_valid_token(token: &str) -> bool {
    !token.is_empty() && !token.bytes().any(|b| b.is_ascii_whitespace())
}

/// Data structure for element nodes
#[derive(PartialEq, Clone)]
pub struct ElementData {
    pub name: String,
    pub namespace: Option<String>,
    /// Written only through `add_attribute` and `remove_attribute`, which keep `class_list`
    /// in step.
    attributes: Attributes,
    /// Derived from the `class` attribute; see `ClassListImpl`.
    class_list: ClassListImpl,
    /// Template contents: NodeId of the fragment root in the same arena (template elements only)
    pub template_contents: Option<NodeId>,
    /// Shadow root: NodeId of the shadow tree root in the same arena, for a shadow host.
    /// A side pointer like `template_contents`, so the shadow tree stays out of `children`.
    pub shadow_root: Option<NodeId>,
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
    pub(crate) fn new(name: &str, namespace: Option<&str>, attributes: Attributes) -> Self {
        let class_list = attributes
            .get("class")
            .map(|value| ClassListImpl::from(value.as_str()))
            .unwrap_or_default();
        Self {
            name: name.into(),
            namespace: Some(namespace.unwrap_or(HTML_NAMESPACE).into()),
            attributes,
            class_list,
            template_contents: None,
            shadow_root: None,
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

    pub fn attribute(&self, name: &str) -> Option<&String> {
        self.attributes.get(name)
    }

    pub fn attributes(&self) -> &Attributes {
        &self.attributes
    }

    pub fn add_attribute(&mut self, name: &str, value: &str) {
        self.attributes.insert(name.into(), value.into());
        if name == "class" {
            self.class_list = ClassListImpl::from(value);
        }
    }

    pub fn remove_attribute(&mut self, name: &str) {
        if name == "class" {
            self.class_list = ClassListImpl::new();
        }
        self.attributes.remove(name);
    }

    /// `classList.add()`.
    pub fn add_class(&mut self, class_name: &str) {
        if !is_valid_token(class_name) {
            return;
        }
        if !self.class_list.contains(class_name) {
            self.class_list.classes.insert(class_name.to_owned());
        }
        self.update_class_attribute();
    }

    /// `classList.remove()`.
    pub fn remove_class(&mut self, class_name: &str) {
        if !is_valid_token(class_name) {
            return;
        }
        self.class_list.classes.shift_remove(class_name);
        self.update_class_attribute();
    }

    /// `classList.toggle()`: adds a missing class and removes a present one, unless `force`
    /// says which way to go. Returns whether the class is present afterwards.
    pub fn toggle_class(&mut self, class_name: &str, force: Option<bool>) -> bool {
        if !is_valid_token(class_name) {
            return false;
        }
        if self.class_list.contains(class_name) {
            if force == Some(true) {
                return true;
            }
            self.remove_class(class_name);
            return false;
        }
        if force == Some(false) {
            return false;
        }
        self.add_class(class_name);
        true
    }

    /// `classList.replace()`. Returns false, and changes nothing, when `old` is not present.
    pub fn replace_class(&mut self, old: &str, new: &str) -> bool {
        if !is_valid_token(old) || !is_valid_token(new) {
            return false;
        }
        let classes = &mut self.class_list.classes;
        let Some(old_index) = classes.get_index_of(old) else {
            return false;
        };
        // Whichever of `old` and `new` comes first becomes `new`; the other one goes.
        match classes.get_index_of(new) {
            Some(new_index) if new_index == old_index => {}
            Some(new_index) if new_index < old_index => {
                classes.shift_remove_index(old_index);
            }
            _ => {
                classes.shift_remove(new);
                classes.shift_remove_index(old_index);
                classes.shift_insert(old_index, new.to_owned());
            }
        }
        self.update_class_attribute();
        true
    }

    /// The DOMTokenList update steps: write the set back to the `class` attribute. Nothing
    /// is written when there is no attribute and the set is empty.
    fn update_class_attribute(&mut self) {
        if !self.attributes.contains_key("class") && self.class_list.is_empty() {
            return;
        }
        self.attributes.insert("class".into(), self.class_list.serialize());
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

    /// The shadow root attached to this element, if it is a shadow host.
    pub fn shadow_root(&self) -> Option<NodeId> {
        self.shadow_root
    }

    pub fn set_shadow_root(&mut self, shadow_root: NodeId) {
        self.shadow_root = Some(shadow_root);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn element(class: Option<&str>) -> ElementData {
        let mut attributes = Attributes::new();
        if let Some(class) = class {
            attributes.insert("class".into(), class.into());
        }
        ElementData::new("div", None, attributes)
    }

    fn classes(e: &ElementData) -> Vec<String> {
        e.classlist().as_vec()
    }

    fn class_attr(e: &ElementData) -> Option<&str> {
        e.attribute("class").map(String::as_str)
    }

    #[test]
    fn parse_keeps_order_and_drops_duplicates() {
        let list = ClassListImpl::from("  b a\tb\n c\x0ca ");
        assert_eq!(list.as_vec(), ["b", "a", "c"]);
        assert_eq!(list.len(), 3);
        assert!(!ClassListImpl::from(" \t ").contains(""));
        assert!(ClassListImpl::from(" \t ").is_empty());
    }

    #[test]
    fn parse_splits_on_ascii_whitespace_only() {
        // U+00A0 and U+3000 are not ASCII whitespace, so they are part of the token.
        let list = ClassListImpl::from("a\u{a0}b c\u{3000}d");
        assert_eq!(list.as_vec(), ["a\u{a0}b", "c\u{3000}d"]);
        assert!(!list.contains("a"));
    }

    #[test]
    fn new_derives_classes_from_the_attribute() {
        assert_eq!(classes(&element(Some("x y"))), ["x", "y"]);
        assert!(element(None).classlist().is_empty());
    }

    #[test]
    fn set_class_attribute_replaces_classes() {
        let mut e = element(Some("x y"));
        e.add_attribute("class", "z");
        assert_eq!(classes(&e), ["z"]);
        e.add_attribute("class", "");
        assert!(e.classlist().is_empty());
    }

    #[test]
    fn remove_class_attribute_clears_classes() {
        let mut e = element(Some("x y"));
        e.remove_attribute("class");
        assert!(e.classlist().is_empty());
        assert_eq!(class_attr(&e), None);
    }

    #[test]
    fn remove_other_attribute_keeps_classes() {
        let mut e = element(Some("x"));
        e.add_attribute("id", "i");
        e.remove_attribute("id");
        assert_eq!(classes(&e), ["x"]);
    }

    #[test]
    fn add_class_writes_the_attribute() {
        let mut e = element(None);
        e.add_class("z");
        assert_eq!(class_attr(&e), Some("z"));
        e.add_class("w");
        assert_eq!(class_attr(&e), Some("z w"));
        assert_eq!(classes(&e), ["z", "w"]);
    }

    #[test]
    fn add_present_class_normalizes_the_attribute() {
        let mut e = element(Some("  a  b a "));
        e.add_class("a");
        assert_eq!(class_attr(&e), Some("a b"));
        assert_eq!(classes(&e), ["a", "b"]);
    }

    #[test]
    fn add_invalid_token_changes_nothing() {
        let mut e = element(Some(" a "));
        e.add_class("");
        e.add_class("b c");
        e.add_class("b\t");
        assert_eq!(class_attr(&e), Some(" a "));
        assert_eq!(classes(&e), ["a"]);
    }

    #[test]
    fn remove_class_writes_the_attribute() {
        let mut e = element(Some("x y x"));
        e.remove_class("x");
        assert_eq!(class_attr(&e), Some("y"));
        e.remove_class("y");
        assert_eq!(class_attr(&e), Some(""));
        assert!(e.classlist().is_empty());
    }

    #[test]
    fn remove_class_without_attribute_adds_none() {
        let mut e = element(None);
        e.remove_class("x");
        assert_eq!(class_attr(&e), None);
    }

    #[test]
    fn toggle_adds_a_missing_class() {
        let mut e = element(Some("x"));
        assert!(e.toggle_class("y", None));
        assert_eq!(class_attr(&e), Some("x y"));
        assert!(!e.toggle_class("y", None));
        assert_eq!(class_attr(&e), Some("x"));
    }

    #[test]
    fn toggle_with_force() {
        let mut e = element(None);
        assert!(!e.toggle_class("x", Some(false)));
        assert_eq!(class_attr(&e), None);
        assert!(e.toggle_class("x", Some(true)));
        assert!(e.toggle_class("x", Some(true)));
        assert_eq!(class_attr(&e), Some("x"));
        assert!(!e.toggle_class("x", Some(false)));
        assert_eq!(class_attr(&e), Some(""));
    }

    #[test]
    fn replace_class() {
        let mut e = element(Some("a b c"));
        assert!(e.replace_class("b", "z"));
        assert_eq!(class_attr(&e), Some("a z c"));
        assert!(!e.replace_class("missing", "q"));
        assert_eq!(class_attr(&e), Some("a z c"));
    }

    #[test]
    fn replace_class_with_one_already_present() {
        // The first of old and new takes the new name; the other copy goes.
        let mut e = element(Some("a b c"));
        assert!(e.replace_class("c", "a"));
        assert_eq!(class_attr(&e), Some("a b"));

        let mut e = element(Some("a b c"));
        assert!(e.replace_class("a", "c"));
        assert_eq!(class_attr(&e), Some("c b"));
    }

    #[test]
    fn replace_class_with_itself_only_normalizes() {
        let mut e = element(Some(" a  b c "));
        assert!(e.replace_class("b", "b"));
        assert_eq!(class_attr(&e), Some("a b c"));
    }

    #[test]
    fn clone_keeps_attribute_and_classes_together() {
        let mut e = element(Some("x"));
        let mut copy = e.clone();
        copy.add_class("y");
        e.remove_attribute("class");
        assert_eq!(classes(&copy), ["x", "y"]);
        assert_eq!(class_attr(&copy), Some("x y"));
        assert!(e.classlist().is_empty());
    }
}
