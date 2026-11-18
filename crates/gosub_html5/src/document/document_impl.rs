use core::fmt::Debug;
use gosub_interface::css3::CssSystem;
use gosub_interface::document::{ControlEditState, Document, DocumentType, OpenSelect, PendingStylesheet};
use gosub_shared::attributes::Attributes;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::fmt;
use std::fmt::{Display, Formatter};
use url::Url;

use crate::document::presentation_hints;
use crate::document::task_queue::is_valid_id_attribute_value;
use crate::node::arena::NodeArena;
use crate::node::data::comment::CommentData;
use crate::node::data::doctype::DocTypeData;
use crate::node::data::element::{ClassListImpl, ElementData};
use crate::node::node_impl::{NodeDataTypeInternal, NodeImpl};
use crate::node::visitor::Visitor;
use crate::node::HTML_NAMESPACE;
use gosub_interface::config::HasDocument;
use gosub_interface::node::{is_valid_shadow_host_name, NodeType, QuirksMode, ShadowRootInit};
use gosub_shared::byte_stream::Location;
use gosub_shared::node::NodeId;
use gosub_shared::types::ParseError;

/// Defines a document
#[derive(Debug)]
pub struct DocumentImpl<C: HasDocument> {
    pub url: Option<Url>,
    pub(crate) arena: NodeArena,
    /// Elements by `id`, per tree: keyed by the tree's root, the document or a shadow root.
    /// Every holder of a duplicate id is included.
    named_id_elements: HashMap<NodeId, HashMap<String, Vec<NodeId>>>,
    /// Reverse index of `named_id_elements`: the tree and id each node is registered under.
    /// Kept in sync so unregistering a node does not require scanning the whole map.
    named_ids_by_node: HashMap<NodeId, (NodeId, String)>,
    pub doctype: DocumentType,
    pub quirks_mode: QuirksMode,
    /// The parser's scripting flag determines whether `noscript` text is serialized literally.
    scripting_enabled: bool,
    pub stylesheets: Vec<<C::CssSystem as CssSystem>::Stylesheet>,
    /// Links whose bytes have not arrived. Filled in after the parse; see
    /// [`Document::add_pending_stylesheet`].
    pending_stylesheets: Vec<PendingStylesheet>,
    hovered_nodes: parking_lot::RwLock<std::collections::HashSet<NodeId>>,
    focus: parking_lot::RwLock<FocusState>,
    /// Text controls the user has typed into; absent = still showing the markup value.
    edits: parking_lot::RwLock<HashMap<NodeId, ControlEditState>>,
    /// Checkboxes/radios the user has toggled; absent = the `checked` attribute.
    checked: parking_lot::RwLock<HashMap<NodeId, bool>>,
    /// `<select>` → option the user picked; absent = the `selected` attribute.
    selected: parking_lot::RwLock<HashMap<NodeId, NodeId>>,
    open_select: parking_lot::RwLock<Option<OpenSelect>>,
    /// Controls the user resized, with their border-box size.
    sizes: parking_lot::RwLock<HashMap<NodeId, (f64, f64)>>,
    /// The errors the HTML parser reported building this document, for whoever wants to show
    /// them; see [`DocumentImpl::set_parse_errors`].
    parse_errors: Vec<ParseError>,
}

#[derive(Debug, Default)]
struct FocusState {
    node: Option<NodeId>,
    /// `node` and its ancestors, for `:focus-within`.
    chain: std::collections::HashSet<NodeId>,
    /// Whether the focus ring shows (`:focus-visible`).
    visible: bool,
}

impl<C: HasDocument> PartialEq for DocumentImpl<C> {
    /// Compare document content and serialization metadata, including the scripting state.
    fn eq(&self, other: &Self) -> bool {
        self.url == other.url
            && self.arena == other.arena
            && self.named_id_elements == other.named_id_elements
            && self.doctype == other.doctype
            && self.quirks_mode == other.quirks_mode
            && self.scripting_enabled == other.scripting_enabled
            && self.stylesheets == other.stylesheets
    }
}

// ── new Document<C> trait impl ──────────────────────────────────────────────

impl<C: HasDocument<Document = Self>> Document<C> for DocumentImpl<C> {
    /// Create an empty document with its root registered and scripting enabled by default.
    fn new(document_type: DocumentType, url: Option<Url>) -> Self {
        let mut doc = Self {
            url,
            arena: NodeArena::new(),
            named_id_elements: HashMap::new(),
            named_ids_by_node: HashMap::new(),
            doctype: document_type,
            quirks_mode: QuirksMode::NoQuirks,
            scripting_enabled: true,
            stylesheets: Vec::new(),
            pending_stylesheets: Vec::new(),
            hovered_nodes: parking_lot::RwLock::new(std::collections::HashSet::new()),
            focus: parking_lot::RwLock::new(FocusState::default()),
            edits: parking_lot::RwLock::new(HashMap::new()),
            checked: parking_lot::RwLock::new(HashMap::new()),
            selected: parking_lot::RwLock::new(HashMap::new()),
            open_select: parking_lot::RwLock::new(None),
            sizes: parking_lot::RwLock::new(HashMap::new()),
            parse_errors: Vec::new(),
        };
        let root = NodeImpl::new_document(Location::default(), QuirksMode::NoQuirks);
        doc.arena.register_node(root);
        doc
    }

    // ── node creation ──────────────────────────────────────────────────────

    fn create_element(
        &mut self,
        name: &str,
        namespace: Option<&str>,
        attributes: Attributes,
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
        if self.refuses_copy(id) {
            return id;
        }
        let Some(node) = self.arena.node(id) else { return id };
        let cloned = NodeImpl::new_from_node(&node);
        self.register_node(cloned)
    }

    fn duplicate_node(&mut self, id: NodeId) -> NodeId {
        if self.refuses_copy(id) {
            return id;
        }
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

    fn attributes(&self, id: NodeId) -> Option<&Attributes> {
        match self.arena.node_ref(id)?.data {
            NodeDataTypeInternal::Element(ref e) => Some(&e.attributes),
            _ => None,
        }
    }

    fn set_attribute(&mut self, id: NodeId, name: &str, value: &str) {
        if let Some(node) = self.arena.node_ref_mut(id) {
            if let NodeDataTypeInternal::Element(ref mut e) = node.data {
                e.add_attribute(name, value);
            }
        }
        if name == "id" {
            self.refresh_named_id(id);
        }
    }

    fn remove_attribute(&mut self, id: NodeId, name: &str) {
        if let Some(node) = self.arena.node_ref_mut(id) {
            if let NodeDataTypeInternal::Element(ref mut e) = node.data {
                e.remove_attribute(name);
            }
        }
        if name == "id" {
            self.refresh_named_id(id);
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
    }

    // ── shadow trees ───────────────────────────────────────────────────────

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

    // ── stylesheets ────────────────────────────────────────────────────────

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

    // ── serialisation ──────────────────────────────────────────────────────

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

    fn is_hovered(&self, id: NodeId) -> bool {
        self.hovered_nodes.read().contains(&id)
    }

    fn is_focused(&self, id: NodeId) -> bool {
        self.focus.read().node == Some(id)
    }

    fn is_focus_visible(&self, id: NodeId) -> bool {
        let f = self.focus.read();
        f.visible && f.node == Some(id)
    }

    fn is_focus_within(&self, id: NodeId) -> bool {
        self.focus.read().chain.contains(&id)
    }

    fn focused_node(&self) -> Option<NodeId> {
        self.focus.read().node
    }

    fn control_edit_state(&self, id: NodeId) -> Option<ControlEditState> {
        self.edits.read().get(&id).cloned()
    }

    fn is_checked(&self, id: NodeId) -> bool {
        // `option:checked` = the select's chosen option.
        if self.tag_name(id) == Some("option") {
            let mut cur = self.parent(id);
            while let Some(p) = cur {
                if self.tag_name(p) == Some("select") {
                    return self.selected_option(p) == Some(id);
                }
                cur = self.parent(p);
            }
            return false;
        }
        match self.checked.read().get(&id) {
            Some(c) => *c,
            None => self.attribute(id, "checked").is_some(),
        }
    }

    fn selected_option(&self, select: NodeId) -> Option<NodeId> {
        if let Some(opt) = self.selected.read().get(&select) {
            return Some(*opt);
        }
        // Default: the `selected` attribute, else the first option.
        let mut first = None;
        let mut stack: Vec<NodeId> = self.children(select).iter().rev().copied().collect();
        while let Some(id) = stack.pop() {
            match self.tag_name(id) {
                Some("option") => {
                    if self.attribute(id, "selected").is_some() {
                        return Some(id);
                    }
                    first.get_or_insert(id);
                }
                Some("optgroup") => stack.extend(self.children(id).iter().rev()),
                _ => {}
            }
        }
        first
    }

    fn open_select(&self) -> Option<OpenSelect> {
        *self.open_select.read()
    }

    fn control_size(&self, id: NodeId) -> Option<(f64, f64)> {
        self.sizes.read().get(&id).copied()
    }
}

// ── Internal helpers (not part of Document trait) ───────────────────────────

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

    pub fn set_hovered_nodes(&self, leaf: Option<NodeId>) {
        let mut set = self.hovered_nodes.write();
        set.clear();
        if let Some(mut id) = leaf {
            loop {
                set.insert(id);
                match self.arena.node_ref(id).and_then(|n| n.parent) {
                    Some(parent) => id = parent,
                    None => break,
                }
            }
        }
    }

    /// Move focus to `node` (`None` blurs); `visible` = show the focus ring.
    pub fn set_focused_node(&self, node: Option<NodeId>, visible: bool) {
        let mut f = self.focus.write();
        f.node = node;
        f.visible = visible;
        f.chain.clear();
        if let Some(mut id) = node {
            loop {
                f.chain.insert(id);
                match self.arena.node_ref(id).and_then(|n| n.parent) {
                    Some(parent) => id = parent,
                    None => break,
                }
            }
        }
    }

    /// `None` reverts to the `selected` attribute.
    pub fn set_selected_option(&self, select: NodeId, option: Option<NodeId>) {
        let mut map = self.selected.write();
        match option {
            Some(o) => {
                map.insert(select, o);
            }
            None => {
                map.remove(&select);
            }
        }
    }

    /// `None` reverts to the stylesheet size.
    pub fn set_control_size(&self, id: NodeId, size: Option<(f64, f64)>) {
        let mut map = self.sizes.write();
        match size {
            Some(s) => {
                map.insert(id, s);
            }
            None => {
                map.remove(&id);
            }
        }
    }

    pub fn set_open_select(&self, state: Option<OpenSelect>) {
        *self.open_select.write() = state;
    }

    /// `None` reverts to the `checked` attribute.
    pub fn set_checked(&self, id: NodeId, checked: Option<bool>) {
        let mut map = self.checked.write();
        match checked {
            Some(c) => {
                map.insert(id, c);
            }
            None => {
                map.remove(&id);
            }
        }
    }

    /// `None` reverts to the markup value.
    pub fn set_control_edit_state(&self, id: NodeId, state: Option<ControlEditState>) {
        let mut edits = self.edits.write();
        match state {
            Some(s) => {
                edits.insert(id, s);
            }
            None => {
                edits.remove(&id);
            }
        }
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
    /// shadow trees do, detached subtrees and template contents do not.
    fn has_id_registry(&self, root: NodeId) -> bool {
        root == NodeId::root()
            || self
                .arena
                .node_ref(root)
                .is_some_and(|n| n.get_shadow_root_data().is_some())
    }

    /// The registry `node_id`'s id belongs in: the root of its tree, if that tree has one.
    fn registry_of(&self, node_id: NodeId) -> Option<NodeId> {
        let root = self.tree_root_of(node_id);
        self.has_id_registry(root).then_some(root)
    }

    /// Register `node_id` and everything below it in `registry`, or drop them when it is `None`.
    /// Shadow trees below it are not walked: they keep their own registry.
    fn refresh_named_ids(&mut self, node_id: NodeId, registry: Option<NodeId>) {
        let mut stack = vec![node_id];
        while let Some(id) = stack.pop() {
            if let Some(node) = self.arena.node_ref(id) {
                stack.extend_from_slice(&node.children);
            }
            self.register_named_id(id, registry);
        }
    }

    /// Bring `node_id`'s registration in line with its `id` attribute, after that changed.
    fn refresh_named_id(&mut self, node_id: NodeId) {
        let has_id = self
            .arena
            .node_ref(node_id)
            .and_then(NodeImpl::get_element_data)
            .is_some_and(|data| data.attributes.contains_key("id"));
        if !has_id && !self.named_ids_by_node.contains_key(&node_id) {
            return;
        }
        let registry = self.registry_of(node_id);
        self.register_named_id(node_id, registry);
    }

    /// Register `node_id` under its `id` attribute in `registry`, dropping whatever it was
    /// registered under before. No registry, or an absent or invalid value, just unregisters.
    fn register_named_id(&mut self, node_id: NodeId, registry: Option<NodeId>) {
        let Some(data) = self.arena.node_ref(node_id).and_then(NodeImpl::get_element_data) else {
            return;
        };
        let id_value = data
            .attributes
            .get("id")
            .filter(|v| is_valid_id_attribute_value(v))
            .cloned();
        self.unregister_named_id(node_id);
        let (Some(registry), Some(id_value)) = (registry, id_value) else {
            return;
        };
        self.named_id_elements
            .entry(registry)
            .or_default()
            .entry(id_value.clone())
            .or_default()
            .push(node_id);
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
            // From the end: a detached subtree unregisters its last-registered nodes first.
            if let Some(pos) = e.get().iter().rposition(|&n| n == node_id) {
                e.get_mut().remove(pos);
            }
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
            if let [only] = self.named_id_elements.get(&root)?.get(name_id)?.as_slice() {
                return Some(*only);
            }
        } else if !is_valid_id_attribute_value(name_id) {
            return None;
        }
        // Duplicates, or a tree without a registry: walk it in order and stop at the first holder.
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            let Some(node) = self.arena.node_ref(id) else {
                continue;
            };
            let holds = node
                .get_element_data()
                .and_then(|data| data.attributes.get("id"))
                .is_some_and(|v| v == name_id);
            if holds {
                return Some(id);
            }
            stack.extend(node.children.iter().rev());
        }
        None
    }

    /// Fetch a node reference by id (internal - not in the Document trait)
    pub fn node_by_id(&self, node_id: NodeId) -> Option<&NodeImpl> {
        self.arena.node_ref(node_id)
    }

    pub fn node_by_id_mut(&mut self, node_id: NodeId) -> Option<&mut NodeImpl> {
        self.arena.node_ref_mut(node_id)
    }

    /// Look up a node by its `id` attribute value, returning the node reference directly
    pub fn get_node_by_named_id(&self, name_id: &str) -> Option<&NodeImpl> {
        self.node_by_id(self.first_named_id_holder(NodeId::root(), name_id)?)
    }

    /// Returns the root node reference, which every document built through
    /// `Document::new()` has.
    ///
    /// `Option` rather than an `expect`, because both callers walk the tree and have something
    /// sensible to do with a document that has no root: print nothing, visit nothing.
    pub fn get_root(&self) -> Option<&NodeImpl> {
        self.arena.node_ref(NodeId::root())
    }

    /// Register a node (assigns id, marks registered). Does NOT attach to tree.
    pub fn register_node(&mut self, mut node: NodeImpl) -> NodeId {
        let node_id = self.arena.get_next_id();
        node.set_id(node_id);
        if let Some(element_data) = node.get_element_data_mut() {
            element_data.node_id = Some(node_id);
        }
        // Not in the document tree yet, so its `id` is registered by `attach_node`.
        self.arena.register_node_with_node_id(node, node_id);
        node_id
    }

    /// Register a node and attach it to a parent
    pub fn register_node_at(&mut self, node: NodeImpl, parent_id: NodeId, position: Option<usize>) -> NodeId {
        let node_id = self.register_node(node);
        self.attach_node(node_id, parent_id, position);
        node_id
    }

    pub fn attach_node(&mut self, node_id: NodeId, parent_id: NodeId, position: Option<usize>) {
        if parent_id == node_id || self.has_node_id_recursive(node_id, parent_id) {
            return;
        }
        // Guarded here rather than in the `Document` methods that call it: this one is public
        // and inherent, so it wins over the trait method for anything holding a concrete
        // `DocumentImpl` - a check on the trait side alone is simply stepped around.
        if self.refuses_tree_mutation("attach_node", node_id) {
            return;
        }
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
            log::warn!("attach_node: node {node_id} not found in arena");
            return;
        };
        node.parent = Some(parent_id);
        let registry = self.registry_of(node_id);
        self.refresh_named_ids(node_id, registry);
    }

    /// Whether a generic tree mutation must leave `node` alone because it is a shadow root.
    ///
    /// A shadow root hangs off its host by a side pointer and never appears in a `children`
    /// list. `attach_shadow_root` returns its raw id, though, so it is one call away from any
    /// of the ordinary mutations - and attaching it would give it a parent and put it into
    /// normal traversal, where it is both a shadow tree and a child of the element it shadows.
    /// Whether `node_id` must not be copied, because it is a shadow root.
    ///
    /// A shadow root's data holds the id of its host, so copying it produces a second root
    /// claiming the same host while the host still names the first - two roots, one host, and a
    /// back pointer that agrees with neither. `ShadowRoot.cloneNode()` throws in the spec for
    /// the same reason, so refusing is the faithful answer; the caller gets the original id
    /// back, which is what the not-found path already returns.
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
        }
    }

    pub fn relocate_node(&mut self, node_id: NodeId, parent_id: NodeId) {
        let Some(node) = self.arena.node_ref(node_id) else {
            return;
        };
        if !node.registered {
            return;
        }
        if node.parent == Some(parent_id) {
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
        let node_id = node.id();
        self.arena.update_node(node);
        self.refresh_named_id(node_id);
    }

    pub fn update_node_ref(&mut self, node: &NodeImpl) {
        if !node.is_registered() {
            log::warn!("Node is not registered to the arena");
            return;
        }
        self.arena.update_node(node.clone());
        self.refresh_named_id(node.id());
    }

    pub fn delete_node_by_id(&mut self, node_id: NodeId) {
        // The host and its shadow root each name the other, so deleting either has to clear the
        // link the other way round or the survivor keeps naming a node that is gone. Done here
        // rather than in `Document::remove` because this method is inherent and public, and so
        // wins over the trait method for anything holding a concrete `DocumentImpl`.
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
        self.clear_interaction_state(node_id);
        self.arena.delete_node(node_id);
    }

    /// Drop every piece of user-interaction state that names `node_id`. `NodeArena` never reuses
    /// ids, so nothing can inherit it - but without this the maps grow for the tab's lifetime and
    /// `focused_node()` / `selected_option()` keep handing out a deleted id. Deliberately *not*
    /// done in `detach_node`: relocation detaches before reattaching, and must keep the state.
    fn clear_interaction_state(&self, node_id: NodeId) {
        self.hovered_nodes.write().remove(&node_id);
        {
            let mut focus = self.focus.write();
            focus.chain.remove(&node_id);
            if focus.node == Some(node_id) {
                focus.node = None;
                focus.visible = false;
            }
        }
        self.edits.write().remove(&node_id);
        self.checked.write().remove(&node_id);
        self.sizes.write().remove(&node_id);
        // Also drop entries that *point at* the node: a deleted `<option>`.
        self.selected
            .write()
            .retain(|select, option| *select != node_id && *option != node_id);
        let mut open = self.open_select.write();
        if open.as_ref().is_some_and(|o| o.select == node_id) {
            *open = None;
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
        attributes: Attributes,
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
    if let Some(root) = doc.get_root() {
        internal_visit(doc, root, visitor);
    }
}

fn internal_visit<C: HasDocument<Document = DocumentImpl<C>>>(
    doc: &DocumentImpl<C>,
    node: &NodeImpl,
    visitor: &mut dyn Visitor,
) {
    visitor.document_enter(node);
    for &child_id in node.children() {
        let Some(child) = doc.node_by_id(child_id) else {
            continue;
        };
        internal_visit(doc, child, visitor);
    }
    visitor.document_leave(node);
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
        doc.attach_node(node, parent, None);
        node
    }

    fn detached_element(doc: &mut DocumentImpl<Config>, name: &str, id: Option<&str>) -> NodeId {
        let mut attributes = Attributes::new();
        if let Some(id) = id {
            attributes.insert("id".to_string(), id.to_string());
        }
        let node =
            DocumentImpl::<Config>::new_element_node(name, Some(HTML_NAMESPACE), attributes, Location::default());
        doc.register_node(node)
    }

    #[test]
    fn deleting_a_node_clears_its_interaction_state() {
        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let root = doc.root();
        let select = element(&mut doc, "select", root);
        let option = element(&mut doc, "option", select);
        let input = element(&mut doc, "input", root);

        doc.set_focused_node(Some(input), true);
        doc.set_checked(input, Some(true));
        doc.set_control_size(input, Some((10.0, 20.0)));
        doc.set_selected_option(select, Some(option));
        doc.set_open_select(Some(OpenSelect {
            select,
            hover: None,
            active: None,
            first_row: 0,
            viewport: (0.0, 100.0),
        }));

        doc.delete_node_by_id(input);
        assert_eq!(doc.focused_node(), None, "focus does not survive its node");
        assert!(!doc.is_checked(input));
        assert_eq!(doc.control_size(input), None);

        // Deleting the *option* clears the select's choice, which points at it.
        doc.delete_node_by_id(option);
        assert_eq!(doc.selected_option(select), None);

        doc.delete_node_by_id(select);
        assert!(doc.open_select().is_none(), "the open dropdown closes with its select");
    }

    /// Change `node`'s `id` attribute in place and run the mutation hook, the way a DOM
    /// `setAttribute` does.
    fn set_id(doc: &mut DocumentImpl<Config>, node: NodeId, id: Option<&str>) {
        let mut n = doc.node_by_id(node).expect("node").clone();
        let data = n.get_element_data_mut().expect("element");
        match id {
            Some(id) => {
                data.attributes.insert("id".to_string(), id.to_string());
            }
            None => {
                data.attributes.remove("id");
            }
        }
        doc.update_node(n);
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
    fn an_invalid_or_removed_id_unregisters_the_node() {
        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let root = doc.root();

        let a = element_with_id(&mut doc, "div", root, Some("keep"));
        set_id(&mut doc, a, Some("has space")); // whitespace is not a valid id
        assert_eq!(doc.node_by_named_id("keep"), None);
        assert_eq!(doc.node_by_named_id("has space"), None);

        let b = element_with_id(&mut doc, "div", root, Some("gone"));
        set_id(&mut doc, b, None);
        assert_eq!(doc.node_by_named_id("gone"), None);
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

        doc.attach_node(div, root, None);
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

        doc.attach_node(parent, root, None);
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
        doc.attach_node(earlier, root, Some(0));
        assert_eq!(doc.node_by_named_id("dup"), Some(earlier));
        assert_eq!(doc.get_node_by_named_id("dup").map(NodeImpl::id), Some(earlier));
        assert_ne!(later, earlier);

        // An ancestor comes before its descendants.
        let outer = element_with_id(&mut doc, "div", root, None);
        element_with_id(&mut doc, "span", outer, Some("nest"));
        doc.set_attribute(outer, "id", "nest");
        assert_eq!(doc.node_by_named_id("nest"), Some(outer));
    }

    fn shadow_root_of(doc: &mut DocumentImpl<Config>, host: NodeId) -> NodeId {
        doc.attach_shadow_root(host, ShadowRootInit::new(ShadowRootMode::Open), Location::default())
            .expect("div can host a shadow root")
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
        doc.attach_node(host, root, None);
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
        doc.attach_node(parent, root, None);
        assert_eq!(doc.node_by_named_id("c"), Some(child), "now in the document tree");
        doc.detach_node(parent);
        doc.attach_node(parent, shadow, None);
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
        doc.attach_node(first, top, None);
        doc.attach_node(second, top, None);

        assert_eq!(doc.tree_root(second), top);
        assert_eq!(doc.node_by_named_id_in(top, "top"), Some(top));
        assert_eq!(doc.node_by_named_id_in(top, "dup"), Some(first));
        assert_eq!(doc.node_by_named_id_in(top, ""), None);
        assert_eq!(doc.node_by_named_id("dup"), None);
    }
}
