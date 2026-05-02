use core::fmt::Debug;
use gosub_interface::css3::CssSystem;
use gosub_interface::document::{Document, DocumentType};
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::fmt;
use std::fmt::{Display, Formatter};
use url::Url;

use crate::document::task_queue::is_valid_id_attribute_value;
use crate::node::arena::NodeArena;
use crate::node::data::comment::CommentData;
use crate::node::data::doctype::DocTypeData;
use crate::node::data::element::{ClassListImpl, ElementData};
use crate::node::node_impl::{NodeDataTypeInternal, NodeImpl};
use crate::node::visitor::Visitor;
use crate::node::HTML_NAMESPACE;
use gosub_interface::config::HasDocument;
use gosub_interface::node::{NodeType, QuirksMode};
use gosub_shared::byte_stream::Location;
use gosub_shared::node::NodeId;

/// Defines a document
#[derive(Debug)]
pub struct DocumentImpl<C: HasDocument> {
    pub url: Option<Url>,
    pub(crate) arena: NodeArena,
    named_id_elements: HashMap<String, NodeId>,
    pub doctype: DocumentType,
    pub quirks_mode: QuirksMode,
    pub stylesheets: Vec<<C::CssSystem as CssSystem>::Stylesheet>,
}

impl<C: HasDocument> PartialEq for DocumentImpl<C> {
    fn eq(&self, other: &Self) -> bool {
        self.url == other.url
            && self.arena == other.arena
            && self.named_id_elements == other.named_id_elements
            && self.doctype == other.doctype
            && self.quirks_mode == other.quirks_mode
            && self.stylesheets == other.stylesheets
    }
}

// ── new Document<C> trait impl ──────────────────────────────────────────────

impl<C: HasDocument<Document = Self>> Document<C> for DocumentImpl<C> {
    fn new(document_type: DocumentType, url: Option<Url>) -> Self {
        let mut doc = Self {
            url,
            arena: NodeArena::new(),
            named_id_elements: HashMap::new(),
            doctype: document_type,
            quirks_mode: QuirksMode::NoQuirks,
            stylesheets: Vec::new(),
        };
        let root = NodeImpl::new_document(Location::default(), QuirksMode::NoQuirks);
        doc.arena.register_node(root);
        doc
    }

    fn new_fragment(quirks_mode: QuirksMode) -> Self {
        let mut doc = Self {
            url: None,
            arena: NodeArena::new(),
            named_id_elements: HashMap::new(),
            doctype: DocumentType::HTML,
            quirks_mode,
            stylesheets: Vec::new(),
        };
        let html_node = NodeImpl::new_element(Location::default(), "html", Some(HTML_NAMESPACE), HashMap::new());
        doc.arena.register_node(html_node);
        doc
    }

    // ── node creation ──────────────────────────────────────────────────────

    fn create_element(
        &mut self,
        name: &str,
        namespace: Option<&str>,
        attributes: HashMap<String, String>,
        location: Location,
    ) -> NodeId {
        let class_list = match attributes.get("class") {
            Some(class_value) => ClassListImpl::from(class_value.as_str()),
            None => ClassListImpl::default(),
        };
        let node = NodeImpl::new(
            location,
            NodeDataTypeInternal::Element(ElementData::new(name, namespace, attributes, class_list)),
        );
        self.register_node(node)
    }

    fn create_text(&mut self, value: &str, location: Location) -> NodeId {
        self.register_node(NodeImpl::new_text(location, value))
    }

    fn create_comment(&mut self, value: &str, location: Location) -> NodeId {
        self.register_node(NodeImpl::new_comment(location, value))
    }

    fn create_doctype(
        &mut self,
        name: &str,
        public_id: Option<&str>,
        system_id: Option<&str>,
        location: Location,
    ) -> NodeId {
        self.register_node(NodeImpl::new_doctype(
            location,
            name,
            public_id.unwrap_or(""),
            system_id.unwrap_or(""),
        ))
    }

    fn clone_node(&mut self, id: NodeId) -> NodeId {
        let Some(node) = self.arena.node(id) else { return id };
        let cloned = NodeImpl::new_from_node(&node);
        self.register_node(cloned)
    }

    fn duplicate_node(&mut self, id: NodeId) -> NodeId {
        let Some(node) = self.arena.node_ref(id) else { return id };
        let dup = NodeImpl::new_from_node(node);
        self.register_node(dup)
    }

    // ── tree navigation ────────────────────────────────────────────────────

    fn root(&self) -> NodeId {
        NodeId::root()
    }

    fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.arena.node_ref(id)?.parent
    }

    fn children(&self, id: NodeId) -> &[NodeId] {
        self.arena.node_ref(id).map_or(&[], |n| n.children.as_slice())
    }

    fn next_sibling(&self, id: NodeId) -> Option<NodeId> {
        self.get_next_sibling(id)
    }

    fn attach(&mut self, node: NodeId, parent: NodeId, position: Option<usize>) {
        self.attach_node(node, parent, position);
    }

    fn detach(&mut self, node: NodeId) {
        self.detach_node(node);
    }

    fn remove(&mut self, node: NodeId) {
        self.delete_node_by_id(node);
    }

    fn relocate_node(&mut self, node: NodeId, parent: NodeId) {
        self.detach_node(node);
        self.attach_node(node, parent, None);
    }

    // ── node type ──────────────────────────────────────────────────────────

    fn node_type(&self, id: NodeId) -> NodeType {
        self.arena.node_ref(id).map_or(NodeType::DocumentNode, |n| n.type_of())
    }

    // ── element data ───────────────────────────────────────────────────────

    fn tag_name(&self, id: NodeId) -> Option<&str> {
        match self.arena.node_ref(id)?.data {
            NodeDataTypeInternal::Element(ref e) => Some(e.name.as_str()),
            _ => None,
        }
    }

    fn namespace(&self, id: NodeId) -> Option<&str> {
        match self.arena.node_ref(id)?.data {
            NodeDataTypeInternal::Element(ref e) => e.namespace.as_deref(),
            _ => None,
        }
    }

    fn attribute(&self, id: NodeId, name: &str) -> Option<&str> {
        match self.arena.node_ref(id)?.data {
            NodeDataTypeInternal::Element(ref e) => e.attributes.get(name).map(String::as_str),
            _ => None,
        }
    }

    fn attributes(&self, id: NodeId) -> Option<&HashMap<String, String>> {
        match self.arena.node_ref(id)?.data {
            NodeDataTypeInternal::Element(ref e) => Some(&e.attributes),
            _ => None,
        }
    }

    fn set_attribute(&mut self, id: NodeId, name: &str, value: &str) {
        let is_element = if let Some(node) = self.arena.node_ref_mut(id) {
            if let NodeDataTypeInternal::Element(ref mut e) = node.data {
                e.add_attribute(name, value);
                true
            } else {
                false
            }
        } else {
            false
        };

        if is_element && name == "id" && is_valid_id_attribute_value(value) {
            if let Entry::Vacant(e) = self.named_id_elements.entry(value.to_string()) {
                e.insert(id);
            }
        }
    }

    fn remove_attribute(&mut self, id: NodeId, name: &str) {
        let Some(node) = self.arena.node_ref_mut(id) else {
            return;
        };
        if let NodeDataTypeInternal::Element(ref mut e) = node.data {
            e.remove_attribute(name);
        }
    }

    fn add_class(&mut self, id: NodeId, class: &str) {
        let Some(node) = self.arena.node_ref_mut(id) else {
            return;
        };
        if let NodeDataTypeInternal::Element(ref mut e) = node.data {
            e.add_class(class);
        }
    }

    fn has_class(&self, id: NodeId, name: &str) -> bool {
        let Some(node) = self.arena.node_ref(id) else {
            return false;
        };
        match &node.data {
            NodeDataTypeInternal::Element(e) => e.classlist().is_active(name),
            _ => false,
        }
    }

    fn template_contents(&self, id: NodeId) -> Option<NodeId> {
        match self.arena.node_ref(id)?.data {
            NodeDataTypeInternal::Element(ref e) => e.template_contents,
            _ => None,
        }
    }

    fn set_template_contents(&mut self, id: NodeId, fragment: NodeId) {
        let Some(node) = self.arena.node_ref_mut(id) else {
            return;
        };
        if let NodeDataTypeInternal::Element(ref mut e) = node.data {
            e.set_template_contents(fragment);
        }
    }

    // ── text / comment / doctype ───────────────────────────────────────────

    fn text_value(&self, id: NodeId) -> Option<&str> {
        match self.arena.node_ref(id)?.data {
            NodeDataTypeInternal::Text(ref t) => Some(t.value.as_str()),
            _ => None,
        }
    }

    fn set_text_value(&mut self, id: NodeId, value: &str) {
        let Some(node) = self.arena.node_ref_mut(id) else {
            return;
        };
        if let NodeDataTypeInternal::Text(ref mut t) = node.data {
            t.value = value.to_owned();
        }
    }

    fn comment_value(&self, id: NodeId) -> Option<&str> {
        match self.arena.node_ref(id)?.data {
            NodeDataTypeInternal::Comment(ref c) => Some(c.value.as_str()),
            _ => None,
        }
    }

    fn doctype_name(&self, id: NodeId) -> Option<&str> {
        match self.arena.node_ref(id)?.data {
            NodeDataTypeInternal::DocType(ref d) => Some(d.name.as_str()),
            _ => None,
        }
    }

    fn doctype_public_id(&self, id: NodeId) -> Option<&str> {
        match self.arena.node_ref(id)?.data {
            NodeDataTypeInternal::DocType(ref d) => Some(d.pub_identifier.as_str()),
            _ => None,
        }
    }

    fn doctype_system_id(&self, id: NodeId) -> Option<&str> {
        match self.arena.node_ref(id)?.data {
            NodeDataTypeInternal::DocType(ref d) => Some(d.sys_identifier.as_str()),
            _ => None,
        }
    }

    // ── document metadata ──────────────────────────────────────────────────

    fn url(&self) -> Option<Url> {
        self.url.clone()
    }

    fn quirks_mode(&self) -> QuirksMode {
        self.quirks_mode
    }

    fn set_quirks_mode(&mut self, mode: QuirksMode) {
        self.quirks_mode = mode;
    }

    fn doctype(&self) -> DocumentType {
        self.doctype
    }

    fn set_doctype(&mut self, doctype: DocumentType) {
        self.doctype = doctype;
    }

    fn node_by_named_id(&self, name_id: &str) -> Option<NodeId> {
        self.named_id_elements.get(name_id).copied()
    }

    fn node_count(&self) -> usize {
        self.arena.node_count()
    }

    fn peek_next_id(&self) -> NodeId {
        self.arena.peek_next_id()
    }

    // ── stylesheets ────────────────────────────────────────────────────────

    fn stylesheets(&self) -> &[<C::CssSystem as CssSystem>::Stylesheet] {
        &self.stylesheets
    }

    fn add_stylesheet(&mut self, sheet: <C::CssSystem as CssSystem>::Stylesheet) {
        self.stylesheets.push(sheet);
    }

    // ── serialisation ──────────────────────────────────────────────────────

    fn write(&self) -> String {
        self.write_from_node(NodeId::root())
    }

    fn write_from_node(&self, node_id: NodeId) -> String {
        crate::writer::DocumentWriter::write_from_node::<C>(node_id, self)
    }
}

// ── Internal helpers (not part of Document trait) ───────────────────────────

impl<C: HasDocument<Document = Self>> DocumentImpl<C> {
    fn on_document_node_mutation(&mut self, node: &NodeImpl) {
        self.on_document_node_mutation_update_named_id(node);
    }

    fn on_document_node_mutation_update_named_id(&mut self, node: &NodeImpl) {
        if !node.is_element_node() {
            return;
        }
        let element_data = node.get_element_data().unwrap();
        if let Some(id_value) = element_data.attributes.get("id") {
            if is_valid_id_attribute_value(id_value) {
                if let Entry::Vacant(e) = self.named_id_elements.entry(id_value.clone()) {
                    e.insert(node.id());
                }
            }
        } else {
            self.named_id_elements.retain(|_, id| *id != node.id());
        }
    }

    /// Fetch a node reference by id (internal — not in the Document trait)
    pub fn node_by_id(&self, node_id: NodeId) -> Option<&NodeImpl> {
        self.arena.node_ref(node_id)
    }

    pub fn node_by_id_mut(&mut self, node_id: NodeId) -> Option<&mut NodeImpl> {
        self.arena.node_ref_mut(node_id)
    }

    /// Look up a node by its `id` attribute value, returning the node reference directly
    pub fn get_node_by_named_id(&self, name_id: &str) -> Option<&NodeImpl> {
        let id = self.named_id_elements.get(name_id).copied()?;
        self.node_by_id(id)
    }

    /// Returns the root node reference
    pub fn get_root(&self) -> &NodeImpl {
        self.arena.node_ref(NodeId::root()).expect("Root node not found")
    }

    /// Register a node (assigns id, marks registered). Does NOT attach to tree.
    pub fn register_node(&mut self, mut node: NodeImpl) -> NodeId {
        let node_id = self.arena.get_next_id();
        node.set_id(node_id);
        if node.is_element_node() {
            let element_data = node.get_element_data_mut().unwrap();
            element_data.node_id = Some(node_id);
        }
        self.on_document_node_mutation(&node);
        self.arena.register_node_with_node_id(node, node_id);
        node_id
    }

    /// Register a node and attach it to a parent
    pub fn register_node_at(&mut self, node: NodeImpl, parent_id: NodeId, position: Option<usize>) -> NodeId {
        self.on_document_node_mutation(&node);
        let node_id = self.register_node(node);
        self.attach_node(node_id, parent_id, position);
        node_id
    }

    pub fn attach_node(&mut self, node_id: NodeId, parent_id: NodeId, position: Option<usize>) {
        if parent_id == node_id || self.has_node_id_recursive(node_id, parent_id) {
            return;
        }
        if let Some(parent_node) = self.arena.node(parent_id) {
            let mut parent_node = parent_node;
            match position {
                Some(position) => {
                    if position > parent_node.children().len() {
                        parent_node.push(node_id);
                    } else {
                        parent_node.insert(node_id, position);
                    }
                }
                None => {
                    parent_node.push(node_id);
                }
            }
            self.update_node(parent_node);
        }
        let mut node = self.arena.node(node_id).unwrap();
        node.parent = Some(parent_id);
        self.update_node(node);
    }

    pub fn detach_node(&mut self, node_id: NodeId) {
        let parent = self.node_by_id(node_id).expect("node not found").parent_id();
        if let Some(parent_id) = parent {
            let mut parent_node = self.node_by_id(parent_id).expect("parent node not found").clone();
            parent_node.remove(node_id);
            self.update_node(parent_node);

            let mut node = self.node_by_id(node_id).expect("node not found").clone();
            node.set_parent(None);
            self.update_node(node);
        }
    }

    pub fn relocate_node(&mut self, node_id: NodeId, parent_id: NodeId) {
        let node = self.arena.node_ref(node_id).unwrap();
        assert!(node.registered, "Node is not registered to the arena");
        if node.parent.is_some() && node.parent.unwrap() == parent_id {
            return;
        }
        self.detach_node(node_id);
        self.attach_node(node_id, parent_id, None);
    }

    pub fn update_node(&mut self, node: NodeImpl) {
        if !node.is_registered() {
            log::warn!("Node is not registered to the arena");
            return;
        }
        self.on_document_node_mutation(&node);
        self.arena.update_node(node);
    }

    pub fn update_node_ref(&mut self, node: &NodeImpl) {
        if !node.is_registered() {
            log::warn!("Node is not registered to the arena");
            return;
        }
        self.on_document_node_mutation(node);
        self.arena.update_node(node.clone());
    }

    pub fn delete_node_by_id(&mut self, node_id: NodeId) {
        let node = self.arena.node(node_id).unwrap();
        let parent_id = node.parent_id();
        if let Some(parent_id) = parent_id {
            let mut parent = self.node_by_id(parent_id).unwrap().clone();
            parent.remove(node_id);
            self.update_node(parent);
        }
        self.arena.delete_node(node_id);
    }

    pub fn get_next_sibling(&self, reference_node: NodeId) -> Option<NodeId> {
        let node = self.node_by_id(reference_node)?;
        let parent = self.node_by_id(node.parent_id()?)?;
        let idx = parent
            .children()
            .iter()
            .position(|&child_id| child_id == reference_node)?;
        let next_idx = idx + 1;
        if parent.children().len() > next_idx {
            return Some(parent.children()[next_idx]);
        }
        None
    }

    pub fn has_node_id_recursive(&self, parent_id: NodeId, target_node_id: NodeId) -> bool {
        let Some(parent) = self.arena.node_ref(parent_id) else {
            return false;
        };
        for &child_node_id in parent.children() {
            if child_node_id == target_node_id {
                return true;
            }
            if self.has_node_id_recursive(child_node_id, target_node_id) {
                return true;
            }
        }
        false
    }

    pub fn nodes(&self) -> &HashMap<NodeId, NodeImpl> {
        self.arena.nodes()
    }

    // ── legacy node constructors (used by parser) ──────────────────────────

    pub fn new_document_node(quirks_mode: QuirksMode, location: Location) -> NodeImpl {
        NodeImpl::new_document(location, quirks_mode)
    }

    pub fn new_doctype_node(
        name: &str,
        public_id: Option<&str>,
        system_id: Option<&str>,
        location: Location,
    ) -> NodeImpl {
        NodeImpl::new_doctype(location, name, public_id.unwrap_or(""), system_id.unwrap_or(""))
    }

    pub fn new_comment_node(comment: &str, location: Location) -> NodeImpl {
        NodeImpl::new_comment(location, comment)
    }

    pub fn new_text_node(value: &str, location: Location) -> NodeImpl {
        NodeImpl::new_text(location, value)
    }

    pub fn new_element_node(
        name: &str,
        namespace: Option<&str>,
        attributes: HashMap<String, String>,
        location: Location,
    ) -> NodeImpl {
        let class_list = match attributes.get("class") {
            Some(class_value) => ClassListImpl::from(class_value.as_str()),
            None => ClassListImpl::default(),
        };
        NodeImpl::new(
            location,
            NodeDataTypeInternal::Element(ElementData::new(name, namespace, attributes, class_list)),
        )
    }

    /// Creates a fragment document with an html element as root (at NodeId::root()).
    /// Used by DocumentBuilderImpl::new_document_fragment. parse_fragment expects
    /// NodeId::root() to be the html element, not a Document node.
    pub fn new_fragment(quirks_mode: QuirksMode) -> Self {
        let mut doc = Self {
            url: None,
            arena: NodeArena::new(),
            named_id_elements: HashMap::new(),
            doctype: DocumentType::HTML,
            quirks_mode,
            stylesheets: Vec::new(),
        };
        let html_node = NodeImpl::new_element(Location::default(), "html", Some(HTML_NAMESPACE), HashMap::new());
        doc.register_node(html_node);
        doc
    }

    // ── display helper ─────────────────────────────────────────────────────

    pub fn print_tree(&self, node: &NodeImpl, prefix: String, last: bool, f: &mut Formatter) {
        let mut buffer = prefix.clone();
        if last {
            buffer.push_str("└─ ");
        } else {
            buffer.push_str("├─ ");
        }

        match &node.data {
            NodeDataTypeInternal::Document(_) => {
                let _ = writeln!(f, "{buffer}Document");
            }
            NodeDataTypeInternal::DocType(DocTypeData {
                name,
                pub_identifier,
                sys_identifier,
            }) => {
                let _ = writeln!(f, r#"{buffer}<!DOCTYPE {name} "{pub_identifier}" "{sys_identifier}">"#);
            }
            NodeDataTypeInternal::Text(data) => {
                let _ = writeln!(f, r#"{buffer}"{}""#, data.value);
            }
            NodeDataTypeInternal::Comment(CommentData { value, .. }) => {
                let _ = writeln!(f, "{buffer}<!-- {value} -->");
            }
            NodeDataTypeInternal::Element(element) => {
                let _ = write!(f, "{}<{}", buffer, element.name);
                for (key, value) in &element.attributes {
                    let _ = write!(f, " {key}={value}");
                }
                let _ = writeln!(f, ">");
            }
        }

        if prefix.len() > 40 {
            let _ = writeln!(f, "...");
            return;
        }

        let mut buffer = prefix;
        if last {
            buffer.push_str("   ");
        } else {
            buffer.push_str("│  ");
        }

        let len = node.children.len();
        for (i, child_id) in node.children.iter().enumerate() {
            let child_node = self.node_by_id(*child_id).expect("Child not found");
            self.print_tree(child_node, buffer.clone(), i == len - 1, f);
        }
    }
}

impl<C: HasDocument<Document = Self>> Display for DocumentImpl<C> {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        let root = self.get_root();
        self.print_tree(root, String::new(), true, f);
        Ok(())
    }
}

// ── Tree iterator ────────────────────────────────────────────────────────────

pub struct TreeIterator<'a, C: HasDocument> {
    current_node_id: Option<NodeId>,
    node_stack: Vec<NodeId>,
    document: &'a C::Document,
}

impl<'a, C: HasDocument> TreeIterator<'a, C> {
    #[must_use]
    pub fn new(doc: &'a C::Document) -> Self {
        let node_stack = vec![doc.root()];
        Self {
            current_node_id: None,
            document: doc,
            node_stack,
        }
    }
}

impl<C: HasDocument> Iterator for TreeIterator<'_, C> {
    type Item = NodeId;

    fn next(&mut self) -> Option<NodeId> {
        if let Some(node_id) = self.node_stack.pop() {
            // Push children in reverse order so first child is processed first
            for child_id in self.document.children(node_id).iter().rev() {
                self.node_stack.push(*child_id);
            }

            self.current_node_id = Some(node_id);
            return self.current_node_id;
        }
        None
    }
}

// ── Document tree walk ───────────────────────────────────────────────────────

pub fn walk_document_tree<C: HasDocument<Document = DocumentImpl<C>>>(
    doc: &DocumentImpl<C>,
    visitor: &mut dyn Visitor,
) {
    let root = doc.get_root();
    internal_visit(doc, root, visitor);
}

fn internal_visit<C: HasDocument<Document = DocumentImpl<C>>>(
    doc: &DocumentImpl<C>,
    node: &NodeImpl,
    visitor: &mut dyn Visitor,
) {
    visitor.document_enter(node);
    for &child_id in node.children() {
        let child = doc.node_by_id(child_id).unwrap();
        internal_visit(doc, child, visitor);
    }
    visitor.document_leave(node);
}
