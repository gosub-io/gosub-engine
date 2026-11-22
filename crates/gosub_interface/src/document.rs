use crate::config::HasCssSystem;
use crate::css3::CssSystem;
use crate::node::{NodeType, QuirksMode, ShadowRootInit};
use gosub_shared::attributes::Attributes;
use gosub_shared::byte_stream::Location;
use gosub_shared::node::NodeId;
use std::fmt::{Debug, Display};
use url::Url;

/// Whether this is a regular HTML document or a fragment (e.g. iframe srcdoc)
#[derive(PartialEq, Debug, Copy, Clone)]
pub enum DocumentType {
    HTML,
    IframeSrcDoc,
}

/// A `<link rel=stylesheet>` the parser has seen but whose bytes are not here yet; see
/// [`Document::add_pending_stylesheet`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingStylesheet {
    /// Where in [`Document::stylesheets`] the sheet belongs.
    pub position: usize,
    pub url: String,
    /// The link's `media` attribute, as written. Hand it to
    /// [`CssSystem::set_stylesheet_media`] once the sheet is parsed.
    pub media: Option<String>,
}

/// Why [`Document::attach`] left the tree as it was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachError {
    /// The node or the parent is not in the document.
    MissingNode,
    /// The node is the parent, or one of its host-including ancestors.
    Cycle,
    /// The node is a shadow root, which hangs off its host and is never anyone's child.
    ShadowRoot,
}

/// Storage-agnostic document interface.
///
/// All node data is accessed through `NodeId` handles. The concrete storage
/// (arena, column store, slotmap, etc.) is entirely hidden behind this trait.
/// No `Node` struct is ever handed out - callers ask the document questions
/// about a node by its ID.
pub trait Document<C: HasCssSystem>: Sized + Display + Debug + PartialEq + 'static {
    // Construction

    /// Create a new empty document of the given type.
    fn new(document_type: DocumentType, url: Option<Url>) -> Self;

    // Node creation - each returns the NodeId of the new node

    fn create_element(
        &mut self,
        name: &str,
        namespace: Option<&str>,
        attributes: Attributes,
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

    /// Deep-clone a node (and its subtree), unattached. Returns the new root NodeId.
    fn clone_node(&mut self, id: NodeId) -> NodeId;

    /// Shallow-copy a node: same type/data/attributes, no children, unattached.
    fn duplicate_node(&mut self, id: NodeId) -> NodeId;

    // Tree structure - all navigation returns NodeId, never &Node

    fn root(&self) -> NodeId;
    fn parent(&self, id: NodeId) -> Option<NodeId>;
    fn children(&self, id: NodeId) -> &[NodeId];
    fn next_sibling(&self, id: NodeId) -> Option<NodeId>;
    /// Whether `id` is a node of this document, attached or not. False once it is removed.
    fn contains(&self, id: NodeId) -> bool;

    /// Insert `node` under `parent`, at `position` or last, moving it from where it was. On an
    /// error nothing changes.
    fn attach(&mut self, node: NodeId, parent: NodeId, position: Option<usize>) -> Result<(), AttachError>;
    fn detach(&mut self, node: NodeId);
    /// Delete `node` itself, and its shadow root if it is a host. Their children are not
    /// deleted: each becomes the root of its own detached tree, and no id below `node` resolves
    /// from the document any more. DOM `remove()` and `removeChild()` keep the node alive, so
    /// they use `detach`.
    fn remove(&mut self, node: NodeId);

    /// Move `node` to the end of `parent`'s children, from wherever it was - also when that is
    /// `parent` already. A move that `attach` would refuse changes nothing.
    fn relocate_node(&mut self, node: NodeId, parent: NodeId);

    fn node_type(&self, id: NodeId) -> NodeType;

    // Element data

    fn tag_name(&self, id: NodeId) -> Option<&str>;
    fn namespace(&self, id: NodeId) -> Option<&str>;

    fn attribute(&self, id: NodeId, name: &str) -> Option<&str>;
    fn attributes(&self, id: NodeId) -> Option<&Attributes>;
    fn set_attribute(&mut self, id: NodeId, name: &str, value: &str);
    fn remove_attribute(&mut self, id: NodeId, name: &str);

    fn add_class(&mut self, id: NodeId, class: &str);
    fn has_class(&self, id: NodeId, name: &str) -> bool;

    /// The element's presentational hints, as the body of a declaration block.
    ///
    /// A markup language may give some of its attributes a meaning the style system has to
    /// honour - `<table cellspacing=4>` is `border-spacing: 4px`. Which attribute means what
    /// belongs to the language, so the document answers it and the cascade only has to rank
    /// what comes back (HTML §15.3). The text is CSS, so the cascade parses it with the real
    /// parser rather than trusting a second one here, and the declarations it yields join the
    /// author origin at specificity zero, ahead of every author sheet.
    ///
    /// `None` for an element with no such attribute, which is nearly all of them. A document
    /// for a language that has no presentational hints keeps the default.
    fn presentational_hints(&self, _id: NodeId) -> Option<String> {
        None
    }

    /// Contents of a `<template>` element (points to a fragment root node)
    fn template_contents(&self, id: NodeId) -> Option<NodeId>;
    fn set_template_contents(&mut self, id: NodeId, fragment: NodeId);

    // Shadow trees
    //
    // A shadow root is a node in the same arena as everything else, but it is not among its
    // host's `children` - the host points at it sideways, exactly like `template_contents`.
    // Nothing that walks `children` can therefore wander into a shadow tree by accident; only
    // code that asks for it explicitly sees one.

    /// Create a shadow root for `host` and link the two.
    ///
    /// Returns `None` - the spec's `NotSupportedError` - when `host` is not an HTML-namespace
    /// element, is not a [`valid shadow host name`](crate::node::is_valid_shadow_host_name),
    /// or already has a shadow root.
    fn attach_shadow_root(&mut self, host: NodeId, init: ShadowRootInit, location: Location) -> Option<NodeId>;

    /// The shadow root attached to element `id`, if it has one.
    fn shadow_root(&self, id: NodeId) -> Option<NodeId>;

    /// The host element of shadow root `id`. `None` when `id` is not a shadow root.
    fn shadow_host(&self, id: NodeId) -> Option<NodeId>;

    /// The flags shadow root `id` was created with. `None` when `id` is not a shadow root.
    fn shadow_root_init(&self, id: NodeId) -> Option<ShadowRootInit>;

    // Text / comment / doctype data

    fn text_value(&self, id: NodeId) -> Option<&str>;
    fn set_text_value(&mut self, id: NodeId, value: &str);

    /// Appends `value` to a text node's existing content in place, returning `true` if the
    /// node was a text node (and `false` otherwise, leaving it untouched). The parser uses
    /// this to merge adjacent text runs; implementations should append without rebuilding
    /// the whole string so that repeated appends stay amortized O(total length) rather than
    /// quadratic. The default falls back to read-and-replace and should be overridden.
    fn append_text_value(&mut self, id: NodeId, value: &str) -> bool {
        let Some(existing) = self.text_value(id) else {
            return false;
        };
        let mut combined = String::with_capacity(existing.len() + value.len());
        combined.push_str(existing);
        combined.push_str(value);
        self.set_text_value(id, &combined);
        true
    }

    fn comment_value(&self, id: NodeId) -> Option<&str>;

    fn doctype_name(&self, id: NodeId) -> Option<&str>;
    fn doctype_public_id(&self, id: NodeId) -> Option<&str>;
    fn doctype_system_id(&self, id: NodeId) -> Option<&str>;

    // Document-level metadata

    fn url(&self) -> Option<Url>;

    fn quirks_mode(&self) -> QuirksMode;
    fn set_quirks_mode(&mut self, mode: QuirksMode);

    fn doctype(&self) -> DocumentType;
    fn set_doctype(&mut self, doctype: DocumentType);

    /// The first element in tree order with this `id`, among the elements in the document tree
    /// (not detached, not inside a shadow tree or a template's contents).
    fn node_by_named_id(&self, name_id: &str) -> Option<NodeId>;

    /// The root of the tree `id` is in: the document, a shadow root, or the top of a subtree that
    /// is not attached anywhere.
    fn tree_root(&self, id: NodeId) -> NodeId;

    /// The first element in tree order with this `id` in the tree below `root`, usually a
    /// [`Self::tree_root`]. Ids resolve in the tree of the element that names them (`label for`,
    /// `form=`), and `ShadowRoot.getElementById` is this with the shadow root.
    fn node_by_named_id_in(&self, root: NodeId, name_id: &str) -> Option<NodeId>;

    /// Whether an element other than `node` has this `id` in the tree below `root`, which is the
    /// document or a shadow root. Answered from the id index: other roots never hold anything.
    fn named_id_held_by_other(&self, root: NodeId, name_id: &str, node: NodeId) -> bool;

    fn node_count(&self) -> usize;
    fn peek_next_id(&self) -> NodeId;

    // CSS stylesheets

    fn stylesheets(&self) -> &[<C::CssSystem as CssSystem>::Stylesheet];
    fn add_stylesheet(&mut self, sheet: <C::CssSystem as CssSystem>::Stylesheet);

    /// Note that the document links a stylesheet it does not have the bytes for.
    ///
    /// The parser records the link and moves on; whoever drives the parse fetches the sheet
    /// and puts it in place afterwards. The position it would have taken is remembered with
    /// it, because the cascade is document order and a sheet that arrives late still belongs
    /// where it was written. So is the link's `media` attribute, which the sheet is put under
    /// once it arrives.
    fn add_pending_stylesheet(&mut self, url: &str, media: Option<&str>);

    /// Take the list of links still waiting for their bytes.
    ///
    /// Draining, because there is exactly one consumer: the pipeline that fetches them.
    ///
    /// The position is what makes this answerable mid-parse, which is what a script will
    /// need: a classic `<script>` may not run until the sheets *before it* have applied,
    /// and those are the entries positioned at or below the current end of
    /// [`Document::stylesheets`].
    fn take_pending_stylesheets(&mut self) -> Vec<PendingStylesheet>;

    /// Put a fetched stylesheet at `position` in the cascade.
    ///
    /// `position` is what [`Document::take_pending_stylesheets`] reported, adjusted by the
    /// caller for sheets it has already inserted ahead of this one.
    fn insert_stylesheet(&mut self, position: usize, sheet: <C::CssSystem as CssSystem>::Stylesheet);

    // Serialisation

    /// Whether scripting is enabled for this document, including `noscript` serialization.
    /// The default matches the HTML parser's default options.
    fn scripting_enabled(&self) -> bool {
        true
    }

    /// Retain the parser's scripting flag for later serialization. Documents supporting
    /// scripting-disabled parsing override this along with [`Document::scripting_enabled`].
    fn set_scripting_enabled(&mut self, _enabled: bool) {}

    fn write(&self) -> String;
    fn write_from_node(&self, id: NodeId) -> String;

    /// Serialize an inline SVG subtree for an XML parser. HTML document implementations must
    /// override this to escape raw text, avoid HTML-only entities, and close void elements.
    /// Other document implementations can keep their existing XML-compatible serialization.
    fn write_xml_from_node(&self, id: NodeId) -> String {
        self.write_from_node(id)
    }
    /// [`Document::write_xml_from_node`] with the attributes `attrs` gives for each element set
    /// on it, replacing any attribute of the same name.
    fn write_svg_from_node(&self, id: NodeId, attrs: &dyn Fn(NodeId) -> Vec<(&'static str, String)>) -> String;
}

/// An open `<select>` dropdown. Row indices count every popup row (options and group labels).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OpenSelect {
    pub select: NodeId,
    /// Row under the pointer (light highlight).
    pub hover: Option<usize>,
    /// Row the keyboard moved to (strong highlight); committed by Enter/Space.
    pub active: Option<usize>,
    /// First row shown when the list is taller than the popup.
    pub first_row: usize,
    /// Viewport `(top, height)` in page px when the dropdown opened: decides whether the popup
    /// opens below or above, and how tall it may be.
    pub viewport: (f64, f64),
}

/// The DOM value of a text control (as opposed to its `value` attribute) plus its editing state.
/// Indices are char indices into `value`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlEditState {
    pub value: String,
    pub caret: usize,
    /// Other end of the selection; `None` (or equal to `caret`) = nothing selected.
    pub anchor: Option<usize>,
    /// First visual row a `<textarea>` shows (the engine keeps the caret inside the view).
    pub scroll: usize,
}

impl ControlEditState {
    pub fn new(value: String, caret: usize) -> Self {
        ControlEditState {
            value,
            caret,
            anchor: None,
            scroll: 0,
        }
    }

    /// The selected char range `[start, end)`, if anything is selected.
    pub fn selection(&self) -> Option<(usize, usize)> {
        let a = self.anchor?;
        if a == self.caret {
            return None;
        }
        Some((a.min(self.caret), a.max(self.caret)))
    }
}
