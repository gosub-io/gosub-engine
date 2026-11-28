use core::fmt::Debug;
use gosub_interface::css3::CssSystem;
use gosub_interface::document::{AttachError, Document, DocumentType, PendingStylesheet};
use gosub_shared::attributes::Attributes;
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fmt::{Display, Formatter};
use std::sync::Arc;
use url::Url;

use crate::document::presentation_hints;
use crate::document::selectedness::{self, Selectedness};
use crate::node::arena::NodeArena;
use crate::node::data::comment::CommentData;
use crate::node::data::doctype::DocTypeData;
use crate::node::data::element::ElementData;
use crate::node::node_impl::{NodeDataTypeInternal, NodeImpl};
use crate::node::HTML_NAMESPACE;
use gosub_interface::config::HasDocument;
use gosub_interface::node::{is_valid_shadow_host_name, NodeType, QuirksMode, ShadowRootInit};
use gosub_shared::byte_stream::Location;
use gosub_shared::node::NodeId;
use gosub_shared::types::ParseError;

/// Up to this many elements sharing an id, the first is found by comparing their positions.
const PAIRWISE_HOLDER_LIMIT: usize = 8;

/// Defines a document
#[derive(Debug)]
pub struct DocumentImpl<C: HasDocument> {
    pub url: Option<Url>,
    pub(crate) arena: NodeArena,
    /// Elements by `id`, per tree: keyed by the tree's root, the document or a shadow root.
    /// Every holder of a duplicate id is included; lookups find the first by walking the tree.
    named_id_elements: HashMap<NodeId, HashMap<Arc<str>, HashSet<NodeId>>>,
    /// Reverse index of `named_id_elements`: the tree and id each node is registered under.
    /// Kept in sync so unregistering a node does not require scanning the whole map. Shares
    /// the id with the key in `named_id_elements`.
    named_ids_by_node: HashMap<NodeId, (NodeId, Arc<str>)>,
    /// Which `<option>`s are selected, kept up to date by every tree and attribute change.
    selectedness: Selectedness,
    pub doctype: DocumentType,
    pub quirks_mode: QuirksMode,
    /// The parser's scripting flag determines whether `noscript` text is serialized literally.
    scripting_enabled: bool,
    pub stylesheets: Vec<<C::CssSystem as CssSystem>::Stylesheet>,
    /// Links whose bytes have not arrived. Filled in after the parse; see
    /// [`Document::add_pending_stylesheet`].
    pending_stylesheets: Vec<PendingStylesheet>,
    /// The errors the HTML parser reported building this document, for whoever wants to show
    /// them; see [`DocumentImpl::set_parse_errors`].
    parse_errors: Vec<ParseError>,
}

impl<C: HasDocument> PartialEq for DocumentImpl<C> {
    /// Compare document content and serialization metadata, including the scripting state.
    /// The id index is left out: it follows the arena.
    fn eq(&self, other: &Self) -> bool {
        self.url == other.url
            && self.arena == other.arena
            && self.doctype == other.doctype
            && self.quirks_mode == other.quirks_mode
            && self.scripting_enabled == other.scripting_enabled
            && self.stylesheets == other.stylesheets
    }
}

// new Document<C> trait impl

impl<C: HasDocument<Document = Self>> Document<C> for DocumentImpl<C> {
    /// Create an empty document with its root registered and scripting enabled by default.
    fn new(document_type: DocumentType, url: Option<Url>) -> Self {
        let mut doc = Self {
            url,
            arena: NodeArena::new(),
            named_id_elements: HashMap::new(),
            named_ids_by_node: HashMap::new(),
            selectedness: Selectedness::default(),
            doctype: document_type,
            quirks_mode: QuirksMode::NoQuirks,
            scripting_enabled: true,
            stylesheets: Vec::new(),
            pending_stylesheets: Vec::new(),
            parse_errors: Vec::new(),
        };
        let root = NodeImpl::new_document(Location::default());
        doc.arena.register_node(root);
        doc
    }

    // node creation

    fn create_element(
        &mut self,
        name: &str,
        namespace: Option<&str>,
        attributes: Attributes,
        location: Location,
    ) -> NodeId {
        let node = NodeImpl::new(
            location,
            NodeDataTypeInternal::Element(ElementData::new(name, namespace, attributes)),
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
        if self.refuses_copy(id) {
            return id;
        }
        let Some(root) = self.copy_node(id) else { return id };
        // A stack rather than recursion: the parser builds trees deep enough to blow the call
        // stack, and a clone has to cope with whatever it built.
        let mut stack = vec![(id, root)];
        while let Some((original, copy)) = stack.pop() {
            for child in self.children(original).to_vec() {
                let Some(child_copy) = self.copy_node(child) else {
                    continue;
                };
                // Linked by hand, not with `attach_node`: the copy is a fresh leaf in a detached
                // tree, so there is no cycle to check and no registry for its id. Asking for one
                // walks every copied ancestor per node, which is quadratic down a deep chain.
                if let Some(parent) = self.arena.node_ref_mut(copy) {
                    parent.push(child_copy);
                }
                if let Some(node) = self.arena.node_ref_mut(child_copy) {
                    node.parent = Some(copy);
                }
                stack.push((child, child_copy));
            }
        }
        // Linked by hand above, so the options of a copied select join it here.
        self.refresh_options(root);
        root
    }

    fn duplicate_node(&mut self, id: NodeId) -> NodeId {
        if self.refuses_copy(id) {
            return id;
        }
        let Some(node) = self.arena.node_ref(id) else { return id };
        let dup = NodeImpl::new_from_node(node);
        self.register_node(dup)
    }

    // tree navigation

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

    fn contains(&self, id: NodeId) -> bool {
        self.arena.node_ref(id).is_some()
    }

    fn attach(&mut self, node: NodeId, parent: NodeId, position: Option<usize>) -> Result<(), AttachError> {
        self.attach_node(node, parent, position)
    }

    fn detach(&mut self, node: NodeId) {
        self.detach_node(node);
    }

    fn remove(&mut self, node: NodeId) {
        self.delete_node_by_id(node);
    }

    fn relocate_node(&mut self, node: NodeId, parent: NodeId) {
        self.relocate_node(node, parent);
    }

    // node type

    fn node_type(&self, id: NodeId) -> NodeType {
        self.arena.node_ref(id).map_or(NodeType::DocumentNode, |n| n.type_of())
    }

    // element data

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
            NodeDataTypeInternal::Element(ref e) => e.attribute(name).map(String::as_str),
            _ => None,
        }
    }

    fn attributes(&self, id: NodeId) -> Option<&Attributes> {
        match self.arena.node_ref(id)?.data {
            NodeDataTypeInternal::Element(ref e) => Some(e.attributes()),
            _ => None,
        }
    }

    fn set_attribute(&mut self, id: NodeId, name: &str, value: &str) {
        let had = self.attribute(id, name).is_some();
        if let Some(node) = self.arena.node_ref_mut(id) {
            if let NodeDataTypeInternal::Element(ref mut e) = node.data {
                e.add_attribute(name, value);
            }
        }
        if name == "id" {
            self.refresh_named_id(id);
        }
        self.selectedness_attribute_changed(id, name, had, true);
    }

    fn remove_attribute(&mut self, id: NodeId, name: &str) {
        let had = self.attribute(id, name).is_some();
        if let Some(node) = self.arena.node_ref_mut(id) {
            if let NodeDataTypeInternal::Element(ref mut e) = node.data {
                e.remove_attribute(name);
            }
        }
        if name == "id" {
            self.refresh_named_id(id);
        }
        self.selectedness_attribute_changed(id, name, had, false);
    }

    fn is_option_selected(&self, id: NodeId) -> bool {
        self.selectedness.is_selected(id)
    }

    fn option_select(&self, option: NodeId) -> Option<NodeId> {
        self.selectedness.select_of(option)
    }

    fn first_selected_option(&self, select: NodeId) -> Option<NodeId> {
        let selected = self.selectedness.selected_in(select);
        if selected.len() <= 1 {
            return selected.first().copied();
        }
        selectedness::list_of_options::<C>(self, select)
            .into_iter()
            .find(|option| selected.contains(option))
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
            NodeDataTypeInternal::Element(e) => e.classlist().contains(name),
            _ => false,
        }
    }

    fn presentational_hints(&self, id: NodeId) -> Option<String> {
        let tag = self.tag_name(id)?;
        // Only the cells need the tree walked, and only up to the table they sit in - so the
        // walk is skipped for every other element, which is nearly all of them.
        let cell = tag.eq_ignore_ascii_case("td") || tag.eq_ignore_ascii_case("th");
        let table = cell.then(|| self.enclosing_table(id)).flatten();
        let svg = self.namespace(id) == Some(crate::node::SVG_NAMESPACE);
        presentation_hints::hints_for(
            tag,
            svg,
            |name| self.attribute(id, name),
            |name| table.and_then(|table| self.attribute(table, name)),
            table.is_some(),
        )
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
        self.refresh_template_children(id);
        self.refresh_options(id);
    }

    // shadow trees

    fn attach_shadow_root(&mut self, host: NodeId, init: ShadowRootInit, location: Location) -> Option<NodeId> {
        // The eligibility half of the spec's "attach a shadow root"; failing it is the
        // NotSupportedError the caller is expected to recover from.
        let host_node = self.arena.node_ref(host)?;
        let NodeDataTypeInternal::Element(ref e) = host_node.data else {
            return None;
        };
        if e.shadow_root.is_some() || !e.is_namespace(HTML_NAMESPACE) || !is_valid_shadow_host_name(e.name()) {
            return None;
        }

        let root = self.register_node(NodeImpl::new_shadow_root(location, host, init));
        if let Some(node) = self.arena.node_ref_mut(host) {
            if let NodeDataTypeInternal::Element(ref mut e) = node.data {
                e.set_shadow_root(root);
            }
        }
        Some(root)
    }

    fn shadow_root(&self, id: NodeId) -> Option<NodeId> {
        match self.arena.node_ref(id)?.data {
            NodeDataTypeInternal::Element(ref e) => e.shadow_root,
            _ => None,
        }
    }

    fn shadow_host(&self, id: NodeId) -> Option<NodeId> {
        Some(self.arena.node_ref(id)?.get_shadow_root_data()?.host)
    }

    fn shadow_root_init(&self, id: NodeId) -> Option<ShadowRootInit> {
        Some(self.arena.node_ref(id)?.get_shadow_root_data()?.init)
    }

    // text / comment / doctype

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

    fn append_text_value(&mut self, id: NodeId, value: &str) -> bool {
        let Some(node) = self.arena.node_ref_mut(id) else {
            return false;
        };
        if let NodeDataTypeInternal::Text(ref mut t) = node.data {
            // In-place append: the backing String grows geometrically, so merging N adjacent
            // text runs into this node stays amortized O(total length) instead of O(N^2).
            t.value.push_str(value);
            true
        } else {
            false
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

    // document metadata

    fn url(&self) -> Option<Url> {
        self.url.clone()
    }

    fn quirks_mode(&self) -> QuirksMode {
        self.quirks_mode
    }

    fn set_quirks_mode(&mut self, mode: QuirksMode) {
        self.quirks_mode = mode;
    }

    /// Report the parser's retained scripting setting used for `noscript` serialization.
    fn scripting_enabled(&self) -> bool {
        self.scripting_enabled
    }

    /// Store the scripting setting so later serialization of `noscript` uses the same mode.
    fn set_scripting_enabled(&mut self, enabled: bool) {
        self.scripting_enabled = enabled;
    }

    fn doctype(&self) -> DocumentType {
        self.doctype
    }

    fn set_doctype(&mut self, doctype: DocumentType) {
        self.doctype = doctype;
    }

    fn node_by_named_id(&self, name_id: &str) -> Option<NodeId> {
        self.first_named_id_holder(NodeId::root(), name_id)
    }

    fn tree_root(&self, id: NodeId) -> NodeId {
        self.tree_root_of(id)
    }

    fn node_by_named_id_in(&self, root: NodeId, name_id: &str) -> Option<NodeId> {
        self.first_named_id_holder(root, name_id)
    }

    fn node_count(&self) -> usize {
        self.arena.node_count()
    }

    fn peek_next_id(&self) -> NodeId {
        self.arena.peek_next_id()
    }

    // stylesheets

    fn stylesheets(&self) -> &[<C::CssSystem as CssSystem>::Stylesheet] {
        &self.stylesheets
    }

    fn add_stylesheet(&mut self, sheet: <C::CssSystem as CssSystem>::Stylesheet) {
        self.stylesheets.push(sheet);
    }

    fn add_pending_stylesheet(&mut self, url: &str, media: Option<&str>) {
        // Where it would have gone had the bytes been here. Inline `<style>` blocks keep
        // landing in `stylesheets` while this one is in flight, so the position is recorded
        // now and the sheet slotted in later rather than appended.
        self.pending_stylesheets.push(PendingStylesheet {
            position: self.stylesheets.len(),
            url: url.to_string(),
            media: media.map(str::to_string),
        });
    }

    fn take_pending_stylesheets(&mut self) -> Vec<PendingStylesheet> {
        std::mem::take(&mut self.pending_stylesheets)
    }

    fn insert_stylesheet(&mut self, position: usize, sheet: <C::CssSystem as CssSystem>::Stylesheet) {
        let position = position.min(self.stylesheets.len());
        self.stylesheets.insert(position, sheet);
    }

    // serialisation

    fn write(&self) -> String {
        self.write_from_node(NodeId::root())
    }

    fn write_from_node(&self, node_id: NodeId) -> String {
        crate::writer::DocumentWriter::write_from_node::<C>(node_id, self)
    }

    /// Serialize a subtree iteratively with XML escaping and balanced tags for inline SVG decoding.
    fn write_xml_from_node(&self, node_id: NodeId) -> String {
        crate::writer::DocumentWriter::write_xml_from_node::<C>(node_id, self)
    }

    fn write_svg_from_node(&self, node_id: NodeId, attrs: &dyn Fn(NodeId) -> Vec<(&'static str, String)>) -> String {
        crate::writer::DocumentWriter::write_svg_from_node::<C>(node_id, self, attrs)
    }
}

// Internal helpers (not part of Document trait)

impl<C: HasDocument<Document = Self>> DocumentImpl<C> {
    /// Keep the errors [`Html5Parser::parse_document`](crate::parser::Html5Parser::parse_document)
    /// returned for this document. The parser hands them back rather than storing them, so a
    /// caller that discards them has none to show.
    pub fn set_parse_errors(&mut self, errors: Vec<ParseError>) {
        self.parse_errors = errors;
    }

    /// The errors kept by [`Self::set_parse_errors`], in the order the parser met them.
    #[must_use]
    pub fn parse_errors(&self) -> &[ParseError] {
        &self.parse_errors
    }

    /// The `<table>` a cell sits in, if it sits in one at all.
    ///
    /// A `<td>` written outside a table is an ordinary element as far as the tree is concerned,
    /// and gets none of the padding a table gives its cells.
    fn enclosing_table(&self, id: NodeId) -> Option<NodeId> {
        let mut current = self.parent(id);
        while let Some(node) = current {
            if self.tag_name(node).is_some_and(|tag| tag.eq_ignore_ascii_case("table")) {
                return Some(node);
            }
            current = self.parent(node);
        }
        None
    }

    /// The root of the tree `node_id` is in: follow parents until there are none. A shadow root
    /// has no parent, so for a node inside a shadow tree that is the shadow root.
    fn tree_root_of(&self, node_id: NodeId) -> NodeId {
        let mut current = node_id;
        while let Some(parent) = self.arena.node_ref(current).and_then(NodeImpl::parent_id) {
            current = parent;
        }
        current
    }

    /// Whether the tree rooted at `root` keeps its ids in the registry: the document tree and
    /// shadow trees do, detached subtrees do not.
    fn has_id_registry(&self, root: NodeId) -> bool {
        root == NodeId::root()
            || self
                .arena
                .node_ref(root)
                .is_some_and(|n| n.get_shadow_root_data().is_some())
    }

    /// Whether the children of `node_id` are template contents. An ordinary `<template>` keeps
    /// its contents as its own children (see the parser), where the spec has a separate
    /// document fragment: those children are in no tree with a registry.
    fn holds_template_contents(&self, node_id: NodeId) -> bool {
        self.arena
            .node_ref(node_id)
            .and_then(NodeImpl::get_element_data)
            .is_some_and(|data| data.template_contents == Some(node_id))
    }

    /// Bring the ids below `node_id` in line after its children became template contents, or
    /// stopped being them.
    fn refresh_template_children(&mut self, node_id: NodeId) {
        let children = self
            .arena
            .node_ref(node_id)
            .map(|n| n.children.clone())
            .unwrap_or_default();
        let registry = self
            .registry_of(node_id)
            .filter(|_| !self.holds_template_contents(node_id));
        for child in children {
            self.refresh_named_ids(child, registry);
        }
    }

    /// The registry `node_id`'s id belongs in: the root of its tree, if that tree has one.
    fn registry_of(&self, node_id: NodeId) -> Option<NodeId> {
        let mut current = node_id;
        while let Some(parent) = self.arena.node_ref(current).and_then(NodeImpl::parent_id) {
            if self.holds_template_contents(parent) {
                return None;
            }
            current = parent;
        }
        self.has_id_registry(current).then_some(current)
    }

    /// Register `node_id` and everything below it in `registry`, or drop them when it is `None`.
    /// Template contents are dropped either way. Shadow trees below it are not walked: they keep
    /// their own registry.
    fn refresh_named_ids(&mut self, node_id: NodeId, registry: Option<NodeId>) {
        let mut stack = vec![(node_id, registry)];
        while let Some((id, registry)) = stack.pop() {
            let below = registry.filter(|_| !self.holds_template_contents(id));
            if let Some(node) = self.arena.node_ref(id) {
                stack.extend(node.children.iter().map(|&child| (child, below)));
            }
            self.register_named_id(id, registry);
        }
    }

    /// Bring the options at and below `node_id` in line after its ancestors changed. An option
    /// that is now in another select's list of options, or in none, runs the selectedness
    /// setting algorithm on both selects: the option insertion and removing steps.
    fn refresh_options(&mut self, node_id: NodeId) {
        let mut stack = vec![node_id];
        while let Some(id) = stack.pop() {
            // Tree order, as the spec runs the steps for an inserted subtree.
            if let Some(node) = self.arena.node_ref(id) {
                stack.extend(node.children.iter().rev());
            }
            if self.selectedness.is_option(id) {
                self.update_nearest_select(id);
            }
        }
    }

    /// Point `option` at its nearest ancestor select; when that changed, run the setting
    /// algorithm on the select it left and the one it joined
    /// (<https://html.spec.whatwg.org/#update-an-option%27s-nearest-ancestor-select>).
    fn update_nearest_select(&mut self, option: NodeId) {
        let new = selectedness::nearest_ancestor_select::<C>(self, option);
        let old = self.selectedness.set_select(option, new);
        if old == new {
            // Moved within its select, maybe out of a disabled optgroup.
            if let Some(select) = new {
                self.selectedness.set_has_no_enabled(select, false);
            }
            return;
        }
        // A selected option that joins a list deselects the one that was.
        if self.selectedness.is_selected(option) {
            self.deselect_others(option);
        }
        if let Some(old) = old {
            self.run_selectedness_setting(old, None);
        }
        if let Some(new) = new {
            self.run_selectedness_setting(new, Some(option));
        }
    }

    /// Set `option`'s selectedness. In a select without `multiple` an option that becomes
    /// selected deselects the others.
    fn set_option_selected(&mut self, option: NodeId, selected: bool) {
        self.selectedness.set_selected(option, selected);
        if selected {
            self.deselect_others(option);
        }
    }

    fn deselect_others(&mut self, option: NodeId) {
        let Some(select) = self.selectedness.select_of(option) else {
            return;
        };
        if self.attribute(select, "multiple").is_some() {
            return;
        }
        for other in self.selectedness.selected_in(select) {
            if other != option {
                self.selectedness.set_selected(other, false);
            }
        }
    }

    /// The selectedness setting algorithm: of the selected options keep the last, and with none
    /// select the first one that is not disabled, in a select that shows one row
    /// (<https://html.spec.whatwg.org/#selectedness-setting-algorithm>). `joined` is the option
    /// that just joined the list, when that is what this runs for.
    fn run_selectedness_setting(&mut self, select: NodeId, joined: Option<NodeId>) {
        // A deleted select's options are leaving it; there is nothing left to choose for.
        if !self.contains(select) || self.attribute(select, "multiple").is_some() {
            return;
        }
        let selected = self.selectedness.selected_in(select);
        match selected.len() {
            // Nearly every change ends here, without a walk over the options.
            1 => {}
            0 => {
                if selectedness::display_size::<C>(self, select) != 1 {
                    return;
                }
                let first_enabled = match joined {
                    // Every other option is disabled, so only the new one can be first.
                    Some(option) if self.selectedness.has_no_enabled(select) => {
                        Some(option).filter(|&o| !selectedness::is_disabled::<C>(self, o))
                    }
                    _ => selectedness::list_of_options::<C>(self, select)
                        .into_iter()
                        .find(|&option| !selectedness::is_disabled::<C>(self, option)),
                };
                self.selectedness.set_has_no_enabled(select, first_enabled.is_none());
                if let Some(option) = first_enabled {
                    self.selectedness.set_selected(option, true);
                }
            }
            _ => {
                let options = selectedness::list_of_options::<C>(self, select);
                let last = options.iter().rev().find(|option| selected.contains(option)).copied();
                for option in selected {
                    if Some(option) != last {
                        self.selectedness.set_selected(option, false);
                    }
                }
            }
        }
    }

    /// The option, optgroup and select attribute change steps
    /// (<https://html.spec.whatwg.org/#the-option-element:html-element-attribute-change-steps>).
    /// `old` and `new` say whether `name` was there before and is now.
    fn selectedness_attribute_changed(&mut self, id: NodeId, name: &str, old: bool, new: bool) {
        if self.selectedness.is_option(id) {
            if !matches!(name, "selected" | "disabled") || old == new {
                return;
            }
            if name == "selected" {
                self.set_option_selected(id, new);
            }
            if let Some(select) = self.selectedness.select_of(id) {
                self.run_selectedness_setting(select, None);
            }
        } else if selectedness::is_html::<C>(self, id, "optgroup") {
            if name != "disabled" || old == new {
                return;
            }
            if let Some(select) = selectedness::nearest_ancestor_select::<C>(self, id) {
                self.run_selectedness_setting(select, None);
            }
        } else if selectedness::is_html::<C>(self, id, "select") {
            let multiple_changed = name == "multiple" && old != new;
            if multiple_changed && !new {
                // Back to one choice: the first selected option keeps it.
                let selected = self.selectedness.selected_in(id);
                let first = selectedness::list_of_options::<C>(self, id)
                    .into_iter()
                    .find(|option| selected.contains(option));
                for option in selected {
                    if Some(option) != first {
                        self.selectedness.set_selected(option, false);
                    }
                }
            }
            if name == "size" || multiple_changed {
                self.run_selectedness_setting(id, None);
            }
        }
    }

    /// Whether `node_id` has an `id` attribute or is registered under one. When neither holds,
    /// refreshing it changes nothing and the ancestor walk can be skipped.
    fn has_named_id_state(&self, node_id: NodeId) -> bool {
        self.named_ids_by_node.contains_key(&node_id)
            || self
                .arena
                .node_ref(node_id)
                .and_then(NodeImpl::get_element_data)
                .is_some_and(|data| data.attributes().contains_key("id"))
    }

    /// Bring `node_id`'s registration in line with its `id` attribute, after that changed.
    fn refresh_named_id(&mut self, node_id: NodeId) {
        if !self.has_named_id_state(node_id) {
            return;
        }
        let registry = self.registry_of(node_id);
        self.register_named_id(node_id, registry);
    }

    /// Register `node_id` under its `id` attribute in `registry`, dropping whatever it was
    /// registered under before. No registry, or an absent or empty value, just unregisters. Any
    /// other value is the element's ID as written, whitespace included
    /// (<https://dom.spec.whatwg.org/#concept-id>).
    fn register_named_id(&mut self, node_id: NodeId, registry: Option<NodeId>) {
        let id_value = registry.and_then(|_| {
            self.arena
                .node_ref(node_id)
                .and_then(NodeImpl::get_element_data)
                .and_then(|data| data.attribute("id"))
                .filter(|v| !v.is_empty())
        });
        let (Some(registry), Some(id_value)) = (registry, id_value) else {
            self.unregister_named_id(node_id);
            return;
        };
        // Registered like this already, as for a subtree moved within its tree.
        if self
            .named_ids_by_node
            .get(&node_id)
            .is_some_and(|(r, v)| *r == registry && **v == **id_value)
        {
            return;
        }
        let id_value: Arc<str> = Arc::from(id_value.as_str());
        self.unregister_named_id(node_id);
        self.named_id_elements
            .entry(registry)
            .or_default()
            .entry(Arc::clone(&id_value))
            .or_default()
            .insert(node_id);
        self.named_ids_by_node.insert(node_id, (registry, id_value));
    }

    fn unregister_named_id(&mut self, node_id: NodeId) {
        let Some((registry, id_value)) = self.named_ids_by_node.remove(&node_id) else {
            return;
        };
        let Some(ids) = self.named_id_elements.get_mut(&registry) else {
            return;
        };
        if let Entry::Occupied(mut e) = ids.entry(id_value) {
            e.get_mut().remove(&node_id);
            if e.get().is_empty() {
                e.remove();
            }
        }
        if ids.is_empty() {
            self.named_id_elements.remove(&registry);
        }
    }

    /// The first element in tree order with `id` `name_id`, in the tree below `root`.
    fn first_named_id_holder(&self, root: NodeId, name_id: &str) -> Option<NodeId> {
        if self.has_id_registry(root) {
            let holders = self.named_id_elements.get(&root)?.get(name_id)?;
            if holders.len() <= PAIRWISE_HOLDER_LIMIT {
                return holders
                    .iter()
                    .copied()
                    .reduce(|first, other| if self.precedes(other, first) { other } else { first });
            }
        } else if name_id.is_empty() {
            return None;
        }
        // Many holders, where comparing pairs would rescan the same siblings, or a tree without a
        // registry: walk it in order and stop at the first holder.
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            let Some(node) = self.arena.node_ref(id) else {
                continue;
            };
            let holds = node
                .get_element_data()
                .and_then(|data| data.attribute("id"))
                .is_some_and(|v| v == name_id);
            if holds {
                return Some(id);
            }
            if !self.holds_template_contents(id) {
                stack.extend(node.children.iter().rev());
            }
        }
        None
    }

    /// Whether `a` comes before `b` in tree order. Both must be in the document tree.
    fn precedes(&self, a: NodeId, b: NodeId) -> bool {
        let path_a = self.path_from_root(a);
        let path_b = self.path_from_root(b);
        let common = path_a.iter().zip(&path_b).take_while(|(x, y)| x == y).count();
        let parent = common.checked_sub(1).map(|i| path_a[i]);
        match (path_a.get(common), path_b.get(common)) {
            // Siblings below the last shared ancestor: the earlier child comes first.
            (Some(&child_a), Some(&child_b)) => {
                parent
                    .and_then(|p| self.arena.node_ref(p))
                    .and_then(|n| n.children.iter().find(|&&c| c == child_a || c == child_b))
                    == Some(&child_a)
            }
            // `a` is an ancestor of `b`.
            (None, Some(_)) => true,
            // The same node, or `b` is an ancestor of `a`.
            _ => false,
        }
    }

    /// `node_id` and its ancestors, the root first.
    fn path_from_root(&self, node_id: NodeId) -> Vec<NodeId> {
        let mut path = vec![node_id];
        let mut current = node_id;
        while let Some(parent) = self.arena.node_ref(current).and_then(NodeImpl::parent_id) {
            path.push(parent);
            current = parent;
        }
        path.reverse();
        path
    }

    /// Fetch a node reference by id (internal - not in the Document trait)
    pub fn node_by_id(&self, node_id: NodeId) -> Option<&NodeImpl> {
        self.arena.node_ref(node_id)
    }

    pub fn node_by_id_mut(&mut self, node_id: NodeId) -> Option<&mut NodeImpl> {
        self.arena.node_ref_mut(node_id)
    }

    /// Returns the root node reference, which every document built through
    /// `Document::new()` has.
    ///
    /// `Option` rather than an `expect`: the `Display` impl prints nothing for a document
    /// without one.
    pub fn get_root(&self) -> Option<&NodeImpl> {
        self.arena.node_ref(NodeId::root())
    }

    /// Register a node (assigns id, marks registered). Does NOT attach to tree.
    pub fn register_node(&mut self, mut node: NodeImpl) -> NodeId {
        let node_id = self.arena.get_next_id();
        node.set_id(node_id);
        // Not in the document tree yet, so its `id` is registered by `attach_node`.
        self.arena.register_node_with_node_id(node, node_id);
        if selectedness::is_html::<C>(self, node_id, "option") {
            let selected = self.attribute(node_id, "selected").is_some();
            self.selectedness.add(node_id, selected);
        }
        node_id
    }

    /// Register a node and attach it to a parent
    pub fn register_node_at(&mut self, node: NodeImpl, parent_id: NodeId, position: Option<usize>) -> NodeId {
        let node_id = self.register_node(node);
        // A new node cannot make a cycle; `attach_node` logs the other refusals.
        let _ = self.attach_node(node_id, parent_id, position);
        node_id
    }

    /// Insert `node_id` under `parent_id`, at `position` or last, moving it from where it was.
    /// On an error the tree is left as it was.
    pub fn attach_node(
        &mut self,
        node_id: NodeId,
        parent_id: NodeId,
        position: Option<usize>,
    ) -> Result<(), AttachError> {
        // Checked before anything moves: a missing node would otherwise be pushed into the
        // parent's children first and only then be found missing.
        if self.arena.node_ref(node_id).is_none() || self.arena.node_ref(parent_id).is_none() {
            log::warn!("attach_node: node {node_id} or parent {parent_id} not found in arena");
            return Err(AttachError::MissingNode);
        }
        if self.would_cycle(node_id, parent_id) {
            return Err(AttachError::Cycle);
        }
        // Checked here and not in the trait methods: this inherent method is public, so a caller
        // with a concrete `DocumentImpl` reaches it directly.
        if self.refuses_tree_mutation("attach_node", node_id) {
            return Err(AttachError::ShadowRoot);
        }
        // Insertion removes the node from where it was, or it ends up in two child lists and
        // the parent walk in `would_cycle` no longer sees every ancestor.
        self.detach_node(node_id);
        if let Some(parent_node) = self.arena.node_ref_mut(parent_id) {
            match position {
                Some(position) if position <= parent_node.children().len() => {
                    parent_node.insert(node_id, position);
                }
                _ => {
                    parent_node.push(node_id);
                }
            }
        }
        let Some(node) = self.arena.node_ref_mut(node_id) else {
            return Err(AttachError::MissingNode);
        };
        node.parent = Some(parent_id);
        let is_leaf = node.children.is_empty();
        self.refresh_options(node_id);
        if is_leaf {
            // Most attaches are the parser adding a leaf: skip the ancestor walk when it has no id.
            self.refresh_named_id(node_id);
            return Ok(());
        }
        let registry = self.registry_of(node_id);
        self.refresh_named_ids(node_id, registry);
        Ok(())
    }

    /// Whether `node_id` is a host-including inclusive ancestor of `parent_id` (DOM "pre-insert
    /// validity"), so putting it under `parent_id` would make a cycle. Crosses from a shadow
    /// root to its host: a host moved into its own shadow tree is a cycle as well.
    ///
    /// Also true when `parent_id` is gone, so callers refuse the move before they detach.
    fn would_cycle(&self, node_id: NodeId, parent_id: NodeId) -> bool {
        if node_id == parent_id || self.arena.node_ref(parent_id).is_none() {
            return true;
        }
        // A leaf without a shadow tree is nobody's ancestor. The parser attaches mostly those.
        let is_leaf = self.arena.node_ref(node_id).is_some_and(|n| n.children.is_empty());
        if is_leaf && self.shadow_root(node_id).is_none() {
            return false;
        }
        let mut current = parent_id;
        loop {
            if current == node_id {
                return true;
            }
            match self.arena.node_ref(current).and_then(NodeImpl::parent_id) {
                Some(parent) => current = parent,
                None => match self.shadow_host(current) {
                    Some(host) => current = host,
                    None => return false,
                },
            }
        }
    }

    /// Whether `node_id` must not be copied, because it is a shadow root.
    ///
    /// A shadow root's data holds the id of its host, so a copy would be a second root for the
    /// same host. `ShadowRoot.cloneNode()` throws in the spec; here the caller gets the original
    /// id back, as on the not-found path.
    fn refuses_copy(&self, node_id: NodeId) -> bool {
        let is_shadow_root = self
            .arena
            .node_ref(node_id)
            .is_some_and(|n| n.type_of() == NodeType::ShadowRootNode);
        if is_shadow_root {
            log::warn!("refusing to copy shadow root {node_id}");
        }
        is_shadow_root
    }

    /// One node of a `clone_node`: registered, unattached, no children yet.
    ///
    /// A template that holds its own contents points at itself, and the copied data still
    /// points at the original - which would send the copy's contents into the original. Any
    /// other target is a declarative shadow root, which belongs to the original alone.
    fn copy_node(&mut self, node_id: NodeId) -> Option<NodeId> {
        let holds_contents = self.holds_template_contents(node_id);
        let copy = NodeImpl::new_from_node(self.arena.node_ref(node_id)?);
        let copy_id = self.register_node(copy);
        if let Some(data) = self
            .arena
            .node_ref_mut(copy_id)
            .and_then(NodeImpl::get_element_data_mut)
        {
            if data.template_contents.is_some() {
                data.template_contents = holds_contents.then_some(copy_id);
            }
        }
        Some(copy_id)
    }

    /// Whether a generic tree mutation must leave `node_id` alone because it is a shadow root.
    ///
    /// A shadow root hangs off its host by a side pointer and never appears in a `children`
    /// list. Attaching it would give it a parent and put it into normal traversal as a child of
    /// the element it shadows.
    fn refuses_tree_mutation(&self, what: &str, node_id: NodeId) -> bool {
        let is_shadow_root = self
            .arena
            .node_ref(node_id)
            .is_some_and(|n| n.type_of() == NodeType::ShadowRootNode);
        if is_shadow_root {
            log::warn!("{what}: refusing to move shadow root {node_id} into the ordinary tree");
        }
        is_shadow_root
    }

    pub fn detach_node(&mut self, node_id: NodeId) {
        let Some(parent) = self.node_by_id(node_id).map(NodeImpl::parent_id) else {
            return;
        };
        if let Some(parent_id) = parent {
            if let Some(parent_node) = self.arena.node_ref_mut(parent_id) {
                parent_node.remove(node_id);
            }

            if let Some(node) = self.arena.node_ref_mut(node_id) {
                node.set_parent(None);
                self.refresh_named_ids(node_id, None);
            }
            self.refresh_options(node_id);
        }
    }

    /// Move `node_id` to the end of `parent_id`'s children, as DOM `appendChild` does - also
    /// when it is already one of them. A move `attach_node` refuses leaves the tree as it was.
    pub fn relocate_node(&mut self, node_id: NodeId, parent_id: NodeId) {
        // `attach_node` detaches the node itself, once its checks have passed.
        let _ = self.attach_node(node_id, parent_id, None);
    }

    /// Delete `node_id` from the arena, and its shadow root if it is a host. Their children stay,
    /// each the root of its own detached tree, and the ids below `node_id` leave the document's
    /// registry.
    pub fn delete_node_by_id(&mut self, node_id: NodeId) {
        // The host and its shadow root each name the other, so deleting either clears the link
        // the other way round. Done here and not in `Document::remove` because this inherent
        // method is public.
        //
        // Deleting the root drops the host's side pointer.
        if let Some(host) = self.shadow_host(node_id) {
            if let Some(host_node) = self.arena.node_ref_mut(host) {
                if let NodeDataTypeInternal::Element(ref mut e) = host_node.data {
                    e.shadow_root = None;
                }
            }
        }
        // Deleting the host takes the root with it. Nothing else can reach a shadow root - it
        // has no parent and appears in no `children` list - so leaving it behind would strand a
        // node whose only remaining reference points at a deleted host.
        if let Some(root) = self.shadow_root(node_id) {
            self.refresh_named_ids(root, None);
            self.orphan_children(root);
            self.arena.delete_node(root);
        }
        let Some(parent) = self.arena.node_ref(node_id).map(NodeImpl::parent_id) else {
            return;
        };
        if let Some(parent_id) = parent {
            if let Some(parent_node) = self.arena.node_ref_mut(parent_id) {
                parent_node.remove(node_id);
            }
        }
        // The node and everything below it leave the document tree, so `getElementById` must
        // stop finding them. The arena only drops this one slot.
        self.refresh_named_ids(node_id, None);
        let children = self.children(node_id).to_vec();
        self.orphan_children(node_id);
        self.arena.delete_node(node_id);
        if let Some(select) = self.selectedness.remove(node_id) {
            self.run_selectedness_setting(select, None);
        }
        self.selectedness.set_has_no_enabled(node_id, false);
        for child in children {
            self.refresh_options(child);
        }
    }

    /// Clear the parent link of `node_id`'s children before its slot goes. The arena keeps them,
    /// and a parent link to a deleted slot would make that slot their tree root.
    fn orphan_children(&mut self, node_id: NodeId) {
        let children = self
            .arena
            .node_ref(node_id)
            .map(|n| n.children.clone())
            .unwrap_or_default();
        for child in children {
            if let Some(node) = self.arena.node_ref_mut(child) {
                node.set_parent(None);
            }
        }
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

    /// Iterate over all registered nodes with their ids.
    pub fn nodes(&self) -> impl Iterator<Item = (NodeId, &NodeImpl)> {
        self.arena.nodes()
    }

    // display helper

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
                for (key, value) in element.attributes() {
                    let _ = write!(f, " {key}={value}");
                }
                let _ = writeln!(f, ">");
            }
            NodeDataTypeInternal::ShadowRoot(data) => {
                let _ = writeln!(f, "{buffer}#shadow-root ({})", data.init.mode.as_attribute());
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

        // A shadow root is not among its host's children, so splice it in front of them.
        let shadow_root = match &node.data {
            NodeDataTypeInternal::Element(e) => e.shadow_root,
            _ => None,
        };
        let subtrees: Vec<NodeId> = shadow_root.into_iter().chain(node.children.iter().copied()).collect();

        let len = subtrees.len();
        for (i, child_id) in subtrees.iter().enumerate() {
            let Some(child_node) = self.node_by_id(*child_id) else {
                continue;
            };
            self.print_tree(child_node, buffer.clone(), i == len - 1, f);
        }
    }
}

impl<C: HasDocument<Document = Self>> Display for DocumentImpl<C> {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        if let Some(root) = self.get_root() {
            self.print_tree(root, String::new(), true, f);
        }
        Ok(())
    }
}

// Tree iterator

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::builder::DocumentBuilderImpl;
    use crate::node::HTML_NAMESPACE;
    use crate::parser::Html5Parser;
    use gosub_css3::system::Css3System;
    use gosub_interface::config::ModuleConfiguration;
    use gosub_interface::document::OpenSelect;
    use gosub_interface::interaction::{InteractionSnapshot, InteractionState};
    use gosub_interface::node::ShadowRootMode;

    #[derive(Clone, Debug, PartialEq)]
    struct Config;

    impl ModuleConfiguration for Config {
        type CssSystem = Css3System;
        type Document = DocumentImpl<Self>;
        type HtmlParser = Html5Parser<'static, Self>;
    }

    fn element(doc: &mut DocumentImpl<Config>, name: &str, parent: NodeId) -> NodeId {
        element_with_id(doc, name, parent, None)
    }

    fn element_with_id(doc: &mut DocumentImpl<Config>, name: &str, parent: NodeId, id: Option<&str>) -> NodeId {
        let node = detached_element(doc, name, id);
        doc.attach_node(node, parent, None).unwrap();
        node
    }

    fn detached_element(doc: &mut DocumentImpl<Config>, name: &str, id: Option<&str>) -> NodeId {
        let mut attributes = Attributes::new();
        if let Some(id) = id {
            attributes.insert("id".to_string(), id.to_string());
        }
        doc.create_element(name, Some(HTML_NAMESPACE), attributes, Location::default())
    }

    /// `state` read against `doc` as a style pass reads it.
    fn snapshot(doc: &DocumentImpl<Config>, state: &InteractionState) -> InteractionSnapshot {
        InteractionSnapshot::new::<Config>(state, doc)
    }

    #[test]
    fn deleting_a_node_drops_its_interaction_state() {
        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let root = doc.root();
        let select = element(&mut doc, "select", root);
        let option = element(&mut doc, "option", select);
        let input = element(&mut doc, "input", root);

        let mut state = InteractionState {
            focus: Some(input),
            focus_visible: true,
            open_select: Some(OpenSelect {
                select,
                hover: None,
                active: None,
                first_row: 0,
                viewport: (0.0, 100.0),
            }),
            ..Default::default()
        };
        state.set_checked(input, Some(true));
        state.set_size(input, Some((10.0, 20.0)));
        state.set_selected(select, Some(option));

        doc.delete_node_by_id(input);
        let ix = snapshot(&doc, &state);
        assert_eq!(ix.focused_node(), None, "focus does not survive its node");
        assert!(!ix.is_checked::<Config>(&doc, input));
        assert_eq!(ix.control_size(input), None);

        // Deleting the *option* clears the select's choice, which points at it.
        doc.delete_node_by_id(option);
        assert_eq!(snapshot(&doc, &state).selected_option::<Config>(&doc, select), None);

        doc.delete_node_by_id(select);
        assert!(
            snapshot(&doc, &state).open_select().is_none(),
            "the open dropdown closes with its select"
        );
    }

    #[test]
    fn deleting_the_picked_option_falls_back_to_the_default() {
        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let (select, a, b, state) = picked_option(&mut doc);

        doc.delete_node_by_id(b);

        let ix = snapshot(&doc, &state);
        assert_eq!(ix.selected_option::<Config>(&doc, select), Some(a));
        assert!(ix.is_checked::<Config>(&doc, a));
    }

    /// Detach and delete take the node out of its parent's child list and leave the order of
    /// the other children alone, for the first, a middle and the last child.
    #[test]
    fn detach_and_delete_keep_the_sibling_order() {
        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let root = doc.root();
        let parent = element(&mut doc, "div", root);
        let kids: Vec<NodeId> = (0..5).map(|_| element(&mut doc, "p", parent)).collect();

        doc.detach_node(kids[4]);
        assert_eq!(doc.children(parent), &kids[..4]);
        assert_eq!(doc.parent(kids[4]), None);
        doc.detach_node(kids[0]);
        assert_eq!(doc.children(parent), &kids[1..4]);
        doc.delete_node_by_id(kids[2]);
        assert_eq!(doc.children(parent), &[kids[1], kids[3]]);

        // Detaching a detached node changes nothing; reattaching puts it back once.
        doc.detach_node(kids[4]);
        doc.attach_node(kids[4], parent, Some(1)).unwrap();
        assert_eq!(doc.children(parent), &[kids[1], kids[4], kids[3]]);

        // A move within the same parent leaves a single entry.
        doc.attach_node(kids[1], parent, None).unwrap();
        assert_eq!(doc.children(parent), &[kids[4], kids[3], kids[1]]);
        doc.attach_node(kids[1], parent, Some(0)).unwrap();
        assert_eq!(doc.children(parent), &[kids[1], kids[4], kids[3]]);
    }

    /// html > (old > input, new): the input focused.
    fn focused_input(doc: &mut DocumentImpl<Config>) -> (NodeId, NodeId, NodeId, InteractionState) {
        let root = doc.root();
        let old = element(doc, "div", root);
        let new = element(doc, "section", root);
        let input = element(doc, "input", old);
        let state = InteractionState {
            focus: Some(input),
            focus_visible: true,
            ..Default::default()
        };
        (old, new, input, state)
    }

    #[test]
    fn moving_the_focused_element_moves_focus_within() {
        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let (old, new, input, state) = focused_input(&mut doc);
        assert!(snapshot(&doc, &state).is_focus_within(old));

        doc.attach_node(input, new, None).unwrap();

        let ix = snapshot(&doc, &state);
        assert!(!ix.is_focus_within(old), "the old parent no longer holds the focus");
        assert!(ix.is_focus_within(new));
        assert!(ix.is_focus_within(input));
        assert!(ix.is_focus_within(doc.root()));
        assert_eq!(ix.focused_node(), Some(input), "the focus stays on the element");
    }

    #[test]
    fn moving_an_ancestor_of_the_focused_element_moves_focus_within() {
        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let (old, new, input, state) = focused_input(&mut doc);
        let wrapper = element(&mut doc, "div", old);
        doc.attach_node(input, wrapper, None).unwrap();

        doc.attach_node(wrapper, new, Some(0)).unwrap();

        let ix = snapshot(&doc, &state);
        assert!(!ix.is_focus_within(old));
        assert!(ix.is_focus_within(new));
        assert!(ix.is_focus_within(wrapper));
    }

    #[test]
    fn relocating_the_focused_element_moves_focus_within() {
        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let (old, new, input, state) = focused_input(&mut doc);

        Document::relocate_node(&mut doc, input, new);

        let ix = snapshot(&doc, &state);
        assert!(!ix.is_focus_within(old));
        assert!(ix.is_focus_within(new));
    }

    #[test]
    fn detaching_the_focused_element_takes_focus_within_out_of_the_document() {
        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let (old, _, input, state) = focused_input(&mut doc);

        doc.detach_node(input);

        let ix = snapshot(&doc, &state);
        assert!(!ix.is_focus_within(old));
        assert!(!ix.is_focus_within(doc.root()));
        assert!(ix.is_focus_within(input));
    }

    #[test]
    fn moving_an_unrelated_node_leaves_focus_within_alone() {
        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let (old, new, input, state) = focused_input(&mut doc);
        let other = element(&mut doc, "p", new);

        doc.attach_node(other, old, None).unwrap();

        let ix = snapshot(&doc, &state);
        assert!(ix.is_focus_within(old));
        assert!(ix.is_focus_within(input));
        assert!(!ix.is_focus_within(new));
        assert!(!ix.is_focus_within(other));
    }

    #[test]
    fn moving_an_ancestor_of_the_hovered_node_moves_hover() {
        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let root = doc.root();
        let old = element(&mut doc, "div", root);
        let new = element(&mut doc, "section", root);
        let wrapper = element(&mut doc, "div", old);
        let leaf = element(&mut doc, "span", wrapper);
        let state = InteractionState {
            hover_leaf: Some(leaf),
            ..Default::default()
        };
        assert!(snapshot(&doc, &state).is_hovered(old));

        doc.attach_node(wrapper, new, None).unwrap();

        let ix = snapshot(&doc, &state);
        assert!(!ix.is_hovered(old), "the old parent is no longer under the pointer");
        assert!(ix.is_hovered(new));
        assert!(ix.is_hovered(wrapper));
        assert!(ix.is_hovered(leaf));
        assert!(ix.is_hovered(root));
    }

    /// html > select > (a, b, c), with `b` picked.
    fn picked_option(doc: &mut DocumentImpl<Config>) -> (NodeId, NodeId, NodeId, InteractionState) {
        let root = doc.root();
        let select = element(doc, "select", root);
        let a = element(doc, "option", select);
        let b = element(doc, "option", select);
        element(doc, "option", select);
        let mut state = InteractionState::default();
        state.set_selected(select, Some(b));
        (select, a, b, state)
    }

    #[test]
    fn a_picked_option_detached_from_its_select_is_no_longer_its_choice() {
        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let (select, a, b, state) = picked_option(&mut doc);

        doc.detach_node(b);

        let ix = snapshot(&doc, &state);
        assert_eq!(
            ix.selected_option::<Config>(&doc, select),
            Some(a),
            "back to the first option"
        );
        assert!(ix.is_checked::<Config>(&doc, a));
        assert!(!ix.is_checked::<Config>(&doc, b));
    }

    #[test]
    fn a_picked_option_moved_to_another_select_is_no_longer_the_first_ones_choice() {
        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let (select, a, b, state) = picked_option(&mut doc);
        let root = doc.root();
        let other = element(&mut doc, "select", root);
        let first = element(&mut doc, "option", other);

        doc.attach_node(b, other, None).unwrap();

        let ix = snapshot(&doc, &state);
        assert_eq!(ix.selected_option::<Config>(&doc, select), Some(a));
        assert_eq!(ix.selected_option::<Config>(&doc, other), Some(first));
        assert!(!ix.is_checked::<Config>(&doc, b));
    }

    #[test]
    fn a_picked_option_moved_within_its_select_stays_picked() {
        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let (select, _, b, state) = picked_option(&mut doc);
        let group = element(&mut doc, "optgroup", select);

        doc.attach_node(b, select, None).unwrap();
        let ix = snapshot(&doc, &state);
        assert_eq!(ix.selected_option::<Config>(&doc, select), Some(b), "moved to the end");

        doc.attach_node(b, group, None).unwrap();
        let ix = snapshot(&doc, &state);
        assert_eq!(
            ix.selected_option::<Config>(&doc, select),
            Some(b),
            "moved into an optgroup"
        );
        assert!(ix.is_checked::<Config>(&doc, b));
    }

    #[test]
    fn a_picked_option_moved_into_a_wrapper_in_its_select_stays_picked() {
        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let (select, _, b, state) = picked_option(&mut doc);
        let wrapper = element(&mut doc, "div", select);

        doc.attach_node(b, wrapper, None).unwrap();

        assert_eq!(snapshot(&doc, &state).selected_option::<Config>(&doc, select), Some(b));
    }

    #[test]
    fn a_picked_option_moved_where_it_stops_being_an_option_of_its_select_is_dropped() {
        // A datalist, hr or option in between, or a second optgroup, ends the select's options.
        let tags: [&[&str]; 4] = [&["datalist"], &["hr"], &["option"], &["optgroup", "optgroup"]];
        for path in tags {
            let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
            let (select, a, b, state) = picked_option(&mut doc);
            let mut parent = select;
            for tag in path {
                parent = element(&mut doc, tag, parent);
            }

            doc.attach_node(b, parent, None).unwrap();

            assert_eq!(
                snapshot(&doc, &state).selected_option::<Config>(&doc, select),
                Some(a),
                "{path:?}"
            );
        }
    }

    #[test]
    fn a_picked_option_moved_out_and_back_is_picked_again() {
        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let (select, a, b, state) = picked_option(&mut doc);
        let root = doc.root();
        let elsewhere = element(&mut doc, "div", root);

        doc.attach_node(b, elsewhere, None).unwrap();
        assert_eq!(snapshot(&doc, &state).selected_option::<Config>(&doc, select), Some(a));

        doc.attach_node(b, select, None).unwrap();
        assert_eq!(snapshot(&doc, &state).selected_option::<Config>(&doc, select), Some(b));
    }

    /// The id of `select`'s default option in `body`, with nothing picked.
    fn default_option(body: &str) -> Option<String> {
        let doc = crate::html_compile::<Config>(&format!("<!DOCTYPE html><body><select id=\"s\">{body}</select>"));
        let select = doc.node_by_named_id("s").unwrap();
        let ix = snapshot(&doc, &InteractionState::default());
        ix.selected_option::<Config>(&doc, select)
            .map(|o| doc.attribute(o, "id").unwrap_or_default().to_string())
    }

    #[test]
    fn the_last_selected_option_is_the_default() {
        let cases = [
            ("<option id=a selected><option id=b selected>", "b"),
            (
                "<option id=a selected><option id=b selected><option id=c selected>",
                "c",
            ),
            (
                "<option id=a selected><option id=b><option id=c selected><option id=d>",
                "c",
            ),
            (
                "<option id=a selected><optgroup><option id=b selected></optgroup><option id=c>",
                "b",
            ),
            ("<optgroup><option id=a selected></optgroup><option id=b selected>", "b"),
            // Selectedness does not care about disabled.
            ("<option id=a><option id=b disabled selected>", "b"),
            ("<option id=a><optgroup disabled><option id=b selected></optgroup>", "b"),
        ];
        for (body, want) in cases {
            assert_eq!(default_option(body).as_deref(), Some(want), "{body}");
        }
    }

    #[test]
    fn without_a_selected_option_the_first_enabled_one_is_the_default() {
        let cases = [
            ("<option id=a><option id=b>", Some("a")),
            ("<option id=a disabled><option id=b>", Some("b")),
            ("<optgroup disabled><option id=a></optgroup><option id=b>", Some("b")),
            (
                "<optgroup disabled><option id=a></optgroup><optgroup><option id=b></optgroup>",
                Some("b"),
            ),
            ("<optgroup><option id=a disabled><option id=b></optgroup>", Some("b")),
            (
                "<option id=a disabled><optgroup disabled><option id=b></optgroup>",
                None,
            ),
            ("", None),
        ];
        for (body, want) in cases {
            assert_eq!(default_option(body).as_deref(), want, "{body}");
        }
    }

    #[test]
    fn an_svg_or_mathml_option_is_not_an_option() {
        for namespace in [crate::node::SVG_NAMESPACE, crate::node::MATHML_NAMESPACE] {
            let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
            let root = doc.root();
            let select = element(&mut doc, "select", root);
            let mut selected = Attributes::new();
            selected.insert("selected".to_string(), String::new());
            let foreign = doc.create_element("option", Some(namespace), selected, Location::default());
            doc.attach_node(foreign, select, None).unwrap();
            let option = element(&mut doc, "option", select);

            let ix = snapshot(&doc, &InteractionState::default());
            assert_eq!(ix.selected_option::<Config>(&doc, select), Some(option), "{namespace}");
            assert!(!ix.is_checked::<Config>(&doc, foreign), "{namespace}");
        }
    }

    #[test]
    fn an_option_in_a_nested_optgroup_is_not_an_option() {
        // The parser never nests optgroups; the DOM can.
        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let root = doc.root();
        let select = element(&mut doc, "select", root);
        let a = element(&mut doc, "option", select);
        let outer = element(&mut doc, "optgroup", select);
        let inner = element(&mut doc, "optgroup", outer);
        let b = element(&mut doc, "option", inner);
        doc.set_attribute(a, "selected", "");
        doc.set_attribute(b, "selected", "");

        let ix = snapshot(&doc, &InteractionState::default());
        assert_eq!(ix.selected_option::<Config>(&doc, select), Some(a));
        assert_eq!(doc.option_select(b), None);

        // `b` is not there to fall back to.
        doc.delete_node_by_id(a);
        let ix = snapshot(&doc, &InteractionState::default());
        assert_eq!(ix.selected_option::<Config>(&doc, select), None);
    }

    #[test]
    fn disabling_the_default_option_keeps_it_the_default() {
        let mut doc =
            crate::html_compile::<Config>("<!DOCTYPE html><body><select id=s><option id=a><option id=b></select>");
        let [select, a, b] = ["s", "a", "b"].map(|id| doc.node_by_named_id(id).unwrap());
        doc.set_attribute(a, "disabled", "");

        let ix = snapshot(&doc, &InteractionState::default());
        assert_eq!(ix.selected_option::<Config>(&doc, select), Some(a));
        assert!(ix.is_checked::<Config>(&doc, a));
        assert!(!ix.is_checked::<Config>(&doc, b));
    }

    #[test]
    fn a_pick_outlives_a_change_to_the_default() {
        let mut doc = crate::html_compile::<Config>(
            "<!DOCTYPE html><body><select id=s><option id=a><option id=b><option id=c></select>",
        );
        let [select, a, b, c] = ["s", "a", "b", "c"].map(|id| doc.node_by_named_id(id).unwrap());
        let mut state = InteractionState::default();
        state.set_selected(select, Some(b));
        doc.set_attribute(a, "disabled", "");
        assert_eq!(snapshot(&doc, &state).selected_option::<Config>(&doc, select), Some(b));

        // Without the pick the document's choice shows again: still `a`, disabled or not.
        doc.delete_node_by_id(b);
        let ix = snapshot(&doc, &state);
        assert_eq!(ix.selected_option::<Config>(&doc, select), Some(a));
        assert!(!ix.is_checked::<Config>(&doc, c));
    }

    #[test]
    fn an_option_outside_a_select_is_checked_when_selected() {
        let doc = crate::html_compile::<Config>(
            "<!DOCTYPE html><body><datalist><option id=a selected><option id=b></datalist>",
        );
        let ix = snapshot(&doc, &InteractionState::default());
        assert!(ix.is_checked::<Config>(&doc, doc.node_by_named_id("a").unwrap()));
        assert!(!ix.is_checked::<Config>(&doc, doc.node_by_named_id("b").unwrap()));
    }

    #[test]
    fn a_picked_option_wins_over_the_selected_attribute() {
        let mut doc = crate::html_compile::<Config>(
            "<!DOCTYPE html><body><select id=s><option id=a disabled><option id=b selected><option id=c selected></select>",
        );
        let (select, a) = (doc.node_by_named_id("s").unwrap(), doc.node_by_named_id("a").unwrap());
        let mut state = InteractionState::default();
        state.set_selected(select, Some(a));
        assert_eq!(snapshot(&doc, &state).selected_option::<Config>(&doc, select), Some(a));

        // Dropping the pick goes back to the last selected option.
        doc.delete_node_by_id(a);
        let c = doc.node_by_named_id("c").unwrap();
        assert_eq!(snapshot(&doc, &state).selected_option::<Config>(&doc, select), Some(c));
    }

    /// Change `node`'s `id` attribute the way a DOM `setAttribute` / `removeAttribute` does.
    fn set_id(doc: &mut DocumentImpl<Config>, node: NodeId, id: Option<&str>) {
        match id {
            Some(id) => doc.set_attribute(node, "id", id),
            None => doc.remove_attribute(node, "id"),
        }
    }

    #[test]
    fn renaming_an_id_releases_the_old_one() {
        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let root = doc.root();
        let a = element_with_id(&mut doc, "div", root, Some("first"));
        assert_eq!(doc.node_by_named_id("first"), Some(a));

        set_id(&mut doc, a, Some("second"));
        assert_eq!(doc.node_by_named_id("second"), Some(a));
        assert_eq!(doc.node_by_named_id("first"), None, "the old id stops resolving");

        // ...and another node can now claim the name the first one gave up.
        let b = element_with_id(&mut doc, "div", root, Some("first"));
        assert_eq!(doc.node_by_named_id("first"), Some(b));
    }

    #[test]
    fn an_empty_or_removed_id_unregisters_the_node() {
        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let root = doc.root();

        let a = element_with_id(&mut doc, "div", root, Some("keep"));
        set_id(&mut doc, a, Some(""));
        assert_eq!(doc.node_by_named_id("keep"), None);
        assert_eq!(doc.node_by_named_id(""), None);

        let b = element_with_id(&mut doc, "div", root, Some("gone"));
        set_id(&mut doc, b, None);
        assert_eq!(doc.node_by_named_id("gone"), None);
    }

    #[test]
    fn an_id_with_whitespace_is_found_as_written() {
        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let root = doc.root();

        let a = element_with_id(&mut doc, "div", root, Some("has space"));
        assert_eq!(doc.node_by_named_id("has space"), Some(a));
        assert_eq!(doc.node_by_named_id("has"), None);

        let b = element_with_id(&mut doc, "div", root, Some(" padded "));
        assert_eq!(doc.node_by_named_id(" padded "), Some(b));
        assert_eq!(doc.node_by_named_id("padded"), None, "no trimming");
    }

    /// A `<template>` the way the parser builds one: it holds its own contents.
    fn template(doc: &mut DocumentImpl<Config>, parent: NodeId) -> NodeId {
        let node = detached_element(doc, "template", None);
        doc.set_template_contents(node, node);
        doc.attach_node(node, parent, None).unwrap();
        node
    }

    #[test]
    fn setting_template_contents_on_an_attached_element_drops_its_children_ids() {
        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let root = doc.root();
        let tpl = element_with_id(&mut doc, "template", root, Some("tpl"));
        let child = element_with_id(&mut doc, "div", tpl, Some("inside"));
        element_with_id(&mut doc, "span", child, Some("deeper"));
        assert_eq!(doc.node_by_named_id("inside"), Some(child));

        doc.set_template_contents(tpl, tpl);
        assert_eq!(doc.node_by_named_id("inside"), None);
        assert_eq!(doc.node_by_named_id("deeper"), None);
        assert_eq!(doc.node_by_named_id("tpl"), Some(tpl), "the template keeps its own id");

        // Pointing the contents elsewhere makes the children ordinary again.
        let fragment = detached_element(&mut doc, "div", None);
        doc.set_template_contents(tpl, fragment);
        assert_eq!(doc.node_by_named_id("inside"), Some(child));
        assert!(doc.node_by_named_id("deeper").is_some());
    }

    #[test]
    fn moving_a_node_out_of_template_contents_makes_its_id_findable() {
        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let root = doc.root();
        let tpl = template(&mut doc, root);
        let span = element_with_id(&mut doc, "span", tpl, Some("moved"));
        assert_eq!(doc.node_by_named_id("moved"), None);

        doc.relocate_node(span, root);
        assert_eq!(doc.node_by_named_id("moved"), Some(span));

        doc.relocate_node(span, tpl);
        assert_eq!(doc.node_by_named_id("moved"), None, "and back into the template");
    }

    #[test]
    fn attaching_a_subtree_leaves_its_template_contents_out() {
        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let root = doc.root();
        let wrapper = detached_element(&mut doc, "div", Some("wrapper"));
        let tpl = template(&mut doc, wrapper);
        element_with_id(&mut doc, "span", tpl, Some("inside"));

        doc.attach_node(wrapper, root, None).unwrap();
        assert_eq!(doc.node_by_named_id("wrapper"), Some(wrapper));
        assert_eq!(doc.node_by_named_id("inside"), None);
    }

    #[test]
    fn nothing_below_a_deleted_root_gets_an_id() {
        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let root = doc.root();
        let child = element(&mut doc, "div", root);

        doc.delete_node_by_id(root);
        set_id(&mut doc, child, Some("new"));
        assert_eq!(doc.node_by_named_id("new"), None);
    }

    #[test]
    fn setting_an_id_again_keeps_documents_equal() {
        let build = || {
            let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
            let root = doc.root();
            let a = element_with_id(&mut doc, "div", root, Some("shared"));
            element_with_id(&mut doc, "div", root, Some("shared"));
            (doc, a)
        };
        let (mut one, a) = build();
        let (other, _) = build();

        set_id(&mut one, a, Some("shared"));
        assert_eq!(one.node_by_named_id("shared"), Some(a));
        assert_eq!(one, other);
    }

    #[test]
    fn deleting_a_node_unregisters_its_named_id() {
        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let root = doc.root();
        let div = element_with_id(&mut doc, "div", root, Some("gone"));
        assert_eq!(doc.node_by_named_id("gone"), Some(div));

        doc.delete_node_by_id(div);
        assert_eq!(
            doc.node_by_named_id("gone"),
            None,
            "a deleted node is not resolvable by id"
        );

        // The id is free again for a replacement node.
        let replacement = element_with_id(&mut doc, "div", root, Some("gone"));
        assert_eq!(doc.node_by_named_id("gone"), Some(replacement));
    }

    #[test]
    fn child_changes_leave_the_parent_id_alone() {
        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let root = doc.root();
        let parent = element_with_id(&mut doc, "div", root, Some("parent"));

        let a = element_with_id(&mut doc, "span", parent, None);
        let b = element_with_id(&mut doc, "span", parent, Some("child"));
        doc.detach_node(a);
        doc.delete_node_by_id(b);

        assert_eq!(doc.node_by_named_id("parent"), Some(parent));
        assert_eq!(doc.node_by_named_id("child"), None);
    }

    #[test]
    fn set_and_remove_attribute_keep_the_named_ids_current() {
        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let root = doc.root();
        let div = element_with_id(&mut doc, "div", root, Some("old"));

        doc.set_attribute(div, "id", "new");
        assert_eq!(doc.node_by_named_id("old"), None, "a renamed id stops resolving");
        assert_eq!(doc.node_by_named_id("new"), Some(div));

        doc.remove_attribute(div, "id");
        assert_eq!(doc.node_by_named_id("new"), None, "a removed id stops resolving");

        // A child change does not bring a stale id back, and the name is free again.
        element_with_id(&mut doc, "span", div, None);
        assert_eq!(doc.node_by_named_id("new"), None);
        let other = element_with_id(&mut doc, "div", root, Some("old"));
        assert_eq!(doc.node_by_named_id("old"), Some(other));
    }

    #[test]
    fn an_element_outside_the_document_is_not_found_by_id() {
        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let root = doc.root();
        let div = detached_element(&mut doc, "div", Some("x"));
        assert_eq!(doc.node_by_named_id("x"), None);

        doc.set_attribute(div, "id", "y");
        assert_eq!(doc.node_by_named_id("y"), None);

        doc.attach_node(div, root, None).unwrap();
        assert_eq!(doc.node_by_named_id("y"), Some(div));
    }

    #[test]
    fn detaching_a_subtree_drops_the_ids_below_it() {
        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let root = doc.root();
        let parent = element_with_id(&mut doc, "div", root, Some("parent"));
        let child = element_with_id(&mut doc, "span", parent, Some("child"));

        doc.detach_node(parent);
        assert_eq!(doc.node_by_named_id("parent"), None);
        assert_eq!(doc.node_by_named_id("child"), None);

        doc.attach_node(parent, root, None).unwrap();
        assert_eq!(doc.node_by_named_id("parent"), Some(parent));
        assert_eq!(doc.node_by_named_id("child"), Some(child));
    }

    #[test]
    fn deleting_a_node_drops_the_ids_below_it() {
        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let root = doc.root();
        let parent = element_with_id(&mut doc, "div", root, None);
        element_with_id(&mut doc, "span", parent, Some("child"));

        doc.delete_node_by_id(parent);
        assert_eq!(doc.node_by_named_id("child"), None);
    }

    #[test]
    fn removing_a_node_drops_the_ids_of_its_whole_subtree() {
        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let root = doc.root();
        let div = element_with_id(&mut doc, "div", root, Some("outer"));
        let p = element(&mut doc, "p", div);
        let span = element_with_id(&mut doc, "span", p, Some("inner"));

        Document::remove(&mut doc, div);
        assert_eq!(doc.node_by_named_id("outer"), None);
        assert_eq!(doc.node_by_named_id("inner"), None);
        assert!(doc.node_by_id(div).is_none());

        // The rest of the subtree is kept, as a detached tree rooted at `p`.
        assert_eq!(doc.parent(p), None);
        assert_eq!(doc.tree_root(span), p);
        assert_eq!(doc.node_by_named_id_in(p, "inner"), Some(span));
    }

    #[test]
    fn ids_inside_a_shadow_tree_are_not_found_from_the_document() {
        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let root = doc.root();
        let host = element_with_id(&mut doc, "div", root, None);
        let shadow = doc
            .attach_shadow_root(host, ShadowRootInit::new(ShadowRootMode::Open), Location::default())
            .expect("div can host a shadow root");
        element_with_id(&mut doc, "span", shadow, Some("inside"));
        assert_eq!(doc.node_by_named_id("inside"), None);
    }

    #[test]
    fn a_duplicate_id_falls_back_to_the_next_holder() {
        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let root = doc.root();
        let a = element_with_id(&mut doc, "div", root, Some("shared"));
        let b = element_with_id(&mut doc, "div", root, Some("shared"));
        assert_eq!(doc.node_by_named_id("shared"), Some(a));

        doc.remove_attribute(a, "id");
        assert_eq!(doc.node_by_named_id("shared"), Some(b), "after removing a's id");

        doc.set_attribute(a, "id", "shared");
        assert_eq!(doc.node_by_named_id("shared"), Some(a), "a comes first again");

        doc.set_attribute(a, "id", "other");
        assert_eq!(doc.node_by_named_id("shared"), Some(b), "after renaming a");

        doc.set_attribute(a, "id", "shared");
        doc.delete_node_by_id(a);
        assert_eq!(doc.node_by_named_id("shared"), Some(b), "after deleting a");
    }

    #[test]
    fn a_duplicate_id_resolves_in_tree_order() {
        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let root = doc.root();

        // Registered second, but inserted before the first holder.
        let later = element_with_id(&mut doc, "div", root, Some("dup"));
        let earlier = detached_element(&mut doc, "div", Some("dup"));
        doc.attach_node(earlier, root, Some(0)).unwrap();
        assert_eq!(doc.node_by_named_id("dup"), Some(earlier));
        assert_ne!(later, earlier);

        // An ancestor comes before its descendants.
        let outer = element_with_id(&mut doc, "div", root, None);
        element_with_id(&mut doc, "span", outer, Some("nest"));
        doc.set_attribute(outer, "id", "nest");
        assert_eq!(doc.node_by_named_id("nest"), Some(outer));
    }

    #[test]
    fn a_duplicate_id_resolves_by_position_at_any_depth() {
        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let root = doc.root();
        let first_branch = element_with_id(&mut doc, "div", root, None);
        let second_branch = element_with_id(&mut doc, "div", root, None);
        let shallow = element_with_id(&mut doc, "p", second_branch, Some("dup"));
        let mid = element_with_id(&mut doc, "div", first_branch, None);
        let deep = element_with_id(&mut doc, "span", mid, Some("dup"));
        assert_eq!(
            doc.node_by_named_id("dup"),
            Some(deep),
            "an earlier branch wins over depth"
        );

        doc.detach_node(second_branch);
        doc.attach_node(second_branch, root, Some(0)).unwrap();
        assert_eq!(
            doc.node_by_named_id("dup"),
            Some(shallow),
            "after moving its branch first"
        );
    }

    #[test]
    fn many_holders_of_one_id_resolve_to_the_first() {
        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let root = doc.root();
        let parent = element_with_id(&mut doc, "div", root, None);
        // Each new holder goes in front, so the last one created is first in tree order.
        let mut front = None;
        for _ in 0..50 {
            let e = detached_element(&mut doc, "p", Some("many"));
            doc.attach_node(e, parent, Some(0)).unwrap();
            front = Some(e);
        }
        assert_eq!(doc.node_by_named_id("many"), front);

        let front = front.expect("holder");
        doc.delete_node_by_id(front);
        let next = doc.node_by_id(parent).expect("parent").children[0];
        assert_eq!(doc.node_by_named_id("many"), Some(next), "after deleting the first");

        // An ancestor of every holder takes over.
        doc.set_attribute(parent, "id", "many");
        assert_eq!(doc.node_by_named_id("many"), Some(parent));
    }

    fn shadow_root_of(doc: &mut DocumentImpl<Config>, host: NodeId) -> NodeId {
        doc.attach_shadow_root(host, ShadowRootInit::new(ShadowRootMode::Open), Location::default())
            .expect("div can host a shadow root")
    }

    /// Moving a subtree with duplicate ids within the document, out of it, into a shadow tree
    /// and back keeps every holder registered once, under the tree it is in.
    #[test]
    fn moving_a_subtree_with_duplicate_ids_keeps_them_current() {
        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let root = doc.root();
        let first = element_with_id(&mut doc, "div", root, Some("dup"));
        let box_ = element_with_id(&mut doc, "div", root, None);
        let inner = element_with_id(&mut doc, "p", box_, Some("dup"));
        let deeper = element_with_id(&mut doc, "span", inner, Some("other"));
        let registered = |doc: &DocumentImpl<Config>| {
            let mut v: Vec<_> = doc
                .named_ids_by_node
                .iter()
                .map(|(n, (r, id))| (n.as_usize(), r.as_usize(), id.to_string()))
                .collect();
            v.sort();
            v
        };

        // Within the document: before the first holder, then back after it.
        doc.attach_node(box_, root, Some(0)).unwrap();
        assert_eq!(doc.node_by_named_id("dup"), Some(inner));
        assert_eq!(doc.node_by_named_id("other"), Some(deeper));
        doc.attach_node(box_, root, None).unwrap();
        assert_eq!(doc.node_by_named_id("dup"), Some(first));
        let in_document = registered(&doc);
        assert_eq!(in_document.len(), 3);

        // Out of the document: only the first holder is left.
        doc.detach_node(box_);
        assert_eq!(doc.node_by_named_id("dup"), Some(first));
        assert_eq!(doc.node_by_named_id("other"), None);
        assert_eq!(
            registered(&doc),
            [(first.as_usize(), root.as_usize(), "dup".to_string())]
        );

        // Into a shadow tree, then back into the document.
        let host = element_with_id(&mut doc, "div", root, None);
        let shadow = shadow_root_of(&mut doc, host);
        doc.attach_node(box_, shadow, None).unwrap();
        assert_eq!(doc.node_by_named_id_in(shadow, "dup"), Some(inner));
        assert_eq!(doc.node_by_named_id_in(shadow, "other"), Some(deeper));
        assert_eq!(doc.node_by_named_id("other"), None);
        doc.attach_node(box_, root, None).unwrap();
        assert_eq!(doc.node_by_named_id_in(shadow, "dup"), None);
        assert_eq!(doc.node_by_named_id("other"), Some(deeper));
        assert_eq!(registered(&doc), in_document);
        assert!(!doc.named_id_elements.contains_key(&shadow));
    }

    #[test]
    fn ids_resolve_in_their_own_tree() {
        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let root = doc.root();
        let outer = element_with_id(&mut doc, "div", root, Some("x"));
        let host = element_with_id(&mut doc, "div", root, Some("host"));
        let shadow = shadow_root_of(&mut doc, host);
        let inner = element_with_id(&mut doc, "span", shadow, Some("x"));

        assert_eq!(doc.tree_root(outer), root);
        assert_eq!(doc.tree_root(inner), shadow);
        assert_eq!(doc.node_by_named_id_in(root, "x"), Some(outer));
        assert_eq!(doc.node_by_named_id_in(shadow, "x"), Some(inner));
        assert_eq!(
            doc.node_by_named_id_in(shadow, "host"),
            None,
            "the document's ids stay out"
        );

        doc.set_attribute(inner, "id", "y");
        assert_eq!(doc.node_by_named_id_in(shadow, "x"), None);
        assert_eq!(doc.node_by_named_id_in(shadow, "y"), Some(inner));
        assert_eq!(doc.node_by_named_id("y"), None);
    }

    #[test]
    fn a_nested_shadow_tree_has_its_own_ids() {
        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let root = doc.root();
        let host = element_with_id(&mut doc, "div", root, None);
        let outer = shadow_root_of(&mut doc, host);
        let inner_host = element_with_id(&mut doc, "div", outer, Some("a"));
        let inner = shadow_root_of(&mut doc, inner_host);
        let b = element_with_id(&mut doc, "span", inner, Some("b"));

        assert_eq!(doc.node_by_named_id_in(outer, "a"), Some(inner_host));
        assert_eq!(doc.node_by_named_id_in(outer, "b"), None);
        assert_eq!(doc.node_by_named_id_in(inner, "b"), Some(b));
        assert_eq!(doc.node_by_named_id_in(inner, "a"), None);
    }

    #[test]
    fn moving_a_host_keeps_its_shadow_ids() {
        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let root = doc.root();
        let host = element_with_id(&mut doc, "div", root, None);
        let shadow = shadow_root_of(&mut doc, host);
        let inner = element_with_id(&mut doc, "span", shadow, Some("x"));

        doc.detach_node(host);
        assert_eq!(doc.node_by_named_id_in(shadow, "x"), Some(inner));
        doc.attach_node(host, root, None).unwrap();
        assert_eq!(doc.node_by_named_id_in(shadow, "x"), Some(inner));
    }

    #[test]
    fn a_shadow_tree_moved_out_and_back_is_registered_again() {
        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let root = doc.root();
        let host = element_with_id(&mut doc, "div", root, None);
        let shadow = shadow_root_of(&mut doc, host);
        let parent = element_with_id(&mut doc, "div", shadow, None);
        let child = element_with_id(&mut doc, "span", parent, Some("c"));

        doc.detach_node(parent);
        assert_eq!(doc.node_by_named_id_in(shadow, "c"), None);
        doc.attach_node(parent, root, None).unwrap();
        assert_eq!(doc.node_by_named_id("c"), Some(child), "now in the document tree");
        doc.detach_node(parent);
        doc.attach_node(parent, shadow, None).unwrap();
        assert_eq!(doc.node_by_named_id_in(shadow, "c"), Some(child));
        assert_eq!(doc.node_by_named_id("c"), None);
    }

    #[test]
    fn deleting_the_host_drops_its_shadow_ids() {
        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let root = doc.root();
        let host = element_with_id(&mut doc, "div", root, None);
        let shadow = shadow_root_of(&mut doc, host);
        let inner = element_with_id(&mut doc, "span", shadow, Some("x"));

        doc.delete_node_by_id(host);
        assert!(!doc.named_id_elements.contains_key(&shadow));
        assert!(!doc.named_ids_by_node.contains_key(&inner));
    }

    #[test]
    fn a_detached_subtree_is_searched_in_place() {
        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let top = detached_element(&mut doc, "div", Some("top"));
        let first = detached_element(&mut doc, "span", Some("dup"));
        let second = detached_element(&mut doc, "span", Some("dup"));
        doc.attach_node(first, top, None).unwrap();
        doc.attach_node(second, top, None).unwrap();

        assert_eq!(doc.tree_root(second), top);
        assert_eq!(doc.node_by_named_id_in(top, "top"), Some(top));
        assert_eq!(doc.node_by_named_id_in(top, "dup"), Some(first));
        assert_eq!(doc.node_by_named_id_in(top, ""), None);
        assert_eq!(doc.node_by_named_id("dup"), None);
    }

    #[test]
    fn re_adding_a_duplicate_id_keeps_documents_equal() {
        let build = || {
            let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
            let root = doc.root();
            let a = element_with_id(&mut doc, "div", root, Some("x"));
            element_with_id(&mut doc, "div", root, Some("x"));
            (doc, a)
        };
        let (plain, _) = build();
        let (mut touched, a) = build();
        touched.set_attribute(a, "id", "y");
        touched.set_attribute(a, "id", "x");
        assert!(plain == touched);
        assert_eq!(touched.node_by_named_id("x"), Some(a));
    }

    #[test]
    fn a_subtree_left_by_a_delete_is_its_own_tree() {
        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let root = doc.root();
        let parent = element_with_id(&mut doc, "div", root, None);
        let kept = element_with_id(&mut doc, "div", parent, None);
        let input = element_with_id(&mut doc, "input", kept, Some("c"));

        let host = element_with_id(&mut doc, "div", root, None);
        let shadow = shadow_root_of(&mut doc, host);
        let inner = element_with_id(&mut doc, "div", shadow, None);
        let inner_input = element_with_id(&mut doc, "input", inner, Some("d"));

        doc.delete_node_by_id(parent);
        doc.delete_node_by_id(host);
        assert_eq!(doc.tree_root(input), kept);
        assert_eq!(doc.node_by_named_id_in(kept, "c"), Some(input));
        assert_eq!(doc.tree_root(inner_input), inner);
        assert_eq!(doc.node_by_named_id_in(inner, "d"), Some(inner_input));
        assert_eq!(doc.node_by_named_id("c"), None);
    }

    #[test]
    fn a_host_cannot_move_into_its_own_shadow_tree() {
        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let root = doc.root();
        let host = element_with_id(&mut doc, "div", root, None);
        let shadow = shadow_root_of(&mut doc, host);
        let inner = element_with_id(&mut doc, "div", shadow, None);

        assert_eq!(doc.attach_node(host, shadow, None), Err(AttachError::Cycle));
        doc.relocate_node(host, inner);
        Document::relocate_node(&mut doc, host, inner);
        assert_eq!(doc.parent(host), Some(root), "the host stays where it was");
        assert!(doc.children(shadow) == [inner]);
        assert!(doc.children(inner).is_empty());
    }

    #[test]
    fn a_node_cannot_move_into_its_own_subtree() {
        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let root = doc.root();
        let outer = element_with_id(&mut doc, "div", root, None);
        let inner = element_with_id(&mut doc, "div", outer, None);

        doc.relocate_node(outer, inner);
        assert_eq!(doc.parent(outer), Some(root), "not left detached");
        assert_eq!(doc.attach_node(outer, outer, None), Err(AttachError::Cycle));
        assert!(doc.children(outer) == [inner]);
    }

    #[test]
    fn relocating_a_node_to_its_own_parent_moves_it_to_the_end() {
        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let root = doc.root();
        let a = element(&mut doc, "div", root);
        let b = element(&mut doc, "div", root);

        doc.relocate_node(a, root);
        assert!(doc.children(root) == [b, a], "inherent");

        Document::relocate_node(&mut doc, b, root);
        assert!(doc.children(root) == [a, b], "through the trait");
    }

    #[test]
    fn attaching_an_attached_node_moves_it() {
        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let root = doc.root();
        let a = element_with_id(&mut doc, "div", root, None);
        let b = element_with_id(&mut doc, "div", root, None);
        let x = element_with_id(&mut doc, "div", a, None);

        doc.attach_node(x, b, None).unwrap();
        assert!(doc.children(a).is_empty(), "x left its old parent");
        assert!(doc.children(b) == [x]);
        assert_eq!(doc.parent(x), Some(b));

        // With x in both child lists this closed a cycle a -> x -> a.
        doc.attach_node(a, x, None).unwrap();
        assert_eq!(doc.parent(a), Some(x));
        assert!(doc.children(root) == [b]);
    }

    #[test]
    fn a_move_to_a_deleted_parent_leaves_the_tree_alone() {
        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let root = doc.root();
        let a = element_with_id(&mut doc, "div", root, None);
        let x = element_with_id(&mut doc, "div", a, Some("x"));
        let gone = element_with_id(&mut doc, "div", root, None);
        doc.delete_node_by_id(gone);

        assert_eq!(doc.attach_node(x, gone, None), Err(AttachError::MissingNode));
        doc.relocate_node(x, gone);
        Document::relocate_node(&mut doc, x, gone);
        assert_eq!(doc.parent(x), Some(a));
        assert!(doc.children(a) == [x]);
        assert_eq!(doc.node_by_named_id("x"), Some(x));
    }

    #[test]
    fn a_deleted_node_is_not_attached() {
        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let root = doc.root();
        let kept = element_with_id(&mut doc, "div", root, None);
        let gone = element_with_id(&mut doc, "div", root, None);
        doc.delete_node_by_id(gone);

        assert_eq!(doc.attach_node(gone, root, None), Err(AttachError::MissingNode));
        assert_eq!(
            Document::attach(&mut doc, gone, root, Some(0)),
            Err(AttachError::MissingNode)
        );
        assert!(doc.children(root) == [kept], "no child slot for a node that is gone");
    }

    #[test]
    fn a_refused_attach_says_why_and_changes_nothing() {
        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let root = doc.root();
        let outer = element_with_id(&mut doc, "div", root, None);
        let inner = element_with_id(&mut doc, "div", outer, None);
        let host = element_with_id(&mut doc, "div", root, None);
        let shadow = shadow_root_of(&mut doc, host);

        assert_eq!(doc.attach_node(outer, inner, None), Err(AttachError::Cycle));
        assert_eq!(doc.attach_node(shadow, outer, None), Err(AttachError::ShadowRoot));
        assert_eq!(Document::attach(&mut doc, outer, outer, None), Err(AttachError::Cycle));
        assert!(doc.children(root) == [outer, host]);
        assert!(doc.children(outer) == [inner]);
        assert_eq!(doc.parent(shadow), None);

        assert_eq!(Document::attach(&mut doc, inner, host, Some(0)), Ok(()));
        assert_eq!(doc.parent(inner), Some(host));
    }

    #[test]
    fn a_shadow_tree_keeps_ids_as_written_and_leaves_template_contents_out() {
        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let root = doc.root();
        let host = element_with_id(&mut doc, "div", root, None);
        let shadow = shadow_root_of(&mut doc, host);

        let spaced = element_with_id(&mut doc, "span", shadow, Some("has space"));
        assert_eq!(doc.node_by_named_id_in(shadow, "has space"), Some(spaced));

        let tpl = template(&mut doc, shadow);
        element_with_id(&mut doc, "span", tpl, Some("inside"));
        assert_eq!(doc.node_by_named_id_in(shadow, "inside"), None);

        let later = element_with_id(&mut doc, "p", shadow, Some("dup"));
        let earlier = detached_element(&mut doc, "p", Some("dup"));
        doc.attach_node(earlier, shadow, Some(0)).unwrap();
        assert_eq!(doc.node_by_named_id_in(shadow, "dup"), Some(earlier));
        assert_ne!(later, earlier);
    }
}
