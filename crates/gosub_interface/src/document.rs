use crate::config::HasCssSystem;
use crate::css3::CssSystem;
use crate::node::{NodeType, QuirksMode};
use gosub_shared::byte_stream::Location;
use gosub_shared::node::NodeId;
use std::collections::HashMap;
use std::fmt::{Debug, Display};
use url::Url;

/// Whether this is a regular HTML document or a fragment (e.g. iframe srcdoc)
#[derive(PartialEq, Debug, Copy, Clone)]
pub enum DocumentType {
    HTML,
    IframeSrcDoc,
}

/// Storage-agnostic document interface.
///
/// All node data is accessed through `NodeId` handles. The concrete storage
/// (arena, column store, slotmap, etc.) is entirely hidden behind this trait.
/// No `Node` struct is ever handed out — callers ask the document questions
/// about a node by its ID.
pub trait Document<C: HasCssSystem>: Sized + Display + Debug + PartialEq + 'static {
    // -----------------------------------------------------------------------
    // Construction
    // -----------------------------------------------------------------------

    /// Create a new empty document of the given type.
    fn new(document_type: DocumentType, url: Option<Url>) -> Self;

    /// Create an HTML fragment document with a root `<html>` node.
    fn new_fragment(quirks_mode: QuirksMode) -> Self;

    // -----------------------------------------------------------------------
    // Node creation — each returns the NodeId of the new node
    // -----------------------------------------------------------------------

    fn create_element(
        &mut self,
        name: &str,
        namespace: Option<&str>,
        attributes: HashMap<String, String>,
        location: Location,
    ) -> NodeId;

    fn create_text(&mut self, value: &str, location: Location) -> NodeId;
    fn create_comment(&mut self, value: &str, location: Location) -> NodeId;
    fn create_doctype(
        &mut self,
        name: &str,
        public_id: Option<&str>,
        system_id: Option<&str>,
        location: Location,
    ) -> NodeId;

    /// Deep-clone a node (and its subtree). Returns the new root NodeId.
    fn clone_node(&mut self, id: NodeId) -> NodeId;

    /// Shallow-copy a node: same type/data/attributes, no children, unattached.
    fn duplicate_node(&mut self, id: NodeId) -> NodeId;

    // -----------------------------------------------------------------------
    // Tree structure — all navigation returns NodeId, never &Node
    // -----------------------------------------------------------------------

    fn root(&self) -> NodeId;
    fn parent(&self, id: NodeId) -> Option<NodeId>;
    fn children(&self, id: NodeId) -> &[NodeId];
    fn next_sibling(&self, id: NodeId) -> Option<NodeId>;

    fn attach(&mut self, node: NodeId, parent: NodeId, position: Option<usize>);
    fn detach(&mut self, node: NodeId);
    fn remove(&mut self, node: NodeId);

    /// Detach a node from its current parent and attach it to a new parent.
    fn relocate_node(&mut self, node: NodeId, parent: NodeId);

    // -----------------------------------------------------------------------
    // Node type
    // -----------------------------------------------------------------------

    fn node_type(&self, id: NodeId) -> NodeType;

    // -----------------------------------------------------------------------
    // Element data
    // -----------------------------------------------------------------------

    fn tag_name(&self, id: NodeId) -> Option<&str>;
    fn namespace(&self, id: NodeId) -> Option<&str>;

    fn attribute(&self, id: NodeId, name: &str) -> Option<&str>;
    fn attributes(&self, id: NodeId) -> Option<&HashMap<String, String>>;
    fn set_attribute(&mut self, id: NodeId, name: &str, value: &str);
    fn remove_attribute(&mut self, id: NodeId, name: &str);

    fn add_class(&mut self, id: NodeId, class: &str);

    /// Contents of a `<template>` element (points to a fragment root node)
    fn template_contents(&self, id: NodeId) -> Option<NodeId>;
    fn set_template_contents(&mut self, id: NodeId, fragment: NodeId);

    // -----------------------------------------------------------------------
    // Text / comment / doctype data
    // -----------------------------------------------------------------------

    fn text_value(&self, id: NodeId) -> Option<&str>;
    fn set_text_value(&mut self, id: NodeId, value: &str);

    fn comment_value(&self, id: NodeId) -> Option<&str>;

    fn doctype_name(&self, id: NodeId) -> Option<&str>;
    fn doctype_public_id(&self, id: NodeId) -> Option<&str>;
    fn doctype_system_id(&self, id: NodeId) -> Option<&str>;

    // -----------------------------------------------------------------------
    // Document-level metadata
    // -----------------------------------------------------------------------

    fn url(&self) -> Option<Url>;

    fn quirks_mode(&self) -> QuirksMode;
    fn set_quirks_mode(&mut self, mode: QuirksMode);

    fn doctype(&self) -> DocumentType;
    fn set_doctype(&mut self, doctype: DocumentType);

    /// Look up a node by its `id` attribute value
    fn node_by_named_id(&self, name_id: &str) -> Option<NodeId>;

    fn node_count(&self) -> usize;
    fn peek_next_id(&self) -> NodeId;

    // -----------------------------------------------------------------------
    // CSS stylesheets
    // -----------------------------------------------------------------------

    fn stylesheets(&self) -> &[<C::CssSystem as CssSystem>::Stylesheet];
    fn add_stylesheet(&mut self, sheet: <C::CssSystem as CssSystem>::Stylesheet);

    // -----------------------------------------------------------------------
    // Serialisation
    // -----------------------------------------------------------------------

    fn write(&self) -> String;
    fn write_from_node(&self, id: NodeId) -> String;
}
