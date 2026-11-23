//! What the user did to a page that its markup does not say: where the pointer is, what has
//! focus, what was typed, toggled, picked or resized, and which dropdown is open.
//!
//! [`InteractionState`] is what input writes. It names nodes and nothing else, so it cannot go
//! stale when the tree changes. [`InteractionSnapshot`] is what a style or paint pass reads: it
//! is built from the state and the tree as it is then, which puts the hover and focus chains
//! where the nodes are now and leaves out entries whose node is gone.

use crate::config::HasDocument;
use crate::document::{ControlEditState, Document, OpenSelect};
use gosub_shared::node::NodeId;
use std::collections::{HashMap, HashSet};

/// The user-interaction state of one page.
#[derive(Debug, Clone, Default)]
pub struct InteractionState {
    /// The node under the pointer. Its ancestors are hovered too; the snapshot finds them.
    pub hover_leaf: Option<NodeId>,
    pub focus: Option<NodeId>,
    /// Whether the focus ring shows (`:focus-visible`).
    pub focus_visible: bool,
    /// Text controls the user has typed into; absent = still showing the markup value.
    pub edits: HashMap<NodeId, ControlEditState>,
    /// Checkboxes/radios the user has toggled; absent = the `checked` attribute.
    pub checked: HashMap<NodeId, bool>,
    /// `<select>` -> option the user picked; absent = the `selected` attribute.
    pub selected: HashMap<NodeId, NodeId>,
    /// Controls the user resized, with their border-box size.
    pub sizes: HashMap<NodeId, (f64, f64)>,
    pub open_select: Option<OpenSelect>,
}

impl InteractionState {
    /// `None` reverts to the markup value.
    pub fn set_edit(&mut self, id: NodeId, state: Option<ControlEditState>) {
        set_or_remove(&mut self.edits, id, state);
    }

    /// `None` reverts to the `checked` attribute.
    pub fn set_checked(&mut self, id: NodeId, checked: Option<bool>) {
        set_or_remove(&mut self.checked, id, checked);
    }

    /// `None` reverts to the `selected` attribute.
    pub fn set_selected(&mut self, select: NodeId, option: Option<NodeId>) {
        set_or_remove(&mut self.selected, select, option);
    }

    /// `None` reverts to the stylesheet size.
    pub fn set_size(&mut self, id: NodeId, size: Option<(f64, f64)>) {
        set_or_remove(&mut self.sizes, id, size);
    }

    /// Forget every entry naming a node `doc` no longer has. Node ids are not reused, so such
    /// an entry can never match again; without this the maps grow for the page's lifetime.
    /// Detached nodes stay: they may come back.
    pub fn prune<C: HasDocument>(&mut self, doc: &C::Document) {
        let live = |id: &NodeId| doc.contains(*id);
        if !self.hover_leaf.as_ref().is_none_or(live) {
            self.hover_leaf = None;
        }
        if !self.focus.as_ref().is_none_or(live) {
            self.focus = None;
            self.focus_visible = false;
        }
        self.edits.retain(|id, _| live(id));
        self.checked.retain(|id, _| live(id));
        self.selected.retain(|select, option| live(select) && live(option));
        self.sizes.retain(|id, _| live(id));
        if !self.open_select.as_ref().is_none_or(|o| live(&o.select)) {
            self.open_select = None;
        }
    }
}

fn set_or_remove<V>(map: &mut HashMap<NodeId, V>, id: NodeId, value: Option<V>) {
    match value {
        Some(v) => {
            map.insert(id, v);
        }
        None => {
            map.remove(&id);
        }
    }
}

/// [`InteractionState`] read against the tree, for one pass.
#[derive(Debug, Clone, Default)]
pub struct InteractionSnapshot {
    /// The hovered node and its ancestors.
    hovered: HashSet<NodeId>,
    focus: Option<NodeId>,
    focus_visible: bool,
    /// The focused node and its ancestors, for `:focus-within`.
    focus_within: HashSet<NodeId>,
    edits: HashMap<NodeId, ControlEditState>,
    checked: HashMap<NodeId, bool>,
    /// Only picks that are still one of their select's options.
    selected: HashMap<NodeId, NodeId>,
    sizes: HashMap<NodeId, (f64, f64)>,
    open_select: Option<OpenSelect>,
}

impl InteractionSnapshot {
    /// Read `state` against `doc` as it is now. Entries naming a node `doc` no longer has are
    /// left out; `state` keeps them, which is harmless as node ids are not reused.
    pub fn new<C: HasDocument>(state: &InteractionState, doc: &C::Document) -> Self {
        let live = |id: &NodeId| doc.contains(*id);
        let focus = state.focus.filter(live);
        Self {
            hovered: ancestors_and_self::<C>(doc, state.hover_leaf.filter(live)),
            focus,
            focus_visible: focus.is_some() && state.focus_visible,
            focus_within: ancestors_and_self::<C>(doc, focus),
            edits: live_entries(&state.edits, live),
            checked: live_entries(&state.checked, live),
            selected: state
                .selected
                .iter()
                .filter(|(&select, &option)| live(&select) && live(&option) && is_option_of::<C>(doc, option, select))
                .map(|(&select, &option)| (select, option))
                .collect(),
            sizes: live_entries(&state.sizes, live),
            open_select: state.open_select.filter(|o| live(&o.select)),
        }
    }

    pub fn is_hovered(&self, id: NodeId) -> bool {
        self.hovered.contains(&id)
    }

    pub fn is_focused(&self, id: NodeId) -> bool {
        self.focus == Some(id)
    }

    /// Focused and the ring should show (keyboard focus, or a text-entry control).
    pub fn is_focus_visible(&self, id: NodeId) -> bool {
        self.focus_visible && self.focus == Some(id)
    }

    /// The focused element or one of its ancestors.
    pub fn is_focus_within(&self, id: NodeId) -> bool {
        self.focus_within.contains(&id)
    }

    pub fn focused_node(&self) -> Option<NodeId> {
        self.focus
    }

    /// What has been typed into a text control; `None` = untouched (shows its markup value).
    pub fn control_edit_state(&self, id: NodeId) -> Option<&ControlEditState> {
        self.edits.get(&id)
    }

    /// Border-box size the user dragged a resizable control (`<textarea>`) to.
    pub fn control_size(&self, id: NodeId) -> Option<(f64, f64)> {
        self.sizes.get(&id).copied()
    }

    /// The open `<select>` dropdown, if any.
    pub fn open_select(&self) -> Option<OpenSelect> {
        self.open_select
    }

    /// Live checkedness; the `checked` attribute is only the default. An `<option>` is checked
    /// when it is its select's chosen option.
    pub fn is_checked<C: HasDocument>(&self, doc: &C::Document, id: NodeId) -> bool {
        if doc.tag_name(id) == Some("option") {
            let mut cur = doc.parent(id);
            while let Some(p) = cur {
                if doc.tag_name(p) == Some("select") {
                    return self.selected_option::<C>(doc, p) == Some(id);
                }
                cur = doc.parent(p);
            }
            return false;
        }
        match self.checked.get(&id) {
            Some(c) => *c,
            None => doc.attribute(id, "checked").is_some(),
        }
    }

    /// The chosen `<option>` of a `<select>`: the one the user picked, else the one with the
    /// `selected` attribute, else the first.
    pub fn selected_option<C: HasDocument>(&self, doc: &C::Document, select: NodeId) -> Option<NodeId> {
        if let Some(&option) = self.selected.get(&select) {
            return Some(option);
        }
        let mut first = None;
        let mut stack: Vec<NodeId> = doc.children(select).iter().rev().copied().collect();
        while let Some(id) = stack.pop() {
            match doc.tag_name(id) {
                Some("option") => {
                    if doc.attribute(id, "selected").is_some() {
                        return Some(id);
                    }
                    first.get_or_insert(id);
                }
                Some("optgroup") => stack.extend(doc.children(id).iter().rev()),
                _ => {}
            }
        }
        first
    }
}

fn live_entries<V: Clone>(map: &HashMap<NodeId, V>, live: impl Fn(&NodeId) -> bool) -> HashMap<NodeId, V> {
    map.iter()
        .filter(|(id, _)| live(id))
        .map(|(&id, v)| (id, v.clone()))
        .collect()
}

/// `node` and every ancestor of it, up to the root of its tree.
fn ancestors_and_self<C: HasDocument>(doc: &C::Document, node: Option<NodeId>) -> HashSet<NodeId> {
    let mut chain = HashSet::new();
    let mut current = node;
    while let Some(id) = current {
        chain.insert(id);
        current = doc.parent(id);
    }
    chain
}

/// Whether `option` is in `select`'s list of options, which is the case when `select` is its
/// nearest ancestor select (<https://html.spec.whatwg.org/#option-element-nearest-ancestor-select>).
fn is_option_of<C: HasDocument>(doc: &C::Document, option: NodeId, select: NodeId) -> bool {
    let mut seen_optgroup = false;
    let mut current = doc.parent(option);
    while let Some(ancestor) = current {
        match doc.tag_name(ancestor) {
            Some("datalist" | "hr" | "option") => return false,
            Some("optgroup") if seen_optgroup => return false,
            Some("optgroup") => seen_optgroup = true,
            Some("select") => return ancestor == select,
            _ => {}
        }
        current = doc.parent(ancestor);
    }
    false
}
