//! The list of active formatting elements.

use crate::parser::helper::matches_tag_and_attrs_without_order;
use crate::parser::Html5Parser;
use gosub_interface::config::HasDocument;
use gosub_interface::document::Document;
use gosub_shared::node::NodeId;

/// Active formatting elements, which could be a regular node(id), or a marker
#[derive(PartialEq, Clone, Copy)]
pub(crate) enum ActiveElement {
    Node(NodeId),
    Marker,
}

impl ActiveElement {
    pub(crate) fn node_id(&self) -> Option<NodeId> {
        match self {
            ActiveElement::Node(id) => Some(*id),
            ActiveElement::Marker => None,
        }
    }
}

impl<C: HasDocument> Html5Parser<'_, C> {
    /// Returns true if the given tag if found in the active formatting elements list (until the first marker)
    pub(crate) fn active_formatting_elements_has_until_marker(&self, tag: &str) -> Option<NodeId> {
        if self.active_formatting_elements.is_empty() {
            return None;
        }

        let mut idx = self.active_formatting_elements.len() - 1;
        loop {
            match self.active_formatting_elements[idx] {
                ActiveElement::Marker => return None,
                ActiveElement::Node(node_id) => {
                    if self.document.tag_name(node_id).unwrap_or_default() == tag {
                        return Some(node_id);
                    }
                }
            }

            if idx == 0 {
                // Reached the beginning of the list
                return None;
            }

            idx -= 1;
        }
    }

    /// Adds a marker to the active formatting stack
    pub(crate) fn active_formatting_elements_push_marker(&mut self) {
        self.active_formatting_elements.push(ActiveElement::Marker);
    }

    /// Clear the active formatting stack until we reach the first marker
    pub(crate) fn active_formatting_elements_clear_until_marker(&mut self) {
        while let Some(active_elem) = self.active_formatting_elements.pop() {
            if let ActiveElement::Marker = active_elem {
                // Found the marker
                return;
            }
        }
    }

    /// Remove the given `node_id` from the active formatting elements list. Will do nothing when the node is not found
    pub(crate) fn active_formatting_elements_remove(&mut self, target_node_id: NodeId) {
        self.active_formatting_elements.retain(|node_id| match node_id {
            ActiveElement::Node(node_id) => *node_id != target_node_id,
            ActiveElement::Marker => true,
        });
    }

    /// Push a node onto the active formatting stack, make sure only max 3 of them can be added (between markers)
    pub(crate) fn active_formatting_elements_push(&mut self, node_id: NodeId) {
        let mut matched = 0;
        let mut first_matched = None;

        for entry in self.active_formatting_elements.iter().rev() {
            match entry {
                ActiveElement::Marker => break,
                &ActiveElement::Node(id) => {
                    if matches_tag_and_attrs_without_order::<C>(self.document, node_id, id) {
                        if matched >= 2 {
                            first_matched = Some(id);
                            break;
                        }
                        matched += 1;
                    }
                }
            }
        }
        if let Some(first_matched) = first_matched {
            self.active_formatting_elements
                .retain(|n| n != &ActiveElement::Node(first_matched));
        }

        self.active_formatting_elements.push(ActiveElement::Node(node_id));
    }

    pub(crate) fn reconstruct_formatting(&mut self) {
        if self.active_formatting_elements.is_empty() {
            return; // Nothing to reconstruct.
        }

        let mut entry_index: usize = self.active_formatting_elements.len() - 1;
        let entry = self.active_formatting_elements[entry_index];

        // If it's a marker or in the stack of open elements, nothing to reconstruct.
        if let ActiveElement::Marker = entry {
            return;
        }

        if entry.node_id().is_some_and(|id| self.open_elements.contains(&id)) {
            return;
        }

        loop {
            // If it's a marker or in the stack of open elements, nothing to reconstruct.
            let entry = self.active_formatting_elements[entry_index];
            if let ActiveElement::Marker = entry {
                entry_index += 1;
                break;
            }

            if entry.node_id().is_some_and(|id| self.open_elements.contains(&id)) {
                entry_index += 1;
                break;
            }

            if entry_index == 0 {
                break;
            }

            entry_index -= 1;
        }

        loop {
            let entry = self.active_formatting_elements[entry_index];
            let Some(node_id) = entry.node_id() else {
                // Marker found. This should not happen!
                break;
            };

            let new_node_id = self.insert_element_from_node(node_id, None);

            self.active_formatting_elements[entry_index] = ActiveElement::Node(new_node_id);

            if entry_index == self.active_formatting_elements.len() - 1 {
                break;
            }

            entry_index += 1;
        }
    }
}
