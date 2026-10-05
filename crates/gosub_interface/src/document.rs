use crate::config::HasCssSystem;
use crate::css3::CssSystem;
use crate::node::{NodeType, QuirksMode, ShadowRootInit};
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

    // Tree structure - all navigation returns NodeId, never &Node

    fn root(&self) -> NodeId;
    fn parent(&self, id: NodeId) -> Option<NodeId>;
    fn children(&self, id: NodeId) -> &[NodeId];
    fn next_sibling(&self, id: NodeId) -> Option<NodeId>;

    fn attach(&mut self, node: NodeId, parent: NodeId, position: Option<usize>);
    fn detach(&mut self, node: NodeId);
    fn remove(&mut self, node: NodeId);

    /// Detach a node from its current parent and attach it to a new parent.
    fn relocate_node(&mut self, node: NodeId, parent: NodeId);

    fn node_type(&self, id: NodeId) -> NodeType;

    // Element data

    fn tag_name(&self, id: NodeId) -> Option<&str>;
    fn namespace(&self, id: NodeId) -> Option<&str>;

    fn attribute(&self, id: NodeId, name: &str) -> Option<&str>;
    fn attributes(&self, id: NodeId) -> Option<&HashMap<String, String>>;
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

    /// Look up a node by its `id` attribute value
    fn node_by_named_id(&self, name_id: &str) -> Option<NodeId>;

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
    /// where it was written.
    fn add_pending_stylesheet(&mut self, url: &str);

    /// Take the list of links still waiting for their bytes, as `(position, url)`.
    ///
    /// Draining, because there is exactly one consumer: the pipeline that fetches them.
    ///
    /// The position is what makes this answerable mid-parse, which is what a script will
    /// need: a classic `<script>` may not run until the sheets *before it* have applied,
    /// and those are the entries positioned at or below the current end of
    /// [`Document::stylesheets`].
    fn take_pending_stylesheets(&mut self) -> Vec<(usize, String)>;

    /// Put a fetched stylesheet at `position` in the cascade.
    ///
    /// `position` is what [`Document::take_pending_stylesheets`] reported, adjusted by the
    /// caller for sheets it has already inserted ahead of this one.
    fn insert_stylesheet(&mut self, position: usize, sheet: <C::CssSystem as CssSystem>::Stylesheet);

    // Serialisation

    fn write(&self) -> String;
    fn write_from_node(&self, id: NodeId) -> String;

    fn is_hovered(&self, _id: NodeId) -> bool {
        false
    }

    // Interaction state read by the `:focus`/`:checked` selectors and the painter.

    fn is_focused(&self, _id: NodeId) -> bool {
        false
    }
    /// Focused and the ring should show (keyboard focus, or a text-entry control).
    fn is_focus_visible(&self, _id: NodeId) -> bool {
        false
    }
    /// The focused element or one of its ancestors.
    fn is_focus_within(&self, _id: NodeId) -> bool {
        false
    }
    fn focused_node(&self) -> Option<NodeId> {
        None
    }
    /// Live checkedness; the `checked` attribute is only the default.
    fn is_checked(&self, id: NodeId) -> bool {
        self.attribute(id, "checked").is_some()
    }
    /// What has been typed into a text control; `None` = untouched (shows its markup value).
    fn control_edit_state(&self, _id: NodeId) -> Option<ControlEditState> {
        None
    }
    /// The chosen `<option>` of a `<select>` (the `selected` attribute or the first option until
    /// the user picks another). `None` = not tracked, use the markup.
    fn selected_option(&self, _select: NodeId) -> Option<NodeId> {
        None
    }
    /// Border-box size the user dragged a resizable control (`<textarea>`) to.
    fn control_size(&self, _id: NodeId) -> Option<(f64, f64)> {
        None
    }
    /// The open `<select>` dropdown, if any.
    fn open_select(&self) -> Option<OpenSelect> {
        None
    }
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
