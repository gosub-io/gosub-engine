use crate::config::HasDocument;
use crate::node::{Node, QuirksMode};
use gosub_shared::byte_stream::Location;
use gosub_shared::node::NodeId;
use std::collections::HashMap;
use std::fmt::{Debug, Display};
use url::Url;

/// Type of the given document
#[derive(PartialEq, Debug, Copy, Clone)]
pub enum DocumentType {
    /// HTML document
    HTML,
    /// Iframe source document
    IframeSrcDoc,
}

pub trait DocumentBuilder<C: HasDocument> {
    fn new_document(url: Option<Url>) -> C::Document;
    fn new_document_fragment(context_node: &<C::Document as Document<C>>::Node, quirks_mode: QuirksMode)
        -> C::Document;
}

pub trait DocumentFragment<C: HasDocument>: Sized + Clone + PartialEq {
    fn new(node_id: NodeId) -> Self;
}

pub trait Document<C: HasDocument<Document = Self>>: Sized + Display + Debug + PartialEq + 'static {
    type Node: Node<C>;

    // Creates a new doc with an optional document root node
    #[allow(clippy::new_ret_no_self)]
    fn new(document_type: DocumentType, url: Option<Url>, root_node: Option<Self::Node>) -> C::Document;

    // /// Creates a new document with an optional document root node
    // fn new_with_handle(document_type: DocumentType, url: Option<Url>, location: &Location, root_node: Option<&Self::Node>) -> DocumentHandle<Self>;

    // /// Returns the document handle for this document
    // fn handle(&self) -> DocumentHandle<Self, C>;

    /// Location of the document (URL, file path, etc.)
    fn url(&self) -> Option<Url>;

    fn set_quirks_mode(&mut self, quirks_mode: QuirksMode);
    fn quirks_mode(&self) -> QuirksMode;
    fn set_doctype(&mut self, doctype: DocumentType);
    fn doctype(&self) -> DocumentType;

    /// Return a node by its node ID
    fn node_by_id(&self, node_id: NodeId) -> Option<&Self::Node>;

    // Return an element node by the "id" attribute
    fn node_by_named_id(&self, id: &str) -> Option<&Self::Node>;

    // fn add_named_id(&mut self, id: &str, node_id: NodeId);
    // /// Remove a named ID from the document
    // fn remove_named_id(&mut self, id: &str);

    fn stylesheets(&self) -> &Vec<C::Stylesheet>;
    fn add_stylesheet(&mut self, stylesheet: C::Stylesheet);

    /// Return the root node of the document
    fn get_root(&self) -> &Self::Node;
    // fn get_root_mut(&mut self) -> &mut Self::Node;

    fn attach_node(&mut self, node_id: NodeId, parent_id: NodeId, position: Option<usize>);
    fn detach_node(&mut self, node_id: NodeId);
    fn relocate_node(&mut self, node_id: NodeId, parent_id: NodeId);

    /// Updates a node into the document
    fn update_node(&mut self, node: Self::Node);

    // Updates a node that is referenced into the document. This is useful for instance when a node is fetched with node_by_id() for instance.
    fn update_node_ref(&mut self, node: &Self::Node);

    // /// Return the parent node from a given ID
    // fn parent_node(&self, node: &Self::Node) -> Option<&Self::Node>;

    /// Removes a node from the document
    fn delete_node_by_id(&mut self, node_id: NodeId);

    /// Returns the next sibling of the reference node
    fn get_next_sibling(&self, node: NodeId) -> Option<NodeId>;

    /// Return number of nodes in the document
    fn node_count(&self) -> usize;

    /// Returns the next node ID that will be used when registering a new node
    fn peek_next_id(&self) -> NodeId;

    /// Register a new node
    fn register_node(&mut self, node: Self::Node) -> NodeId;
    /// Register a new node at a specific position
    fn register_node_at(&mut self, node: Self::Node, parent_id: NodeId, position: Option<usize>) -> NodeId;

    /// Node creation methods. The root node is needed in order to fetch the document handle (it can't be created from the document itself)
    fn new_document_node(quirks_mode: QuirksMode, location: Location) -> Self::Node;
    fn new_doctype_node(name: &str, public_id: Option<&str>, system_id: Option<&str>, location: Location)
        -> Self::Node;
    fn new_comment_node(comment: &str, location: Location) -> Self::Node;
    fn new_text_node(value: &str, location: Location) -> Self::Node;
    fn new_element_node(
        name: &str,
        namespace: Option<&str>,
        attributes: HashMap<String, String>,
        location: Location,
    ) -> Self::Node;

    fn write(&self) -> String;
    fn write_from_node(&self, node_id: NodeId) -> String;
    fn cloned_node_by_id(&self, node_id: NodeId) -> Option<Self::Node>;
}
