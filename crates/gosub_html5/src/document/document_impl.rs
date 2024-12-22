use crate::DocumentHandle;
use core::fmt::Debug;
use gosub_interface::document::{Document as OtherDocument, Document, DocumentType};
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::fmt;
use std::fmt::{Display, Formatter};
use url::Url;

use crate::document::task_queue::is_valid_id_attribute_value;
use crate::node::arena::NodeArena;
use crate::node::data::comment::CommentData;
use crate::node::data::doctype::DocTypeData;
use crate::node::data::document::DocumentData;
use crate::node::data::element::{ClassListImpl, ElementData};
use crate::node::data::text::TextData;
use crate::node::node_impl::{NodeDataTypeInternal, NodeImpl};
use crate::node::visitor::Visitor;
use gosub_interface::config::HasDocument;
use gosub_interface::node::Node;
use gosub_interface::node::QuirksMode;
use gosub_shared::byte_stream::Location;
use gosub_shared::node::NodeId;

/// Defines a document
#[derive(Debug)]
pub struct DocumentImpl<C: HasDocument> {
    // pub handle: Weak<DocumentHandle<Self>>,
    /// URL of the given document (if any)
    pub url: Option<Url>,
    /// Holds and owns all nodes in the document
    pub(crate) arena: NodeArena<C>,
    /// HTML elements with ID (e.g., <div id="myid">)
    named_id_elements: HashMap<String, NodeId>,
    /// Document type of this document
    pub doctype: DocumentType,
    /// Quirks mode of this document
    pub quirks_mode: QuirksMode,
    /// Loaded stylesheets as extracted from the document
    pub stylesheets: Vec<C::Stylesheet>,
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

impl<C: HasDocument<Document = Self>> Document<C> for DocumentImpl<C> {
    type Node = NodeImpl<C>;

    /// Creates a new document without a doc handle
    #[must_use]
    fn new(document_type: DocumentType, url: Option<Url>, root_node: Option<Self::Node>) -> DocumentHandle<C> {
        let mut doc = Self {
            url,
            arena: NodeArena::new(),
            named_id_elements: HashMap::new(),
            doctype: document_type,
            quirks_mode: QuirksMode::NoQuirks,
            stylesheets: Vec::new(),
        };

        if let Some(node) = root_node {
            doc.register_node(node);

            DocumentHandle::create(doc)
        } else {
            let mut doc_handle = DocumentHandle::create(doc);
            let node = Self::Node::new_document(doc_handle.clone(), Location::default(), QuirksMode::NoQuirks);
            doc_handle.get_mut().arena.register_node(node);

            doc_handle
        }
    }

    /// Returns the URL of the document, or "" when no location is set
    fn url(&self) -> Option<Url> {
        self.url.clone()
    }

    fn set_quirks_mode(&mut self, quirks_mode: QuirksMode) {
        self.quirks_mode = quirks_mode;
    }

    fn quirks_mode(&self) -> QuirksMode {
        self.quirks_mode
    }

    fn set_doctype(&mut self, doctype: DocumentType) {
        self.doctype = doctype;
    }

    fn doctype(&self) -> DocumentType {
        self.doctype
    }

    /// Fetches a node by id or returns None when no node with this ID is found
    fn node_by_id(&self, node_id: NodeId) -> Option<&Self::Node> {
        self.arena.node_ref(node_id)
    }

    fn node_by_named_id(&self, id: &str) -> Option<&Self::Node> {
        self.named_id_elements
            .get(id)
            .and_then(|node_id| self.arena.node_ref(*node_id))
    }

    fn stylesheets(&self) -> &Vec<C::Stylesheet> {
        &self.stylesheets
    }

    // /// Add given node to the named ID elements
    // fn add_named_id(&mut self, id: &str, node_id: NodeId) {
    //     self.named_id_elements.insert(id.to_string(), node_id);
    // }
    //
    // /// Remove a named ID from the document
    // fn remove_named_id(&mut self, id: &str) {
    //     self.named_id_elements.remove(id);
    // }

    fn add_stylesheet(&mut self, stylesheet: C::Stylesheet) {
        self.stylesheets.push(stylesheet);
    }

    /// returns the root node
    fn get_root(&self) -> &Self::Node {
        self.arena.node_ref(NodeId::root()).expect("Root node not found !?")
    }

    fn attach_node(&mut self, node_id: NodeId, parent_id: NodeId, position: Option<usize>) {
        // Check if any children of node have parent as child. This will keep adding the node to itself
        if parent_id == node_id || self.has_node_id_recursive(node_id, parent_id) {
            return;
        }

        if let Some(parent_node) = self.node_by_id(parent_id) {
            let mut parent_node = parent_node.clone();

            // Make sure position can never be larger than the number of children in the parent
            match position {
                Some(position) => {
                    if position > parent_node.children().len() {
                        parent_node.push(node_id);
                    } else {
                        parent_node.insert(node_id, position);
                    }
                }
                None => {
                    // No position given, add to end of the children list
                    parent_node.push(node_id);
                }
            }

            self.update_node(parent_node);
        }

        let mut node = self.arena.node(node_id).unwrap();
        node.parent = Some(parent_id);
        self.update_node(node);
    }

    // /// returns the root node
    // fn get_root_mut(&mut self) -> &mut Self::Node {
    //     self.arena.node_mut(NodeId::root()).expect("Root node not found !?")
    // }

    fn detach_node(&mut self, node_id: NodeId) {
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

    /// Relocates a node to another parent node
    fn relocate_node(&mut self, node_id: NodeId, parent_id: NodeId) {
        let node = self.arena.node_ref(node_id).unwrap();
        assert!(node.registered, "Node is not registered to the arena");

        if node.parent.is_some() && node.parent.unwrap() == parent_id {
            // Nothing to do when we want to relocate to its own parent
            return;
        }

        self.detach_node(node_id);
        self.attach_node(node_id, parent_id, None);
    }

    fn update_node(&mut self, node: Self::Node) {
        if !node.is_registered() {
            log::warn!("Node is not registered to the arena");
            return;
        }

        self.on_document_node_mutation(&node);
        self.arena.update_node(node);
    }

    fn update_node_ref(&mut self, node: &Self::Node) {
        if !node.is_registered() {
            log::warn!("Node is not registered to the arena");
            return;
        }

        self.on_document_node_mutation(node);
        self.arena.update_node(node.clone());
    }

    /// Removes a node by id from the arena. Note that this does not check the nodelist itself to see
    /// if the node is still available as a child or parent in the tree.
    fn delete_node_by_id(&mut self, node_id: NodeId) {
        let node = self.arena.node(node_id).unwrap();
        let parent_id = node.parent_id();

        if let Some(parent_id) = parent_id {
            let mut parent = self.node_by_id(parent_id).unwrap().clone();
            parent.remove(node_id);
            self.update_node(parent);
        }

        self.arena.delete_node(node_id);
    }

    // /// Returns the parent node of the given node, or None when no parent is found
    // fn parent_node(&self, node: &Self::Node) -> Option<&Self::Node> {
    //     match node.parent_id() {
    //         Some(parent_node_id) => self.node_by_id(parent_node_id),
    //         None => None,
    //     }
    // }

    /// Retrieves the next sibling NodeId (to the right) of the reference_node or None.
    fn get_next_sibling(&self, reference_node: NodeId) -> Option<NodeId> {
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

    fn node_count(&self) -> usize {
        self.arena.node_count()
    }

    fn peek_next_id(&self) -> NodeId {
        self.arena.peek_next_id()
    }

    /// Register a node. It is not connected to anything yet, but it does receive a nodeId
    fn register_node(&mut self, mut node: Self::Node) -> NodeId {
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

    /// Inserts a node to the parent node at the given position in the children (or none
    /// to add at the end). Will automatically register the node if not done so already
    fn register_node_at(&mut self, node: Self::Node, parent_id: NodeId, position: Option<usize>) -> NodeId {
        self.on_document_node_mutation(&node);

        let node_id = self.register_node(node);
        self.attach_node(node_id, parent_id, position);

        node_id
    }

    /// Creates a new document node
    fn new_document_node(handle: DocumentHandle<C>, quirks_mode: QuirksMode, location: Location) -> Self::Node {
        NodeImpl::new(
            handle.clone(),
            location,
            &NodeDataTypeInternal::Document(DocumentData::new(quirks_mode)),
        )
    }

    fn new_doctype_node(
        handle: DocumentHandle<C>,
        name: &str,
        public_id: Option<&str>,
        system_id: Option<&str>,
        location: Location,
    ) -> Self::Node {
        NodeImpl::new(
            handle.clone(),
            location,
            &NodeDataTypeInternal::DocType(DocTypeData::new(name, public_id.unwrap_or(""), system_id.unwrap_or(""))),
        )
    }

    /// Creates a new comment node
    fn new_comment_node(handle: DocumentHandle<C>, comment: &str, location: Location) -> Self::Node {
        NodeImpl::new(
            handle.clone(),
            location,
            &NodeDataTypeInternal::Comment(CommentData::with_value(comment)),
        )
    }

    /// Creates a new text node
    fn new_text_node(handle: DocumentHandle<C>, value: &str, location: Location) -> Self::Node {
        NodeImpl::new(
            handle.clone(),
            location,
            &NodeDataTypeInternal::Text(TextData::with_value(value)),
        )
    }

    /// Creates a new element node
    fn new_element_node(
        handle: DocumentHandle<C>,
        name: &str,
        namespace: Option<&str>,
        attributes: HashMap<String, String>,
        location: Location,
    ) -> Self::Node {
        // Extract class list from the class-attribute (if exists)
        let class_list = match attributes.get("class") {
            Some(class_value) => ClassListImpl::from(class_value.as_str()),
            None => ClassListImpl::default(),
        };

        NodeImpl::new(
            handle.clone(),
            location,
            &NodeDataTypeInternal::Element(ElementData::new(
                handle.clone(),
                name,
                namespace,
                attributes,
                class_list,
            )),
        )
    }

    fn write(&self) -> String {
        self.write_from_node(NodeId::root())
    }

    fn write_from_node(&self, _node_id: NodeId) -> String {
        todo!(); //This should definitely be implemented
    }

    fn cloned_node_by_id(&self, node_id: NodeId) -> Option<Self::Node> {
        self.arena.node(node_id)
    }
}

impl<C: HasDocument<Document = Self>> DocumentImpl<C> {
    // Called whenever a node is being mutated in the document.
    fn on_document_node_mutation(&mut self, node: &NodeImpl<C>) {
        // self.on_document_node_mutation_update_id_in_node(node);
        self.on_document_node_mutation_update_named_id(node);
    }

    /// Update document's named id structure when the node has ID elements
    fn on_document_node_mutation_update_named_id(&mut self, node: &NodeImpl<C>) {
        if !node.is_element_node() {
            return;
        }

        let element_data = node.get_element_data().unwrap();
        if let Some(id_value) = element_data.attributes.get("id") {
            // When we have an ID attribute: update the named ID element map.
            if is_valid_id_attribute_value(id_value) {
                match self.named_id_elements.entry(id_value.clone()) {
                    Entry::Vacant(entry) => {
                        entry.insert(node.id());
                    }
                    Entry::Occupied(_) => {}
                }
            }
        } else {
            // If we don't have an ID attribute in the node, make sure that we remove and "old" id's that might be in the map.
            self.named_id_elements.retain(|_, id| *id != node.id());
        }
    }

    /// Print a node and all its children in a tree-like structure
    pub fn print_tree(&self, node: &C::Node, prefix: String, last: bool, f: &mut Formatter) {
        let mut buffer = prefix.clone();
        if last {
            buffer.push_str("└─ ");
        } else {
            buffer.push_str("├─ ");
        }
        // buffer.push_str(format!("{} ", node.id).as_str());

        match &node.data {
            NodeDataTypeInternal::Document(_) => {
                _ = writeln!(f, "{buffer}Document");
            }
            NodeDataTypeInternal::DocType(DocTypeData {
                name,
                pub_identifier,
                sys_identifier,
            }) => {
                _ = writeln!(f, r#"{buffer}<!DOCTYPE {name} "{pub_identifier}" "{sys_identifier}">"#,);
            }
            NodeDataTypeInternal::Text(TextData { value, .. }) => {
                _ = writeln!(f, r#"{buffer}"{value}""#);
            }
            NodeDataTypeInternal::Comment(CommentData { value, .. }) => {
                _ = writeln!(f, "{buffer}<!-- {value} -->");
            }
            NodeDataTypeInternal::Element(element) => {
                _ = write!(f, "{}<{}", buffer, element.name);
                for (key, value) in &element.attributes {
                    _ = write!(f, " {key}={value}");
                }

                // for (key, value) in node.style.borrow().iter() {
                //     _ = write!(f, "[CSS: {key}={value}]");
                // }

                _ = writeln!(f, ">");
            }
        }

        if prefix.len() > 40 {
            _ = writeln!(f, "...");
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
        self.print_tree(root, "".to_string(), true, f);
        Ok(())
    }
}

impl<C: HasDocument<Document = Self>> DocumentImpl<C> {
    /// Fetches a node by named id (string) or returns None when no node with this ID is found
    pub fn get_node_by_named_id(&self, named_id: &str) -> Option<&C::Node> {
        let node_id = self.named_id_elements.get(named_id)?;
        self.arena.node_ref(*node_id)
    }

    // /// Fetches a mutable node by named id (string) or returns None when no node with this ID is found
    // pub fn get_node_by_named_id_mut<D>(
    //     &mut self,
    //     named_id: &str,
    // ) -> Option<&mut C::Node> {
    //     let node_id = self.named_id_elements.get(named_id)?;
    //     self.arena.node_mut(*node_id)
    // }

    // pub fn count_nodes(&self) -> usize {
    //     self.arena.count_nodes()
    // }

    pub fn has_node_id_recursive(&self, parent_id: NodeId, target_node_id: NodeId) -> bool {
        let parent = self.arena.node_ref(parent_id);
        if parent.is_none() {
            return false;
        }

        for child_node_id in parent.unwrap().children() {
            if *child_node_id == target_node_id {
                return true;
            }
            if self.has_node_id_recursive(*child_node_id, target_node_id) {
                return true;
            }
        }

        false
    }

    pub fn peek_next_id(&self) -> NodeId {
        self.arena.peek_next_id()
    }

    pub fn nodes(&self) -> &HashMap<NodeId, C::Node> {
        self.arena.nodes()
    }
}

// Walk the document tree with the given visitor
pub fn walk_document_tree<C: HasDocument>(handle: DocumentHandle<C>, visitor: &mut Box<dyn Visitor<C>>) {
    let binding = handle.get();
    let root = binding.get_root();
    internal_visit(handle.clone(), root, visitor);
}

fn internal_visit<C: HasDocument>(handle: DocumentHandle<C>, node: &C::Node, visitor: &mut Box<dyn Visitor<C>>) {
    visitor.document_enter(node);

    let binding = handle.get();
    for child_id in node.children() {
        let child = binding.node_by_id(*child_id).unwrap();
        internal_visit(handle.clone(), child, visitor);
    }
    drop(binding);

    // Leave node
    visitor.document_leave(node);
}

/// Constructs an iterator from a given DocumentHandle.
/// WARNING: mutations in the document would be reflected
/// in the iterator. It's advised to consume the entire iterator
/// before mutating the document again.
pub struct TreeIterator<C: HasDocument> {
    current_node_id: Option<NodeId>,
    node_stack: Vec<NodeId>,
    document: DocumentHandle<C>,
}

impl<C: HasDocument> TreeIterator<C> {
    #[must_use]
    pub fn new(doc: DocumentHandle<C>) -> Self {
        let node_stack = vec![doc.get().get_root().id()];

        Self {
            current_node_id: None,
            document: doc,
            node_stack,
        }
    }
}

impl<C: HasDocument> Iterator for TreeIterator<C> {
    type Item = NodeId;

    fn next(&mut self) -> Option<NodeId> {
        self.current_node_id = self.node_stack.pop();

        if let Some(current_node_id) = self.current_node_id {
            let doc_read = self.document.get();

            if let Some(sibling_id) = self.document.get().get_next_sibling(current_node_id) {
                self.node_stack.push(sibling_id);
            }

            if let Some(current_node) = doc_read.node_by_id(current_node_id) {
                if let Some(&child_id) = current_node.children().first() {
                    self.node_stack.push(child_id);
                }
            }
        }

        self.current_node_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::builder::DocumentBuilderImpl;
    use crate::document::fragment::DocumentFragmentImpl;
    use crate::document::query::DocumentQuery;
    use crate::document::task_queue::DocumentTaskQueue;
    use crate::node::HTML_NAMESPACE;
    use crate::parser::query::Query;
    use crate::parser::tree_builder::TreeBuilder;
    use gosub_css3::system::Css3System;
    use gosub_interface::config::HasCssSystem;
    use gosub_interface::document::DocumentBuilder;
    use gosub_interface::node::ClassList;
    use gosub_interface::node::ElementDataType;
    use gosub_interface::node::NodeType;
    use gosub_shared::byte_stream::Location;
    use std::collections::HashMap;

    #[derive(Clone, Debug, PartialEq)]
    struct Config;

    impl HasCssSystem for Config {
        type CssSystem = Css3System;
    }
    impl HasDocument for Config {
        type Document = DocumentImpl<Self>;
        type DocumentFragment = DocumentFragmentImpl<Self>;
        type DocumentBuilder = DocumentBuilderImpl;
    }
    type Handle = DocumentHandle<Config>;
    type Document = DocumentImpl<Config>;

    #[test]
    fn relocate() {
        let mut doc_handle = DocumentBuilderImpl::new_document(None);

        let parent_node = Document::new_element_node(
            doc_handle.clone(),
            "parent",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let node1 = Document::new_element_node(
            doc_handle.clone(),
            "div1",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let node2 = Document::new_element_node(
            doc_handle.clone(),
            "div2",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let node3 = Document::new_element_node(
            doc_handle.clone(),
            "div3",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let node3_1 = Document::new_element_node(
            doc_handle.clone(),
            "div3_1",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );

        let parent_id = doc_handle.get_mut().register_node_at(parent_node, NodeId::root(), None);
        let node1_id = doc_handle.get_mut().register_node_at(node1, parent_id, None);
        let node2_id = doc_handle.get_mut().register_node_at(node2, parent_id, None);
        let node3_id = doc_handle.get_mut().register_node_at(node3, parent_id, None);
        let node3_1_id = doc_handle.get_mut().register_node_at(node3_1, node3_id, None);

        assert_eq!(
            format!("{}", doc_handle.get()),
            r#"└─ Document
   └─ <parent>
      ├─ <div1>
      ├─ <div2>
      └─ <div3>
         └─ <div3_1>
"#
        );

        doc_handle.get_mut().relocate_node(node3_1_id, node1_id);
        assert_eq!(
            format!("{}", doc_handle.get()),
            r#"└─ Document
   └─ <parent>
      ├─ <div1>
      │  └─ <div3_1>
      ├─ <div2>
      └─ <div3>
"#
        );

        doc_handle.get_mut().relocate_node(node1_id, node2_id);
        assert_eq!(
            format!("{}", doc_handle.get()),
            r#"└─ Document
   └─ <parent>
      ├─ <div2>
      │  └─ <div1>
      │     └─ <div3_1>
      └─ <div3>
"#
        );
    }

    #[test]
    fn verify_node_ids_in_element_data() {
        let mut doc_handle: DocumentHandle<Config> = DocumentBuilderImpl::new_document(None);

        let node_1: NodeImpl<Config> = DocumentImpl::new_element_node(
            doc_handle.clone(),
            "div",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let node_2: NodeImpl<Config> = DocumentImpl::new_element_node(
            doc_handle.clone(),
            "div",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );

        doc_handle.get_mut().register_node_at(node_1, NodeId::root(), None);
        doc_handle.get_mut().register_node_at(node_2, NodeId::root(), None);

        let binding = doc_handle.get();
        let get_node1 = binding.node_by_id(NodeId::from(1usize)).unwrap();
        let get_node2 = binding.node_by_id(NodeId::from(2usize)).unwrap();

        let NodeDataTypeInternal::Element(_) = &get_node1.data else {
            panic!()
        };
        assert_eq!(get_node1.id(), NodeId::from(1usize));

        let NodeDataTypeInternal::Element(_) = &get_node2.data else {
            panic!()
        };
        assert_eq!(get_node2.id(), NodeId::from(2usize));
    }

    #[test]
    fn document_task_queue() {
        let doc_handle: Handle = DocumentBuilderImpl::new_document(None);

        // Using task queue to create the following structure initially:
        // <div>
        //   <p>
        //     <!-- comment inside p -->
        //     hey
        //   </p>
        //   <!-- comment inside div -->
        // </div>

        // then flush the queue and use it again to add an attribute to <p>:
        // <p id="myid">hey</p>
        let mut task_queue = DocumentTaskQueue::new(doc_handle.clone());

        // NOTE: only elements return the ID
        let div_id = task_queue.create_element("div", NodeId::root(), None, HTML_NAMESPACE, Location::default());
        assert_eq!(div_id, NodeId::from(1usize));

        let p_id = task_queue.create_element("p", div_id, None, HTML_NAMESPACE, Location::default());
        assert_eq!(p_id, NodeId::from(2usize));

        task_queue.create_comment("comment inside p", p_id, Location::default());
        task_queue.create_text("hey", p_id, Location::default());
        task_queue.create_comment("comment inside div", div_id, Location::default());

        // at this point, the DOM should have NO nodes (besides root)
        assert_eq!(doc_handle.get().node_count(), 1);

        // validate our queue is loaded
        assert!(!task_queue.is_empty());
        let errors = task_queue.flush();
        assert!(errors.is_empty());

        // validate queue is empty
        assert!(task_queue.is_empty());

        // DOM should now have all our nodes
        assert_eq!(doc_handle.get().arena.node_count(), 6);

        // NOTE: these checks are scoped separately since this is using an
        // immutable borrow, and we make a mutable borrow after (to insert the attribute).
        // We need this immutable borrow to die off before making a new mutable borrow
        // (and again an immutable borrow for validation afterward)
        {
            // validate DOM is correctly laid out
            let doc_read = doc_handle.get();
            let root = doc_read.get_root(); // <!DOCTYPE html>
            let root_children = &root.children;

            // div child
            let div_child = doc_read.node_by_id(root_children[0]).unwrap();
            assert_eq!(div_child.type_of(), NodeType::ElementNode);
            assert_eq!(div_child.get_element_data().unwrap().name, "div");
            let div_children = &div_child.children;

            // p child
            let p_child = doc_read.node_by_id(div_children[0]).unwrap();
            assert_eq!(p_child.type_of(), NodeType::ElementNode);
            assert_eq!(p_child.get_element_data().unwrap().name, "p");
            let p_children = &p_child.children;

            // comment inside p
            let p_comment = doc_read.node_by_id(p_children[0]).unwrap();
            assert_eq!(p_comment.type_of(), NodeType::CommentNode);
            let NodeDataTypeInternal::Comment(p_comment_data) = &p_comment.data else {
                panic!()
            };
            assert_eq!(p_comment_data.value, "comment inside p");

            // body inside p
            let p_body = doc_read.node_by_id(p_children[1]).unwrap();
            assert_eq!(p_body.type_of(), NodeType::TextNode);
            let NodeDataTypeInternal::Text(p_body_data) = &p_body.data else {
                panic!()
            };
            assert_eq!(p_body_data.value, "hey");

            // comment inside div
            let div_comment = doc_read.node_by_id(div_children[1]).unwrap();
            assert_eq!(div_comment.type_of(), NodeType::CommentNode);
            let NodeDataTypeInternal::Comment(div_comment_data) = &div_comment.data else {
                panic!()
            };
            assert_eq!(div_comment_data.value, "comment inside div");
        }

        // use task queue again to add an ID attribute
        // NOTE: inserting attribute in task queue always succeeds
        // since it doesn't touch DOM until flush
        let _ = task_queue.insert_attribute("id", "myid", p_id, Location::default());
        let errors = task_queue.flush();
        println!("{:?}", errors);
        assert!(errors.is_empty());

        let doc_read = doc_handle.get();
        // validate ID is searchable in dom
        assert_eq!(*doc_read.named_id_elements.get("myid").unwrap(), p_id);

        // validate attribute is applied to underlying element
        let p_node = doc_read.node_by_id(p_id).unwrap();
        let NodeDataTypeInternal::Element(p_element) = &p_node.data else {
            panic!()
        };
        assert_eq!(p_element.attributes().get("id").unwrap(), "myid");
    }

    #[test]
    fn task_queue_insert_attribute_failues() {
        let doc_handle: Handle = DocumentBuilderImpl::new_document(None);

        let mut task_queue = DocumentTaskQueue::new(doc_handle.clone());
        let div_id = task_queue.create_element("div", NodeId::root(), None, HTML_NAMESPACE, Location::default());
        task_queue.create_comment("content", div_id, Location::default()); // this is NodeId::from(2)
        task_queue.flush();

        // NOTE: inserting attribute in task queue always succeeds
        // since it doesn't touch DOM until flush
        let _ = task_queue.insert_attribute("id", "myid", NodeId::from(1usize), Location::default());
        let _ = task_queue.insert_attribute("id", "myid", NodeId::from(2usize), Location::default());
        let _ = task_queue.insert_attribute("id", "otherid", NodeId::from(2usize), Location::default());
        let _ = task_queue.insert_attribute("id", "dummyid", NodeId::from(42usize), Location::default());
        let _ = task_queue.insert_attribute("id", "my id", NodeId::from(1usize), Location::default());
        let _ = task_queue.insert_attribute("id", "123", NodeId::from(1usize), Location::default());
        let _ = task_queue.insert_attribute("id", "", NodeId::from(1usize), Location::default());
        let errors = task_queue.flush();
        for error in &errors {
            println!("{}", error);
        }
        assert_eq!(errors.len(), 5);
        assert_eq!(errors[0], "ID attribute value 'myid' already exists in DOM");
        assert_eq!(errors[1], "Node id 2 is not an element");
        assert_eq!(errors[2], "Node id 42 not found");
        assert_eq!(errors[3], "ID attribute value 'my id' did not pass validation");
        assert_eq!(errors[4], "ID attribute value '' did not pass validation");

        // validate that invalid changes did not apply to DOM
        let doc_read = doc_handle.get();
        assert!(!doc_read.named_id_elements.contains_key("my id"));
        assert!(!doc_read.named_id_elements.contains_key(""));
    }

    // this is basically a replica of document_task_queue() test
    // but using tree builder directly instead of the task queue
    #[test]
    fn document_tree_builder() {
        let mut doc_handle: Handle = DocumentBuilderImpl::new_document(None);

        // Using tree builder to create the following structure:
        // <div>
        //   <p id="myid">
        //     <!-- comment inside p -->
        //     hey
        //   </p>
        //   <!-- comment inside div -->
        // </div>

        // NOTE: only elements return the ID
        let div_node = Document::new_element_node(
            doc_handle.clone(),
            "div",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let div_id = doc_handle.get_mut().register_node_at(div_node, NodeId::root(), None);
        assert_eq!(div_id, NodeId::from(1usize));

        let p_node = Document::new_element_node(
            doc_handle.clone(),
            "p",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let p_id = doc_handle.get_mut().register_node_at(p_node, div_id, None);
        assert_eq!(p_id, NodeId::from(2usize));

        let node = Document::new_comment_node(doc_handle.clone(), "comment inside p", Location::default());
        doc_handle.get_mut().register_node_at(node, p_id, None);

        let node = Document::new_text_node(doc_handle.clone(), "hey", Location::default());
        doc_handle.get_mut().register_node_at(node, p_id, None);

        let node = Document::new_comment_node(doc_handle.clone(), "comment inside div", Location::default());
        doc_handle.get_mut().register_node_at(node, div_id, None);

        let mut binding = doc_handle.get_mut();
        let mut node = binding.cloned_node_by_id(p_id).unwrap();
        if let Some(data) = node.get_element_data_mut() {
            data.add_attribute("id", "myid");
        }
        binding.update_node(node);
        // binding.add_named_id("myid", p_id);
        drop(binding);

        // DOM should now have all our nodes
        assert_eq!(doc_handle.get().node_count(), 6);

        // validate DOM is correctly laid out
        let doc_read = doc_handle.get();
        let root = doc_read.get_root(); // <!DOCTYPE html>
        let root_children = &root.children;

        // div child
        let div_child = doc_read.node_by_id(root_children[0]).unwrap();
        assert_eq!(div_child.type_of(), NodeType::ElementNode);
        assert_eq!(div_child.get_element_data().unwrap().name, "div");
        let div_children = &div_child.children;

        // p child
        let p_child = doc_read.node_by_id(div_children[0]).unwrap();
        assert_eq!(p_child.type_of(), NodeType::ElementNode);
        assert_eq!(p_child.get_element_data().unwrap().name, "p");
        let p_children = &p_child.children;

        // comment inside p
        let p_comment = doc_read.node_by_id(p_children[0]).unwrap();
        assert_eq!(p_comment.type_of(), NodeType::CommentNode);
        let NodeDataTypeInternal::Comment(p_comment_data) = &p_comment.data else {
            panic!()
        };
        assert_eq!(p_comment_data.value, "comment inside p");

        // body inside p
        let p_body = doc_read.node_by_id(p_children[1]).unwrap();
        assert_eq!(p_body.type_of(), NodeType::TextNode);
        let NodeDataTypeInternal::Text(p_body_data) = &p_body.data else {
            panic!()
        };
        assert_eq!(p_body_data.value, "hey");

        // comment inside div
        let div_comment = doc_read.node_by_id(div_children[1]).unwrap();
        assert_eq!(div_comment.type_of(), NodeType::CommentNode);
        let NodeDataTypeInternal::Comment(div_comment_data) = &div_comment.data else {
            panic!()
        };
        assert_eq!(div_comment_data.value, "comment inside div");

        // validate ID is searchable in dom
        assert_eq!(*doc_read.named_id_elements.get("myid").unwrap(), p_id);

        // validate attribute is applied to underlying element
        let p_node = doc_read.node_by_id(p_id).unwrap();
        let NodeDataTypeInternal::Element(p_element) = &p_node.data else {
            panic!()
        };
        assert_eq!(p_element.attributes().get("id").unwrap(), "myid");
    }

    #[test]
    fn insert_generic_attribute() {
        let mut doc_handle: Handle = DocumentBuilderImpl::new_document(None);

        let node = Document::new_element_node(
            doc_handle.clone(),
            "div",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let node_id = doc_handle.get_mut().register_node_at(node, NodeId::root(), None);

        let mut binding = doc_handle.get_mut();
        let mut node = binding.cloned_node_by_id(node_id).unwrap();
        if let Some(data) = node.get_element_data_mut() {
            data.add_attribute("key", "value");
            binding.update_node(node);
        }
        drop(binding);

        let doc_read = doc_handle.get();
        let Some(data) = doc_read.node_by_id(node_id).unwrap().get_element_data() else {
            panic!()
        };
        assert_eq!(data.attributes().get("key").unwrap(), "value");
    }

    #[test]
    fn task_queue_insert_generic_attribute() {
        let doc_handle: Handle = DocumentBuilderImpl::new_document(None);

        let mut task_queue = DocumentTaskQueue::new(doc_handle.clone());
        let div_id = task_queue.create_element("div", NodeId::root(), None, HTML_NAMESPACE, Location::default());
        let _ = task_queue.insert_attribute("key", "value", div_id, Location::default());
        let errors = task_queue.flush();
        assert!(errors.is_empty());
        let doc_read = doc_handle.get();
        let NodeDataTypeInternal::Element(element) = &doc_read.node_by_id(div_id).unwrap().data else {
            panic!()
        };
        assert_eq!(element.attributes().get("key").unwrap(), "value");
    }

    #[test]
    fn insert_class_attribute() {
        let mut doc_handle: Handle = DocumentBuilderImpl::new_document(None);

        let div_node = Document::new_element_node(
            doc_handle.clone(),
            "div",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let div_id = doc_handle.get_mut().register_node_at(div_node, NodeId::root(), None);

        let mut binding = doc_handle.get_mut();
        let mut node = binding.cloned_node_by_id(div_id).unwrap();
        if let Some(data) = node.get_element_data_mut() {
            data.add_attribute("class", "one two three");
            binding.update_node(node);
        }
        drop(binding);

        let binding = doc_handle.get();
        let NodeDataTypeInternal::Element(element_data) = &binding.node_by_id(div_id).unwrap().data else {
            panic!()
        };
        assert!(element_data.classlist().contains("one"));
        assert!(element_data.classlist().contains("two"));
        assert!(element_data.classlist().contains("three"));
    }

    #[test]
    fn task_queue_insert_class_attribute() {
        let doc_handle: Handle = DocumentBuilderImpl::new_document(None);

        let mut task_queue = DocumentTaskQueue::new(doc_handle.clone());
        let div_id = task_queue.create_element("div", NodeId::root(), None, HTML_NAMESPACE, Location::default());
        let _ = task_queue.insert_attribute("class", "one two three", div_id, Location::default());
        let errors = task_queue.flush();
        println!("{:?}", errors);
        assert!(errors.is_empty());

        let binding = doc_handle.get();
        let element = binding.node_by_id(div_id).unwrap().get_element_data().unwrap();

        assert!(element.classlist().contains("one"));
        assert!(element.classlist().contains("two"));
        assert!(element.classlist().contains("three"));
    }

    #[test]
    fn uninitialized_query() {
        let doc_handle: Handle = DocumentBuilderImpl::new_document(None);

        let query = Query::new();
        let found_ids = DocumentQuery::query(doc_handle.clone(), &query);
        if let Err(err) = found_ids {
            assert_eq!(
                err.to_string(),
                "query: generic error: Query predicate is uninitialized"
            );
        } else {
            panic!()
        }
    }

    #[test]
    fn single_query_equals_tag_find_first() {
        // <div>
        //     <div>
        //         <p>
        //     <p>
        // <div>
        //     <p>
        // <p>
        let mut doc_handle: Handle = DocumentBuilderImpl::new_document(None);

        let div_node = Document::new_element_node(
            doc_handle.clone(),
            "div",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let div_id = doc_handle.get_mut().register_node_at(div_node, NodeId::root(), None);

        let div_node_2 = Document::new_element_node(
            doc_handle.clone(),
            "div",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let div_id_2 = doc_handle.get_mut().register_node_at(div_node_2, div_id, None);

        let p_node = Document::new_element_node(
            doc_handle.clone(),
            "p",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let p_id = doc_handle.get_mut().register_node_at(p_node, div_id_2, None);

        let p_node = Document::new_element_node(
            doc_handle.clone(),
            "p",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let _ = doc_handle.get_mut().register_node_at(p_node, div_id, None);

        let div_node_3 = Document::new_element_node(
            doc_handle.clone(),
            "div",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let div_id_3 = doc_handle.get_mut().register_node_at(div_node_3, NodeId::root(), None);

        let p_node = Document::new_element_node(
            doc_handle.clone(),
            "p",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let _ = doc_handle.get_mut().register_node_at(p_node, div_id_3, None);

        let p_node = Document::new_element_node(
            doc_handle.clone(),
            "p",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        doc_handle.get_mut().register_node_at(p_node, NodeId::root(), None);

        let query = Query::new().equals_tag("p").find_first();
        let found_ids = DocumentQuery::query(doc_handle.clone(), &query).unwrap();
        assert_eq!(found_ids.len(), 1);
        assert_eq!(found_ids, [p_id]);
    }

    #[test]
    fn single_query_equals_tag_find_all() {
        // <div>
        //     <div>
        //         <p>
        //     <p>
        // <div>
        //     <p>
        // <p>
        let mut doc_handle: Handle = DocumentBuilderImpl::new_document(None);

        let div_node = Document::new_element_node(
            doc_handle.clone(),
            "div",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let div_id = doc_handle.get_mut().register_node_at(div_node, NodeId::root(), None);

        let div_node_2 = Document::new_element_node(
            doc_handle.clone(),
            "div",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let div_id_2 = doc_handle.get_mut().register_node_at(div_node_2, div_id, None);

        let p_node = Document::new_element_node(
            doc_handle.clone(),
            "p",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let p_id = doc_handle.get_mut().register_node_at(p_node, div_id_2, None);

        let p_node_2 = Document::new_element_node(
            doc_handle.clone(),
            "p",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let p_id_2 = doc_handle.get_mut().register_node_at(p_node_2, div_id, None);

        let div_node_3 = Document::new_element_node(
            doc_handle.clone(),
            "div",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let div_id_3 = doc_handle.get_mut().register_node_at(div_node_3, NodeId::root(), None);

        let p_node_3 = Document::new_element_node(
            doc_handle.clone(),
            "p",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let p_id_3 = doc_handle.get_mut().register_node_at(p_node_3, div_id_3, None);

        let p_node_4 = Document::new_element_node(
            doc_handle.clone(),
            "p",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let p_id_4 = doc_handle.get_mut().register_node_at(p_node_4, NodeId::root(), None);

        let query = Query::new().equals_tag("p").find_all();
        let found_ids = DocumentQuery::query(doc_handle.clone(), &query).unwrap();
        assert_eq!(found_ids.len(), 4);
        assert_eq!(found_ids, [p_id, p_id_2, p_id_3, p_id_4]);
    }

    #[test]
    fn single_query_equals_id() {
        // <div>
        //     <div>
        //         <p>
        //     <p id="myid">
        // <div>
        //     <p>
        // <p>
        let mut doc_handle: Handle = DocumentBuilderImpl::new_document(None);

        let div_node = Document::new_element_node(
            doc_handle.clone(),
            "div",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let div_id = doc_handle.get_mut().register_node_at(div_node, NodeId::root(), None);

        let div_node_2 = Document::new_element_node(
            doc_handle.clone(),
            "div",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let div_id_2 = doc_handle.get_mut().register_node_at(div_node_2, div_id, None);

        let p_node = Document::new_element_node(
            doc_handle.clone(),
            "p",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let _ = doc_handle.get_mut().register_node_at(p_node, div_id_2, None);

        let p_node_2 = Document::new_element_node(
            doc_handle.clone(),
            "p",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let p_id_2 = doc_handle.get_mut().register_node_at(p_node_2, div_id, None);

        let mut binding = doc_handle.get_mut();
        let mut node = binding.cloned_node_by_id(p_id_2).unwrap();
        if let Some(data) = node.get_element_data_mut() {
            data.add_attribute("id", "myid");
            binding.update_node(node);
        }
        drop(binding);

        let div_node_3 = Document::new_element_node(
            doc_handle.clone(),
            "div",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let div_id_3 = doc_handle.get_mut().register_node_at(div_node_3, NodeId::root(), None);

        let p_node = Document::new_element_node(
            doc_handle.clone(),
            "p",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let _ = doc_handle.get_mut().register_node_at(p_node, div_id_3, None);

        let p_node = Document::new_element_node(
            doc_handle.clone(),
            "p",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let _ = doc_handle.get_mut().register_node_at(p_node, NodeId::root(), None);

        let query = Query::new().equals_id("myid").find_first();
        let found_ids = DocumentQuery::query(doc_handle.clone(), &query).unwrap();
        assert_eq!(found_ids.len(), 1);
        assert_eq!(found_ids, [p_id_2]);
    }

    #[test]
    fn single_query_contains_class_find_first() {
        // <div>
        //     <div>
        //         <p class="one two">
        //     <p class="one">
        // <div>
        //     <p class="two three">
        // <p class="three">
        let mut doc_handle: Handle = DocumentBuilderImpl::new_document(None);

        let div_node = Document::new_element_node(
            doc_handle.clone(),
            "div",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let div_id = doc_handle.get_mut().register_node_at(div_node, NodeId::root(), None);

        let div_node_2 = Document::new_element_node(
            doc_handle.clone(),
            "div",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let div_id_2 = doc_handle.get_mut().register_node_at(div_node_2, div_id, None);

        let p_node = Document::new_element_node(
            doc_handle.clone(),
            "p",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let p_id = doc_handle.get_mut().register_node_at(p_node, div_id_2, None);

        let mut binding = doc_handle.get_mut();
        let mut node = binding.cloned_node_by_id(p_id).unwrap();
        if let Some(data) = node.get_element_data_mut() {
            data.add_attribute("class", "one two");
            binding.update_node(node);
        }
        drop(binding);

        let p_node_2 = Document::new_element_node(
            doc_handle.clone(),
            "p",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let _ = doc_handle.get_mut().register_node_at(p_node_2, div_id, None);

        let mut binding = doc_handle.get_mut();
        let mut node = binding.cloned_node_by_id(div_id).unwrap();
        if let Some(data) = node.get_element_data_mut() {
            data.add_attribute("class", "one");
            data.add_attribute("id", "myid");
            binding.update_node(node);
        }
        drop(binding);

        let div_node_3 = Document::new_element_node(
            doc_handle.clone(),
            "div",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let div_id_3 = doc_handle.get_mut().register_node_at(div_node_3, NodeId::root(), None);

        let p_node_3 = Document::new_element_node(
            doc_handle.clone(),
            "p",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let p_id_3 = doc_handle.get_mut().register_node_at(p_node_3, div_id_3, None);

        let mut binding = doc_handle.get_mut();
        let mut node = binding.cloned_node_by_id(p_id_3).unwrap();
        if let Some(data) = node.get_element_data_mut() {
            data.add_attribute("class", "two_tree");
            binding.update_node(node);
        }
        drop(binding);

        let p_node_4 = Document::new_element_node(
            doc_handle.clone(),
            "p",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let p_id_4 = doc_handle.get_mut().register_node_at(p_node_4, NodeId::root(), None);

        let mut binding = doc_handle.get_mut();
        let mut node = binding.cloned_node_by_id(p_id_4).unwrap();
        if let Some(data) = node.get_element_data_mut() {
            data.add_attribute("class", "three");
            binding.update_node(node);
        }
        drop(binding);

        let query = Query::new().contains_class("two").find_first();
        let found_ids = DocumentQuery::query(doc_handle.clone(), &query).unwrap();
        assert_eq!(found_ids.len(), 1);
        assert_eq!(found_ids, [p_id]);
    }

    #[test]
    fn single_query_contains_class_find_all() {
        // <div>
        //     <div>
        //         <p class="one two">
        //     <p class="one">
        // <div>
        //     <p class="two three">
        // <p class="three">
        let mut doc_handle: Handle = DocumentBuilderImpl::new_document(None);

        let div_node = Document::new_element_node(
            doc_handle.clone(),
            "div",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let div_id = doc_handle.get_mut().register_node_at(div_node, NodeId::root(), None);

        let div_node_2 = Document::new_element_node(
            doc_handle.clone(),
            "div",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let div_id_2 = doc_handle.get_mut().register_node_at(div_node_2, div_id, None);

        let p_node = Document::new_element_node(
            doc_handle.clone(),
            "p",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let p_id = doc_handle.get_mut().register_node_at(p_node, div_id_2, None);

        let mut binding = doc_handle.get_mut();
        let mut node = binding.cloned_node_by_id(p_id).unwrap();
        if let Some(data) = node.get_element_data_mut() {
            data.add_attribute("class", "one two");
            binding.update_node(node);
        }
        drop(binding);

        let p_node_2 = Document::new_element_node(
            doc_handle.clone(),
            "p",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let p_id_2 = doc_handle.get_mut().register_node_at(p_node_2, div_id, None);

        let mut binding = doc_handle.get_mut();
        let mut node = binding.cloned_node_by_id(p_id_2).unwrap();
        if let Some(data) = node.get_element_data_mut() {
            data.add_attribute("class", "one");
            data.add_attribute("id", "myid");
            binding.update_node(node);
        }
        drop(binding);

        let div_node_3 = Document::new_element_node(
            doc_handle.clone(),
            "div",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let div_id_3 = doc_handle.get_mut().register_node_at(div_node_3, NodeId::root(), None);

        let p_node_3 = Document::new_element_node(
            doc_handle.clone(),
            "p",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let p_id_3 = doc_handle.get_mut().register_node_at(p_node_3, div_id_3, None);

        let mut binding = doc_handle.get_mut();
        let mut node = binding.cloned_node_by_id(p_id_3).unwrap();
        if let Some(data) = node.get_element_data_mut() {
            data.add_attribute("class", "two three");
            binding.update_node(node);
        }
        drop(binding);

        let p_node_4 = Document::new_element_node(
            doc_handle.clone(),
            "p",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let p_id_4 = doc_handle.get_mut().register_node_at(p_node_4, NodeId::root(), None);

        let mut binding = doc_handle.get_mut();
        let mut node = binding.cloned_node_by_id(p_id_4).unwrap();
        if let Some(data) = node.get_element_data_mut() {
            data.add_attribute("class", "three");
            binding.update_node(node);
        }
        drop(binding);

        let query = Query::new().contains_class("two").find_all();
        let found_ids = DocumentQuery::query(doc_handle.clone(), &query).unwrap();
        assert_eq!(found_ids.len(), 2);
        assert_eq!(found_ids, [p_id, p_id_3]);
    }

    #[test]
    fn single_query_contains_attribute_find_first() {
        // <div>
        //     <div id="myid" style="somestyle">
        //         <p title="hey">
        //     <p>
        // <div style="otherstyle" id="otherid">
        //     <p>
        // <p title="yo" style="cat">
        let mut doc_handle: Handle = DocumentBuilderImpl::new_document(None);

        let div_node = Document::new_element_node(
            doc_handle.clone(),
            "div",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let div_id = doc_handle.get_mut().register_node_at(div_node, NodeId::root(), None);

        let div_node_2 = Document::new_element_node(
            doc_handle.clone(),
            "div",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let div_id_2 = doc_handle.get_mut().register_node_at(div_node_2, div_id, None);

        let mut binding = doc_handle.get_mut();
        let mut node = binding.cloned_node_by_id(div_id_2).unwrap();
        if let Some(data) = node.get_element_data_mut() {
            data.add_attribute("id", "myid");
            data.add_attribute("style", "somestyle");
            binding.update_node(node);
        }
        drop(binding);

        let p_node = Document::new_element_node(
            doc_handle.clone(),
            "p",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let p_id = doc_handle.get_mut().register_node_at(p_node, div_id_2, None);

        let mut binding = doc_handle.get_mut();
        let mut node = binding.cloned_node_by_id(p_id).unwrap();
        if let Some(data) = node.get_element_data_mut() {
            data.add_attribute("title", "key");
            binding.update_node(node);
        }
        drop(binding);

        let p_node = Document::new_element_node(
            doc_handle.clone(),
            "p",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let _ = doc_handle.get_mut().register_node_at(p_node, div_id, None);

        let div_node_3 = Document::new_element_node(
            doc_handle.clone(),
            "div",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let div_id_3 = doc_handle.get_mut().register_node_at(div_node_3, NodeId::root(), None);

        let mut binding = doc_handle.get_mut();
        let mut node = binding.cloned_node_by_id(div_id_3).unwrap();
        if let Some(data) = node.get_element_data_mut() {
            data.add_attribute("style", "otherstyle");
            data.add_attribute("id", "otherid");
            binding.update_node(node);
        }
        drop(binding);

        let p_node = Document::new_element_node(
            doc_handle.clone(),
            "p",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let _ = doc_handle.get_mut().register_node_at(p_node, div_id_3, None);

        let p_node_4 = Document::new_element_node(
            doc_handle.clone(),
            "p",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let p_id_4 = doc_handle.get_mut().register_node_at(p_node_4, NodeId::root(), None);

        let mut binding = doc_handle.get_mut();
        let mut node = binding.cloned_node_by_id(p_id_4).unwrap();
        if let Some(data) = node.get_element_data_mut() {
            data.add_attribute("title", "yo");
            data.add_attribute("style", "cat");
            binding.update_node(node);
        }
        drop(binding);

        let query = Query::new().contains_attribute("style").find_first();
        let found_ids = DocumentQuery::query(doc_handle.clone(), &query).unwrap();
        assert_eq!(found_ids.len(), 1);
        assert_eq!(found_ids, [div_id_2]);
    }

    #[test]
    fn single_query_contains_attribute_find_all() {
        // <div>
        //     <div id="myid" style="somestyle">
        //         <p title="hey">
        //     <p>
        // <div style="otherstyle" id="otherid">
        //     <p>
        // <p title="yo" style="cat">
        let mut doc_handle: Handle = DocumentBuilderImpl::new_document(None);

        let div_node = Document::new_element_node(
            doc_handle.clone(),
            "div",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let div_id = doc_handle.get_mut().register_node_at(div_node, NodeId::root(), None);

        let div_node_2 = Document::new_element_node(
            doc_handle.clone(),
            "div",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let div_id_2 = doc_handle.get_mut().register_node_at(div_node_2, div_id, None);

        let mut binding = doc_handle.get_mut();
        let mut node = binding.cloned_node_by_id(div_id_2).unwrap();
        if let Some(data) = node.get_element_data_mut() {
            data.add_attribute("id", "myid");
            data.add_attribute("style", "somestyle");
            binding.update_node(node);
        }
        drop(binding);

        let p_node = Document::new_element_node(
            doc_handle.clone(),
            "p",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let p_id = doc_handle.get_mut().register_node_at(p_node, div_id_2, None);

        let mut binding = doc_handle.get_mut();
        let mut node = binding.cloned_node_by_id(p_id).unwrap();
        if let Some(data) = node.get_element_data_mut() {
            data.add_attribute("title", "key");
            binding.update_node(node);
        }
        drop(binding);

        let p_node = Document::new_element_node(
            doc_handle.clone(),
            "p",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let _ = doc_handle.get_mut().register_node_at(p_node, div_id, None);

        let div_node_3 = Document::new_element_node(
            doc_handle.clone(),
            "div",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let div_id_3 = doc_handle.get_mut().register_node_at(div_node_3, NodeId::root(), None);

        let mut binding = doc_handle.get_mut();
        let mut node = binding.cloned_node_by_id(div_id_3).unwrap();
        if let Some(data) = node.get_element_data_mut() {
            data.add_attribute("style", "otherstyle");
            data.add_attribute("id", "otherid");

            binding.update_node(node);
        }
        drop(binding);

        let p_node = Document::new_element_node(
            doc_handle.clone(),
            "p",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let _ = doc_handle.get_mut().register_node_at(p_node, div_id_3, None);

        let p_node_4 = Document::new_element_node(
            doc_handle.clone(),
            "p",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let p_id_4 = doc_handle.get_mut().register_node_at(p_node_4, NodeId::root(), None);

        let mut binding = doc_handle.get_mut();
        let mut node = binding.cloned_node_by_id(p_id_4).unwrap();
        if let Some(data) = node.get_element_data_mut() {
            data.add_attribute("title", "yo");
            data.add_attribute("style", "cat");

            binding.update_node(node);
        }
        drop(binding);

        let query = Query::new().contains_attribute("style").find_all();
        let found_ids = DocumentQuery::query(doc_handle.clone(), &query).unwrap();
        assert_eq!(found_ids.len(), 3);
        assert_eq!(found_ids, [div_id_2, div_id_3, p_id_4]);
    }

    #[test]
    fn single_query_contains_child_find_first() {
        // <div>
        //     <div>
        //         <p>
        //     <p>
        // <div>
        //     <p>
        // <p>
        let mut doc_handle: Handle = DocumentBuilderImpl::new_document(None);

        let div_node = Document::new_element_node(
            doc_handle.clone(),
            "div",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let div_id = doc_handle.get_mut().register_node_at(div_node, NodeId::root(), None);

        let div_node_2 = Document::new_element_node(
            doc_handle.clone(),
            "div",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let div_id_2 = doc_handle.get_mut().register_node_at(div_node_2, div_id, None);

        let p_node = Document::new_element_node(
            doc_handle.clone(),
            "p",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let _ = doc_handle.get_mut().register_node_at(p_node, div_id_2, None);

        let p_node = Document::new_element_node(
            doc_handle.clone(),
            "p",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let _ = doc_handle.get_mut().register_node_at(p_node, div_id, None);

        let div_node_3 = Document::new_element_node(
            doc_handle.clone(),
            "div",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let div_id_3 = doc_handle.get_mut().register_node_at(div_node_3, NodeId::root(), None);

        let p_node = Document::new_element_node(
            doc_handle.clone(),
            "p",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let _ = doc_handle.get_mut().register_node_at(p_node, div_id_3, None);

        let p_node = Document::new_element_node(
            doc_handle.clone(),
            "p",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let _ = doc_handle.get_mut().register_node_at(p_node, NodeId::root(), None);

        let query = Query::new().contains_child_tag("p").find_first();
        let found_ids = DocumentQuery::query(doc_handle.clone(), &query).unwrap();
        assert_eq!(found_ids.len(), 1);
        assert_eq!(found_ids, [NodeId::root()]);
    }

    #[test]
    fn single_query_contains_child_find_all() {
        // <div>
        //     <div>
        //         <p>
        //     <p>
        // <div>
        //     <p>
        // <p>
        let mut doc_handle: Handle = DocumentBuilderImpl::new_document(None);

        let div_node = Document::new_element_node(
            doc_handle.clone(),
            "div",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let div_id = doc_handle.get_mut().register_node_at(div_node, NodeId::root(), None);

        let div_node_2 = Document::new_element_node(
            doc_handle.clone(),
            "div",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let div_id_2 = doc_handle.get_mut().register_node_at(div_node_2, div_id, None);

        let p_node = Document::new_element_node(
            doc_handle.clone(),
            "p",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let _ = doc_handle.get_mut().register_node_at(p_node, div_id_2, None);

        let p_node = Document::new_element_node(
            doc_handle.clone(),
            "p",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let _ = doc_handle.get_mut().register_node_at(p_node, div_id, None);

        let div_node_3 = Document::new_element_node(
            doc_handle.clone(),
            "div",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let div_id_3 = doc_handle.get_mut().register_node_at(div_node_3, NodeId::root(), None);

        let p_node = Document::new_element_node(
            doc_handle.clone(),
            "p",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let _ = doc_handle.get_mut().register_node_at(p_node, div_id_3, None);

        let p_node = Document::new_element_node(
            doc_handle.clone(),
            "p",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let _ = doc_handle.get_mut().register_node_at(p_node, NodeId::root(), None);

        let query = Query::new().contains_child_tag("p").find_all();
        let found_ids = DocumentQuery::query(doc_handle.clone(), &query).unwrap();
        assert_eq!(found_ids.len(), 4);
        assert_eq!(found_ids, [NodeId::root(), div_id, div_id_2, div_id_3]);
    }

    #[test]
    fn single_query_has_parent_find_first() {
        // <div>
        //     <div>
        //         <p>
        //     <p>
        // <div>
        //     <p>
        // <p>
        let mut doc_handle: Handle = DocumentBuilderImpl::new_document(None);

        let div_node = Document::new_element_node(
            doc_handle.clone(),
            "div",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let div_id = doc_handle.get_mut().register_node_at(div_node, NodeId::root(), None);

        let div_node_2 = Document::new_element_node(
            doc_handle.clone(),
            "div",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let div_id_2 = doc_handle.get_mut().register_node_at(div_node_2, div_id, None);

        let p_node = Document::new_element_node(
            doc_handle.clone(),
            "p",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let _ = doc_handle.get_mut().register_node_at(p_node, div_id_2, None);

        let p_node = Document::new_element_node(
            doc_handle.clone(),
            "p",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let _ = doc_handle.get_mut().register_node_at(p_node, div_id, None);

        let div_node_3 = Document::new_element_node(
            doc_handle.clone(),
            "div",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let div_id_3 = doc_handle.get_mut().register_node_at(div_node_3, NodeId::root(), None);

        let p_node = Document::new_element_node(
            doc_handle.clone(),
            "p",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let _ = doc_handle.get_mut().register_node_at(p_node, div_id_3, None);

        let p_node = Document::new_element_node(
            doc_handle.clone(),
            "p",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let _ = doc_handle.get_mut().register_node_at(p_node, NodeId::root(), None);

        let query = Query::new().has_parent_tag("div").find_first();
        let found_ids = DocumentQuery::query(doc_handle.clone(), &query).unwrap();
        assert_eq!(found_ids.len(), 1);
        assert_eq!(found_ids, [div_id_2]);
    }

    #[test]
    fn single_query_has_parent_find_all() {
        // <div>
        //     <div>
        //         <p>
        //     <p>
        // <div>
        //     <p>
        // <p>
        let mut doc_handle: Handle = DocumentBuilderImpl::new_document(None);

        let div_node = Document::new_element_node(
            doc_handle.clone(),
            "div",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let div_id = doc_handle.get_mut().register_node_at(div_node, NodeId::root(), None);

        let div_node_2 = Document::new_element_node(
            doc_handle.clone(),
            "div",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let div_id_2 = doc_handle.get_mut().register_node_at(div_node_2, div_id, None);

        let p_node = Document::new_element_node(
            doc_handle.clone(),
            "p",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let p_id = doc_handle.get_mut().register_node_at(p_node, div_id_2, None);

        let p_node_2 = Document::new_element_node(
            doc_handle.clone(),
            "p",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let p_id_2 = doc_handle.get_mut().register_node_at(p_node_2, div_id, None);

        let div_node_3 = Document::new_element_node(
            doc_handle.clone(),
            "div",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let div_id_3 = doc_handle.get_mut().register_node_at(div_node_3, NodeId::root(), None);

        let p_node_3 = Document::new_element_node(
            doc_handle.clone(),
            "p",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let p_id_3 = doc_handle.get_mut().register_node_at(p_node_3, div_id_3, None);

        let p_node = Document::new_element_node(
            doc_handle.clone(),
            "p",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let _ = doc_handle.get_mut().register_node_at(p_node, NodeId::root(), None);

        let query = Query::new().has_parent_tag("div").find_all();
        let found_ids = DocumentQuery::query(doc_handle.clone(), &query).unwrap();
        assert_eq!(found_ids.len(), 4);
        assert_eq!(found_ids, [div_id_2, p_id, p_id_2, p_id_3]);
    }

    #[test]
    fn tree_iterator() {
        let mut doc_handle: Handle = DocumentBuilderImpl::new_document(None);

        // <div>
        //     <div>
        //         <p>first p tag
        //         <p>second p tag
        //     <p>third p tag
        let div_node = Document::new_element_node(
            doc_handle.clone(),
            "div",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let div_id = doc_handle.get_mut().register_node_at(div_node, NodeId::root(), None);

        let div_node_2 = Document::new_element_node(
            doc_handle.clone(),
            "div",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let div_id_2 = doc_handle.get_mut().register_node_at(div_node_2, div_id, None);

        let p_node = Document::new_element_node(
            doc_handle.clone(),
            "p",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let p_id = doc_handle.get_mut().register_node_at(p_node, div_id_2, None);

        let text_node = Document::new_text_node(doc_handle.clone(), "first p tag", Location::default());
        let text_id = doc_handle.get_mut().register_node_at(text_node, p_id, None);

        let p_node_2 = Document::new_element_node(
            doc_handle.clone(),
            "p",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let p_id_2 = doc_handle.get_mut().register_node_at(p_node_2, div_id_2, None);

        let text_node_2 = Document::new_text_node(doc_handle.clone(), "second p tag", Location::default());
        let text_id_2 = doc_handle.get_mut().register_node_at(text_node_2, p_id_2, None);
        let p_node_3 = Document::new_element_node(
            doc_handle.clone(),
            "p",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let p_id_3 = doc_handle.get_mut().register_node_at(p_node_3, div_id, None);

        let text_node_3 = Document::new_text_node(doc_handle.clone(), "third p tag", Location::default());
        let text_id_3 = doc_handle.get_mut().register_node_at(text_node_3, p_id_3, None);

        let tree_iterator = TreeIterator::new(doc_handle.clone());

        let expected_order = vec![
            NodeId::root(),
            div_id,
            div_id_2,
            p_id,
            text_id,
            p_id_2,
            text_id_2,
            p_id_3,
            text_id_3,
        ];

        let mut traversed_nodes = Vec::new();
        for current_node_id in tree_iterator {
            traversed_nodes.push(current_node_id);
        }

        assert_eq!(expected_order, traversed_nodes);
    }

    #[test]
    fn tree_iterator_mutation() {
        let mut doc_handle: Handle = DocumentBuilderImpl::new_document(None);

        let div_node = Document::new_element_node(
            doc_handle.clone(),
            "div",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let div_id = doc_handle.get_mut().register_node_at(div_node, NodeId::root(), None);

        let mut tree_iterator = TreeIterator::new(doc_handle.clone());
        let mut current_node_id;

        current_node_id = tree_iterator.next();
        assert_eq!(current_node_id.unwrap(), NodeId::root());

        // we mutate the tree while the iterator is still "open"
        let div_node_2 = Document::new_element_node(
            doc_handle.clone(),
            "div_1",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        let div_id_2 = doc_handle.get_mut().register_node_at(div_node_2, NodeId::root(), None);

        current_node_id = tree_iterator.next();
        assert_eq!(current_node_id.unwrap(), div_id);

        // and find this node on next iteration
        current_node_id = tree_iterator.next();
        assert_eq!(current_node_id.unwrap(), div_id_2);
    }
}
