//! What the user did to a page that its markup does not say: where the pointer is, what has
//! focus, what was typed, toggled, picked or resized, and which dropdown is open.

use crate::document::{ControlEditState, OpenSelect};
use gosub_shared::node::NodeId;
use std::collections::HashMap;

/// The user-interaction state of one page.
#[derive(Debug, Clone, Default)]
pub struct InteractionState {
    /// The node under the pointer. Its ancestors are hovered too; they are found from the tree
    /// when asked, so they follow it when it changes.
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
