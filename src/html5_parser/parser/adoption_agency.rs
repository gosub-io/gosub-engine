use crate::html5_parser::node::{Node, NodeData, HTML_NAMESPACE};
use crate::html5_parser::parser::{ActiveElement, Html5Parser, Scope};
use crate::html5_parser::tokenizer::token::Token;
use std::collections::HashMap;

const ADOPTION_AGENCY_OUTER_LOOP_DEPTH: usize = 8;
const ADOPTION_AGENCY_INNER_LOOP_DEPTH: usize = 3;

pub enum AdoptionResult {
    ProcessAsAnyOther,
    Completed,
}

impl<'a> Html5Parser<'a> {
    pub fn run_adoption_agency(&mut self, token: &Token) -> AdoptionResult {
        // Step 1
        let subject = match token {
            Token::EndTagToken { name, .. } => name,
            Token::StartTagToken { name, .. } => name,
            _ => panic!("run adoption agency called with non start/end tag token"),
        };

        // Step 2
        let current_node_id = current_node!(self).id;
        if current_node!(self).name == *subject
            && !self
                .active_formatting_elements
                .iter()
                .any(|elem| elem == &ActiveElement::NodeId(current_node_id))
        {
            self.open_elements.pop();
            return AdoptionResult::Completed
        }

        // Step 3
        let mut outer_loop_counter = 0;

        // Step 4
        loop {
            // Step 4.1
            if outer_loop_counter >= ADOPTION_AGENCY_OUTER_LOOP_DEPTH {
                return AdoptionResult::Completed
            }

            // Step 4.2
            outer_loop_counter += 1;

            // Step 4.3
            let formatting_element_idx = self.find_formatting_element(subject);
            if formatting_element_idx.is_none() {
                return AdoptionResult::ProcessAsAnyOther
            }

            let formatting_element_idx = formatting_element_idx.expect("formatting element not found");
            let formatting_element_id = self.active_formatting_elements[formatting_element_idx].node_id().expect("formatting element not found");
            let formatting_element_node= self.document.get_node_by_id(formatting_element_id).expect("formatting element not found").clone();

            // Step 4.4
            if !open_elements_has_id!(self, formatting_element_id) {
                self.parse_error("formatting element not in open elements");
                self.active_formatting_elements
                    .remove(formatting_element_idx);

                return AdoptionResult::Completed
            }

            // Step 4.5
            if !self.is_in_scope(&formatting_element_node.name, Scope::Regular)
            {
                self.parse_error("formatting element not in scope");
                return AdoptionResult::Completed;
            }

            // Step 4.6
            if formatting_element_id != current_node_id {
                self.parse_error("formatting element not current node");
                // do not return here
            }

            // Step 4.7
            let furthest_block_idx = self.find_furthest_block_idx(formatting_element_id);

            // Step 4.8
            if furthest_block_idx.is_none() {
                // Remove up until and including the formatting element from the stack of open elements
                while let Some(top) = self.open_elements.last() {
                    if top == &formatting_element_id {
                        self.open_elements.pop();
                        break;
                    } else {
                        self.open_elements.pop();
                    }
                }

                // Remove the formatting element from the list of active formatting elements
                if let Some(pos) = self.active_formatting_elements.iter().position(|elem| elem == &ActiveElement::NodeId(formatting_element_id)) {
                    self.active_formatting_elements.remove(pos);
                }

                return AdoptionResult::Completed
            }

            let furthest_block_idx = furthest_block_idx.expect("furthest block not found");

            let node_id = *self.open_elements.get(furthest_block_idx).expect("node not found");
            let furthest_block = self.document.get_node_by_id(node_id).expect("node not found").clone();

            // Step 4.9
            let common_ancestor_id = *self.open_elements.get(formatting_element_idx + 1).expect("node not found");

            // Step 4.10
            let mut bookmark = formatting_element_idx;

            // Step 4.11
            let mut node_idx = furthest_block_idx;
            let last_node_idx = furthest_block_idx;
            let mut last_node_id = *self
                .open_elements
                .get(last_node_idx)
                .expect("last node not found");

            // Step 4.12
            let mut inner_loop_counter = 0;

            // Step 4.13
            loop {
                // Step 4.13.1
                inner_loop_counter += 1;

                node_idx -= 1;

                // Step 4.13.2
                let node_id = self.open_elements[node_idx];
                let node = open_elements_get!(self, node_idx).clone();

                // Step 4.13.3
                if node_id == formatting_element_id {
                    break;
                }

                // Step 4.13.4
                if inner_loop_counter > ADOPTION_AGENCY_INNER_LOOP_DEPTH
                    && self
                        .active_formatting_elements
                        .contains(&ActiveElement::NodeId(node_id))
                {
                    self.active_formatting_elements.remove(node_idx);

                    if node_id <= bookmark {
                        bookmark -= 1;
                    }

                    continue;
                }

                // Step 4.13.5
                if !self
                    .active_formatting_elements
                    .contains(&ActiveElement::NodeId(node_id))
                {
                    // We have removed the node from the given node_idx
                    self.open_elements.remove(node_idx);
                    continue;
                }

                // Step 4.13.6
                // replace the old node with the new replacement node
                let replacement_node_id = self.replace_node(&node, node_idx, common_ancestor_id);

                // Step 4.13.7
                if last_node_idx == furthest_block_idx {
                    bookmark = node_idx + 1;
                }

                // Step 4.13.8
                self.document.append(last_node_id, replacement_node_id);

                // Step 4.13.9
                last_node_id = replacement_node_id;
            }

            // Step 4.14
            self.document.append(last_node_id, common_ancestor_id);

            // Step 4.15
            let new_element = match formatting_element_node.data {
                NodeData::Element { ref attributes, .. } => {
                     Node::new_element(
                        formatting_element_node.name.as_str(),
                        attributes.clone(),
                        HTML_NAMESPACE,
                    )
                }
                _ => panic!("formatting element is not an element")
            };

            // Step 4.16
            for &child in furthest_block.children.iter() {
                self.document.append(child, new_element.id)
            }

            // Step 4.17
            let new_element_id = self.document.add_node(new_element, furthest_block.id);

            // Step 4.18
            self.active_formatting_elements
                .remove(formatting_element_idx);
            self.active_formatting_elements
                .insert(bookmark, ActiveElement::NodeId(new_element_id));

            // Step 4.19
            // Remove formatting element from the stack of open elements, and insert the new element into the stack of open elements immediately below the position of furthest block in that stack.
            self.open_elements.remove(formatting_element_idx);
            self.open_elements.insert(furthest_block_idx - 1, new_element_id);
        }
    }

    fn replace_node(&mut self, node: &Node, node_idx: usize, common_ancestor: usize) -> usize {
        let node_attributes = match node.data {
            NodeData::Element { ref attributes, .. } => attributes.clone(),
            _ => HashMap::new(),
        };

        let replacement_node =
            Node::new_element(node.name.as_str(), node_attributes, HTML_NAMESPACE);
        let replacement_node_id = self.document.add_node(replacement_node, common_ancestor);

        self.active_formatting_elements[node_idx] = ActiveElement::NodeId(replacement_node_id);
        self.open_elements[node_idx] = replacement_node_id;

        replacement_node_id
    }

    // Find the furthest block element in the stack of open elements that is above the formatting element
    fn find_furthest_block_idx(&self, formatting_element_id: usize) -> Option<usize> {
        let mut index_of_formatting_element = None;

        // Find the index of the wanted formatting element id
        for (idx, &element_id) in self.open_elements.iter().enumerate() {
            if element_id == formatting_element_id {
                index_of_formatting_element = Some(idx);
                break;
            }
        }

        let index_of_formatting_element = match index_of_formatting_element {
            Some(idx) => idx,
            None => return None,
        };

        // Iterate
        for idx in (index_of_formatting_element..self.open_elements.len()).rev() {
            let element_id = self.open_elements[idx];
            let element = self.document.get_node_by_id(element_id).expect("element not found");

            if element.is_special() {
                return Some(idx);
            }
        }

        None
    }


    // Find the formatting element with the given subject between the end of the list and the first marker (or start when there is no marker)
    fn find_formatting_element(&self, subject: &str) -> Option<usize> {
        if self.active_formatting_elements.is_empty() {
            return None;
        }

        for idx in (0..self.active_formatting_elements.len()).rev() {
            match self.active_formatting_elements[idx] {
                ActiveElement::Marker => {
                    // Marker found, do not continue
                    break;
                },
                ActiveElement::NodeId(node_id) => {
                    // Check if the given node is an element with the given subject
                    let node = self.document.get_node_by_id(node_id).expect("node not found").clone();
                    if let NodeData::Element {
                        ref name,
                        ..
                    } = node.data
                    {
                        if name == subject {
                            return Some(idx);
                        }
                    }
                }
            }
        }

        None
    }
}