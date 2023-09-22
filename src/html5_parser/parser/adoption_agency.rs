use crate::html5_parser::node::{Node, NodeData, NodeId, HTML_NAMESPACE};
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
    /**
     * When we talk about nodes, there are 3 contexts to consider:
     *
     * - The actual node data. This is called "node" in the code.
     * - The node id. This is called "node_id" in the code.
     * - The node index. This is called "node_idx" in the code. This is the index of the node in
     *   either the open_elements or active_formatting_elements stack.
     */
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
                .any(|elem| elem == &ActiveElement::Node(current_node_id))
        {
            self.open_elements.pop();
            return AdoptionResult::Completed;
        }

        // Step 3
        let mut outer_loop_counter = 0;

        // Step 4
        loop {
            // Step 4.1
            if outer_loop_counter >= ADOPTION_AGENCY_OUTER_LOOP_DEPTH {
                return AdoptionResult::Completed;
            }

            // Step 4.2
            outer_loop_counter += 1;

            // Step 4.3
            let formatting_element_idx_in_afe = self.find_formatting_element(subject);
            if formatting_element_idx_in_afe.is_none() {
                return AdoptionResult::ProcessAsAnyOther;
            }

            let formatting_element_idx_in_afe =
                formatting_element_idx_in_afe.expect("formatting element not found");
            let formatting_element_id = self.active_formatting_elements
                [formatting_element_idx_in_afe]
                .node_id()
                .expect("formatting element not found");
            let formatting_element_node = self
                .document
                .get_node_by_id(formatting_element_id)
                .expect("formatting element not found")
                .clone();

            // Step 4.4
            if !open_elements_has_id!(self, formatting_element_id) {
                self.parse_error("formatting element not in open elements");
                self.active_formatting_elements
                    .remove(formatting_element_idx_in_afe);

                return AdoptionResult::Completed;
            }

            // Step 4.5
            if !self.is_in_scope(&formatting_element_node.name, Scope::Regular) {
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
                if let Some(pos) = self
                    .active_formatting_elements
                    .iter()
                    .position(|elem| elem == &ActiveElement::Node(formatting_element_id))
                {
                    self.active_formatting_elements.remove(pos);
                }

                return AdoptionResult::Completed;
            }

            let furthest_block_idx = furthest_block_idx.expect("furthest block not found");
            let furthest_block_id = *self
                .open_elements
                .get(furthest_block_idx)
                .expect("node not found");
            let mut furthest_block_node = self
                .document
                .get_node_by_id(furthest_block_id)
                .expect("node not found")
                .clone();

            // Step 4.9
            let formatting_element_idx_in_oe = self
                .open_elements
                .iter()
                .position(|&id| id == formatting_element_id)
                .expect("formatting element not found");
            let common_ancestor_id = *self
                .open_elements
                .get(formatting_element_idx_in_oe - 1)
                .expect("node not found");

            // Step 4.10
            let mut bookmark = formatting_element_idx_in_afe;

            // Step 4.11
            let last_node_idx = furthest_block_idx;
            let mut last_node_id = *self
                .open_elements
                .get(last_node_idx)
                .expect("last node not found");
            // let last_node = self.document.get_node_by_id(last_node_id).expect("last node not found").clone();

            // let node_id = *self.open_elements.get(furthest_block_idx).expect("node not found");
            let mut node_idx = furthest_block_idx;
            // let node = self.document.get_node_by_id(node_id).expect("node not found").clone();

            // Step 4.12
            let mut inner_loop_counter = 0;

            // Step 4.13
            loop {
                // Step 4.13.1
                inner_loop_counter += 1;

                node_idx -= 1;

                // Step 4.13.2
                let mut node = open_elements_get!(self, node_idx).clone();
                let node_id = self.open_elements[node_idx];

                // Step 4.13.3
                if node_id == formatting_element_id {
                    break;
                }

                // Step 4.13.4
                if inner_loop_counter > ADOPTION_AGENCY_INNER_LOOP_DEPTH
                    && self
                        .active_formatting_elements
                        .contains(&ActiveElement::Node(node_id))
                {
                    self.active_formatting_elements.remove(node_idx);
                    continue;
                }

                // Step 4.13.5
                if !self
                    .active_formatting_elements
                    .contains(&ActiveElement::Node(node_id))
                {
                    // We have removed the node from the given node_idx
                    self.open_elements.remove(node_idx);
                    continue;
                }

                if last_node_id == furthest_block_id {
                    bookmark = node_idx + 1;
                }

                // Step 4.13.6
                // replace the old node with the new replacement node
                let node_attributes = match node.data {
                    NodeData::Element { ref attributes, .. } => attributes.clone(),
                    _ => HashMap::new(),
                };

                let replacement_node =
                    Node::new_element(node.name.as_str(), node_attributes, HTML_NAMESPACE);
                let replacement_node_id =
                    self.document.add_node(replacement_node, common_ancestor_id);

                self.active_formatting_elements[node_idx] =
                    ActiveElement::Node(replacement_node_id);
                self.open_elements[node_idx] = replacement_node_id;

                if node.parent.is_some() {
                    node.parent = None
                }

                let node_id = replacement_node_id;
                // let node = self.document.get_node_by_id(node_id).expect("node not found").clone();

                // Step 4.13.7
                if last_node_idx == furthest_block_idx {
                    bookmark = node_idx - 1;
                }

                // Step 4.13.8
                self.document.append(last_node_id, node_id);

                // Step 4.13.9
                last_node_id = node_id;
            }

            // Step 4.14
            self.document.append(last_node_id, common_ancestor_id);

            // Step 4.15
            let new_element = match formatting_element_node.data {
                NodeData::Element { ref attributes, .. } => Node::new_element(
                    formatting_element_node.name.as_str(),
                    attributes.clone(),
                    HTML_NAMESPACE,
                ),
                _ => panic!("formatting element is not an element"),
            };

            // Step 4.16
            for &child in furthest_block_node.children.iter() {
                self.document.append(child, new_element.id)
            }
            furthest_block_node.children.clear();

            // Step 4.17
            let new_element_id = self.document.add_node(new_element, furthest_block_node.id);

            // Step 4.18
            self.active_formatting_elements
                .remove(formatting_element_idx_in_afe);
            self.active_formatting_elements
                .insert(bookmark, ActiveElement::Node(new_element_id));

            // Step 4.19
            self.open_elements.remove(formatting_element_idx_in_oe);
            self.open_elements
                .insert(furthest_block_idx, new_element_id);
        }
    }

    // Find the furthest block element in the stack of open elements that is above the formatting element
    fn find_furthest_block_idx(&self, formatting_element_id: NodeId) -> Option<usize> {
        let mut formatting_element_idx = None;

        // Find the index of the wanted formatting element id
        for (idx, &element_id) in self.open_elements.iter().enumerate() {
            if element_id == formatting_element_id {
                formatting_element_idx = Some(idx);
                break;
            }
        }

        let formatting_element_idx = match formatting_element_idx {
            Some(idx) => idx,
            None => return None,
        };

        // Iterate
        for idx in (0..formatting_element_idx).rev() {
            let element_id = self.open_elements[idx];
            let element = self
                .document
                .get_node_by_id(element_id)
                .expect("element not found");

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
                }
                ActiveElement::Node(node_id) => {
                    // Check if the given node is an element with the given subject
                    let node = self
                        .document
                        .get_node_by_id(node_id)
                        .expect("node not found")
                        .clone();
                    if let NodeData::Element { ref name, .. } = node.data {
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

#[cfg(test)]
mod test {
    use super::*;
    use crate::html5_parser::input_stream::InputStream;

    macro_rules! node_create {
        ($self:expr, $name:expr) => {{
            let node = Node::new_element($name, HashMap::new(), HTML_NAMESPACE);
            let node_id = $self.document.add_node(node, NodeId::root());
            $self.open_elements.push(node_id);
        }};
    }

    #[test]
    fn test_find_furthest_block_idx_1() {
        let mut stream = InputStream::new();
        let mut parser = Html5Parser::new(&mut stream);

        // Node 0 is the document node
        node_create!(parser, "div"); // node 1
        node_create!(parser, "ul");
        node_create!(parser, "b");
        node_create!(parser, "li");
        node_create!(parser, "p");
        node_create!(parser, "table");
        node_create!(parser, "i"); // node 7

        parser
            .active_formatting_elements
            .push(ActiveElement::Node(NodeId(3)));
        parser
            .active_formatting_elements
            .push(ActiveElement::Node(NodeId(7)));

        assert!(parser.find_furthest_block_idx(NodeId(1)).is_none());
        assert_eq!(
            parser
                .find_furthest_block_idx(NodeId(2))
                .expect("furthest element not found"),
            0
        );
        assert_eq!(
            parser
                .find_furthest_block_idx(NodeId(3))
                .expect("furthest element not found"),
            1
        );
        assert_eq!(
            parser
                .find_furthest_block_idx(NodeId(4))
                .expect("furthest element not found"),
            1
        );
        assert_eq!(
            parser
                .find_furthest_block_idx(NodeId(5))
                .expect("furthest element not found"),
            3
        );
        assert_eq!(
            parser
                .find_furthest_block_idx(NodeId(6))
                .expect("furthest element not found"),
            4
        );
        assert_eq!(
            parser
                .find_furthest_block_idx(NodeId(7))
                .expect("furthest element not found"),
            5
        );
    }

    #[test]
    fn test_find_furthest_block_idx_2() {
        let mut stream = InputStream::new();
        let mut parser = Html5Parser::new(&mut stream);

        // Node 0 is the document node
        node_create!(parser, "div"); // node 1
        node_create!(parser, "ul");
        node_create!(parser, "b");
        node_create!(parser, "span");
        node_create!(parser, "li");
        node_create!(parser, "a");
        node_create!(parser, "p");
        node_create!(parser, "table");
        node_create!(parser, "i"); // node 9

        parser
            .active_formatting_elements
            .push(ActiveElement::Node(NodeId(3)));
        parser
            .active_formatting_elements
            .push(ActiveElement::Node(NodeId(9)));

        assert!(parser.find_furthest_block_idx(NodeId(1)).is_none());
        assert_eq!(
            parser
                .find_furthest_block_idx(NodeId(2))
                .expect("furthest element not found"),
            0
        );
        assert_eq!(
            parser
                .find_furthest_block_idx(NodeId(3))
                .expect("furthest element not found"),
            1
        );
        assert_eq!(
            parser
                .find_furthest_block_idx(NodeId(4))
                .expect("furthest element not found"),
            1
        );
        assert_eq!(
            parser
                .find_furthest_block_idx(NodeId(5))
                .expect("furthest element not found"),
            1
        );
        assert_eq!(
            parser
                .find_furthest_block_idx(NodeId(6))
                .expect("furthest element not found"),
            4
        );
        assert_eq!(
            parser
                .find_furthest_block_idx(NodeId(7))
                .expect("furthest element not found"),
            4
        );
        assert_eq!(
            parser
                .find_furthest_block_idx(NodeId(8))
                .expect("furthest element not found"),
            6
        );
        assert_eq!(
            parser
                .find_furthest_block_idx(NodeId(9))
                .expect("furthest element not found"),
            7
        );
    }
}
