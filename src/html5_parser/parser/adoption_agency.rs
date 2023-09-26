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
            let mut formatting_element_idx: usize = 0;
            let mut formatting_element_id: usize = 0;
            let mut formatting_element_name = String::from("");
            let mut formatting_element_attributes = HashMap::new();

            for idx in (0..self.active_formatting_elements.len()).rev() {
                match self.active_formatting_elements[idx] {
                    ActiveElement::Marker => break,
                    ActiveElement::NodeId(node_id) => {
                        let temp_node = self.document.get_node_by_id(node_id).expect("node not found").clone();
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
                return AdoptionResult::ProcessAsAnyOther
            }

            // Step 4.4
            if !open_elements_has!(self, formatting_element_name) {
                self.parse_error("formatting element not in open elements");
                self.active_formatting_elements
                    .remove(formatting_element_idx);

                return AdoptionResult::Completed
            }

            // Step 4.5
            if open_elements_has!(self, formatting_element_name)
                && !self.is_in_scope(&formatting_element_name, Scope::Regular)
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
                while let Some(top) = self.open_elements.pop() {
                    if top == formatting_element_id {
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
            let node_id = *self.open_elements.get(furthest_block_idx).expect("furthest block not found");
            let furthest_block = self.document.get_node_by_id(node_id).expect("furthest block not found").clone();

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
            if !furthest_block.children.is_empty() {
                for &child in furthest_block.children.iter() {
                    self.document.append(child, new_element.id)
                }
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

    fn find_furthest_block_idx(&self, formatting_element_id: usize) -> Option<usize> {
        let mut index_of_formatting_element = None;

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

        for idx in (0..index_of_formatting_element).rev() {
            let element_id = self.open_elements[idx];
            let element = self.document.get_node_by_id(element_id).expect("element not found");

            if element.is_special() {
                return Some(idx);
            }
        }

        None
    }
}

#[cfg(test)]
mod test {
    use crate::html5_parser::input_stream::{Encoding, InputStream};
    use super::*;

    #[test]
    fn test_adoption_agency() {
        let mut stream = InputStream::new();
        stream.read_from_str("<p>One <b>Two <i>Three</p> Four</i> Five</b> Six</p>", Some(Encoding::UTF8));
        let mut parser = Html5Parser::new(&mut stream);
        parser.parse();

        println!("{}", parser.document);
        // let document = parser.document;
        // let table = document.get_node_by_id(1).unwrap();
        // let tr = document.get_node_by_id(2).unwrap();
        // let td = document.get_node_by_id(3).unwrap();
        // let select = document.get_node_by_id(4).unwrap();
        // let option = document.get_node_by_id(5).unwrap();
        //
        // assert_eq!(table.children, vec![tr.id]);
        // assert_eq!(tr.children, vec![td.id]);
        // assert_eq!(td.children, vec![select.id]);
        // assert_eq!(select.children, vec![option.id]);
    }

}
