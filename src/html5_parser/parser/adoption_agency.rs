use crate::html5_parser::node::{Node, NodeData, HTML_NAMESPACE};
use crate::html5_parser::parser::{ActiveElement, Html5Parser, Scope};
use crate::html5_parser::tokenizer::token::Token;
use std::collections::HashMap;

const ADOPTION_AGENCY_OUTER_LOOP_DEPTH: usize = 8;
const ADOPTION_AGENCY_INNER_LOOP_DEPTH: usize = 3;

impl<'a> Html5Parser<'a> {
    pub fn run_adoption_agency(&mut self, token: &Token) {
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
            return;
        }

        // Step 3
        let mut outer_loop_counter = 0;

        // Step 4
        loop {
            // Step 4.1
            if outer_loop_counter >= ADOPTION_AGENCY_OUTER_LOOP_DEPTH {
                return;
            }

            // Step 4.2
            outer_loop_counter += 1;

            // Step 4.3
            let mut formatting_element_idx: usize = 0;
            let mut formatting_element_id: usize = 0;
            let mut formatting_element_name = String::from("");
            let mut formatting_element_attributes = HashMap::new();

            for idx in (0..self.active_formatting_elements.len()).rev() {
                match self.active_formatting_elements[idx] {
                    ActiveElement::Marker => break,
                    ActiveElement::NodeId(node_id) => {
                        let temp_node = self.document.get_node_by_id(node_id).unwrap().clone();
                        if let NodeData::Element {
                            ref name,
                            ref attributes,
                            ..
                        } = temp_node.data
                        {
                            if name == subject && !attributes.is_empty() {
                                formatting_element_idx = idx;
                                formatting_element_id = node_id;
                                formatting_element_name = String::from(name);
                                formatting_element_attributes = attributes.clone();
                            }
                        }
                    }
                }
            }

            if formatting_element_idx == 0 {
                // @TODO: process as any other end tag
                return;
            }

            // Step 4.4
            if !open_elements_has!(self, formatting_element_name) {
                self.parse_error("formatting element not in open elements");
                self.active_formatting_elements
                    .remove(formatting_element_idx);
                return;
            }

            // Step 4.5
            if open_elements_has!(self, formatting_element_name)
                && !self.is_in_scope(&formatting_element_name, Scope::Regular)
            {
                self.parse_error("formatting element not in scope");
                return;
            }

            // Step 4.6
            if formatting_element_id != current_node_id {
                self.parse_error("formatting element not current node");
                // do not return here
            }

            // Step 4.7
            let mut furthest_block_idx = 0;
            let mut furthest_block_id = 0;
            let mut furthest_block_children = Vec::new();

            for idx in (0..formatting_element_idx).rev() {
                match self.active_formatting_elements[idx] {
                    ActiveElement::Marker => {}
                    ActiveElement::NodeId(node_id) => {
                        let node = self.document.get_node_by_id(node_id).unwrap();
                        if node.is_special() {
                            furthest_block_idx = idx;
                            furthest_block_id = node_id;
                            furthest_block_children = self
                                .document
                                .get_node_by_id(furthest_block_id)
                                .expect("Node should exist.")
                                .children
                                .clone();
                        }
                    }
                }
            }

            // Step 4.8
            if furthest_block_idx == 0 {
                while current_node!(self).id != formatting_element_id {
                    self.open_elements.pop();
                }
                self.active_formatting_elements
                    .remove(formatting_element_idx);
                return;
            }

            // Step 4.9
            let common_ancestor_idx = formatting_element_idx - 1;
            let common_ancestor = *self
                .open_elements
                .get(common_ancestor_idx)
                .expect("common ancestor not found");

            // Step 4.10
            let mut bookmark = formatting_element_idx;

            // Step 4.11
            let node_idx = furthest_block_idx;
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

                // Step 4.13.2
                let &node_idx = self
                    .open_elements
                    .get(node_idx - 1)
                    .expect("node not found");
                let node_id = *self.open_elements.get(node_idx).expect("node not found");
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
                let replacement_node_id = self.replace_node(&node, node_idx, common_ancestor);

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
            // Insert whatever last node ended up being in the previous step at the appropriate place for inserting a node, but using common ancestor as the override target.

            // Step 4.15
            let new_element = Node::new_element(
                formatting_element_name.as_str(),
                formatting_element_attributes,
                HTML_NAMESPACE,
            );

            // Step 4.16
            if !furthest_block_children.is_empty() {
                for &child in furthest_block_children.iter() {
                    self.document.append(child, new_element.id)
                }
            }

            // Step 4.17
            let new_element_id = self.document.add_node(new_element, furthest_block_id);

            // Step 4.18
            self.active_formatting_elements
                .remove(formatting_element_idx);
            self.active_formatting_elements
                .insert(bookmark, ActiveElement::NodeId(new_element_id));

            // Step 4.19
            // Remove formatting element from the stack of open elements, and insert the new element into the stack of open elements immediately below the position of furthest block in that stack.
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
}
