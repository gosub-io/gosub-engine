use crate::html5_parser::node::data::element::ElementData;
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

impl<'stream> Html5Parser<'stream> {
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
            let formatting_element_idx_afe = self.find_formatting_element(subject);
            if formatting_element_idx_afe.is_none() {
                return AdoptionResult::ProcessAsAnyOther;
            }

            let mut formatting_element_idx_afe =
                formatting_element_idx_afe.expect("formatting element not found");
            let formatting_element_id = self.active_formatting_elements[formatting_element_idx_afe]
                .node_id()
                .expect("formatting element not found");
            let formatting_element_node = get_node_by_id!(self, formatting_element_id).clone();

            // Step 4.4
            if !self.open_elements_has_id(formatting_element_id) {
                self.parse_error("formatting element not in open elements");
                self.active_formatting_elements
                    .remove(formatting_element_idx_afe);

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
            let furthest_block_idx_oe = self.find_furthest_block_idx(formatting_element_id);

            // Step 4.8
            if furthest_block_idx_oe.is_none() {
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
                self.active_formatting_elements
                    .remove(formatting_element_idx_afe);

                return AdoptionResult::Completed;
            }

            let furthest_block_idx_oe = furthest_block_idx_oe.expect("furthest block not found");
            let furthest_block_id = open_elements_get!(self, furthest_block_idx_oe).id;
            let furthest_block_node = get_node_by_id!(self, furthest_block_id).clone();

            // Step 4.9
            // Find the index of the wanted formatting element id in the open elements stack
            let idx = self.open_elements_find_index(formatting_element_id);
            let common_ancestor_id = *self.open_elements.get(idx - 1).expect("node not found");

            // Step 4.10
            let mut bookmark_afe = formatting_element_idx_afe;

            // Step 4.11
            let mut node_idx_oe = furthest_block_idx_oe;
            let last_node_idx_oe = furthest_block_idx_oe;
            let mut last_node_id = open_elements_get!(self, last_node_idx_oe).id;

            // Step 4.12
            let mut inner_loop_counter = 0;

            // Step 4.13
            loop {
                // Step 4.13.1
                inner_loop_counter += 1;

                // Step 4.13.2
                node_idx_oe -= 1;
                let node_id = open_elements_get!(self, node_idx_oe).id;
                let node = get_node_by_id!(self, node_id).clone();

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
                    let idx_afe = self
                        .active_formatting_elements
                        .iter()
                        .position(|elem| elem == &ActiveElement::Node(node_id))
                        .expect("node not found");
                    self.active_formatting_elements.remove(idx_afe);
                }

                // Step 4.13.5
                if !self
                    .active_formatting_elements
                    .contains(&ActiveElement::Node(node_id))
                {
                    // We have removed the node from the given node_idx
                    self.open_elements.remove(node_idx_oe);
                    continue;
                }

                // Step 4.13.6
                // replace the old node with the new replacement node
                let node_attributes = match node.data {
                    NodeData::Element(ElementData { attributes, .. }) => {
                        attributes.attributes.clone()
                    }
                    _ => HashMap::new(),
                };

                let replacement_node =
                    Node::new_element(&self.document, node.name.as_str(), node_attributes, HTML_NAMESPACE);
                let replacement_node_id = self
                    .document
                    .borrow_mut()
                    .add_node(replacement_node, common_ancestor_id);

                let afe_idx = self
                    .active_formatting_elements
                    .iter()
                    .position(|elem| elem == &ActiveElement::Node(node_id))
                    .expect("node not found");
                self.active_formatting_elements[afe_idx] = ActiveElement::Node(replacement_node_id);
                let idx = self
                    .open_elements
                    .iter()
                    .position(|elem| elem == &node_id)
                    .expect("node not found");
                self.open_elements[idx] = replacement_node_id;

                let node_id = replacement_node_id;

                // Step 4.13.7
                if last_node_id == furthest_block_id {
                    bookmark_afe = afe_idx + 1;
                }

                // Step 4.13.8
                self.document.borrow_mut().relocate(last_node_id, node_id);

                // Step 4.13.9
                last_node_id = node_id;
            }

            // Step 4.14
            self.document
                .borrow_mut()
                .relocate(last_node_id, common_ancestor_id);

            // Step 4.15
            let new_element = match formatting_element_node.data {
                NodeData::Element(ElementData {
                    name, attributes, ..
                }) => {
                    Node::new_element(&self.document, name.as_str(), attributes.attributes.clone(), HTML_NAMESPACE)
                }
                _ => panic!("formatting element is not an element"),
            };

            // Step 4.17
            let new_element_id = self
                .document
                .borrow_mut()
                .add_node(new_element, furthest_block_id);

            // Step 4.16
            for child in furthest_block_node.children.iter() {
                self.document.borrow_mut().relocate(*child, new_element_id);
            }

            // Step 4.18
            // if the bookmark_afe is BEFORE the formatting_elements_idx_afe, then we need to adjust
            // the formatting_element_idx, as we insert a new element and the formatting_element_idx_afe
            // has changed.
            if bookmark_afe < formatting_element_idx_afe {
                formatting_element_idx_afe += 1;
            }

            self.active_formatting_elements
                .insert(bookmark_afe, ActiveElement::Node(new_element_id));
            self.active_formatting_elements
                .remove(formatting_element_idx_afe);

            // Step 4.19
            self.open_elements
                .insert(furthest_block_idx_oe - 1, new_element_id);
            let idx = self.open_elements_find_index(formatting_element_id);
            self.open_elements.remove(idx);
        }
    }

    /// Find the furthest block element in the stack of open elements that is above the formatting element
    fn find_furthest_block_idx(&self, formatting_element_id: NodeId) -> Option<usize> {
        // Find the index of the wanted formatting element id
        let element_idx_oe = self
            .open_elements
            .iter()
            .position(|&element_id| element_id == formatting_element_id);

        let element_idx_oe = match element_idx_oe {
            Some(idx) => idx,
            None => return None,
        };

        // Iterate
        ((element_idx_oe + 1)..self.open_elements.len())
            .find(|&idx| open_elements_get!(self, idx).is_special())
    }

    /// Find the formatting element with the given subject between the end of the list and the first marker (or start when there is no marker)
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
                    let node = get_node_by_id!(self, node_id).clone();
                    if let NodeData::Element(ElementData { name, .. }) = &node.data {
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
            let node = Node::new_element(&$self.document, $name, HashMap::new(), HTML_NAMESPACE);
            let node_id = $self.document.borrow_mut().add_node(node, NodeId::root());
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
            .push(ActiveElement::Node(3.into()));
        parser
            .active_formatting_elements
            .push(ActiveElement::Node(7.into()));

        assert_eq!(
            parser
                .find_furthest_block_idx(1.into())
                .expect("furthest element not found"),
            1
        );
        assert_eq!(
            parser
                .find_furthest_block_idx(2.into())
                .expect("furthest element not found"),
            3
        );
        assert_eq!(
            parser
                .find_furthest_block_idx(3.into())
                .expect("furthest element not found"),
            3
        );
        assert_eq!(
            parser
                .find_furthest_block_idx(4.into())
                .expect("furthest element not found"),
            4
        );
        assert_eq!(
            parser
                .find_furthest_block_idx(5.into())
                .expect("furthest element not found"),
            5
        );
        assert!(parser.find_furthest_block_idx(6.into()).is_none());
        assert!(parser.find_furthest_block_idx(7.into()).is_none());
    }

    #[test]
    fn find_furthest_block_idx_3() {
        let mut stream = InputStream::new();
        let mut parser = Html5Parser::new(&mut stream);

        // Node 0 is the document node
        node_create!(parser, "p"); // node 1
        node_create!(parser, "b");
        node_create!(parser, "i"); // node 3

        assert!(parser.find_furthest_block_idx(2.into()).is_none());
    }

    #[test]
    fn find_furthest_block_idx_4() {
        let mut stream = InputStream::new();
        let mut parser = Html5Parser::new(&mut stream);

        // Node 0 is the document node
        node_create!(parser, "html"); // node 1
        node_create!(parser, "body"); // node 2
        node_create!(parser, "p"); // node 3
        node_create!(parser, "b"); // node 4
        node_create!(parser, "i"); // node 5

        assert_eq!(parser.find_furthest_block_idx(4.into()), None);
        assert_eq!(parser.find_furthest_block_idx(5.into()), None);
        assert_eq!(parser.find_furthest_block_idx(3.into()), None);
        assert_eq!(parser.find_furthest_block_idx(2.into()), Some(2));
    }

    #[test]
    fn find_furthest_block_idx_5() {
        let mut stream = InputStream::new();
        let mut parser = Html5Parser::new(&mut stream);

        // Node 0 is the document node
        node_create!(parser, "html"); // node 1
        node_create!(parser, "body"); // node 2
        node_create!(parser, "b"); // node 3
        node_create!(parser, "p"); // node 4

        assert_eq!(parser.find_furthest_block_idx(3.into()), Some(3));
    }

    #[test]
    fn find_furthest_block_idx_6() {
        let mut stream = InputStream::new();
        let mut parser = Html5Parser::new(&mut stream);

        // Node 0 is the document node
        node_create!(parser, "html"); // node 1
        node_create!(parser, "body"); // node 2
        node_create!(parser, "b"); // node 3
        node_create!(parser, "p"); // node 4
        node_create!(parser, "b");
        node_create!(parser, "b");
        node_create!(parser, "b");
        node_create!(parser, "b"); // 8
        node_create!(parser, "b");
        node_create!(parser, "i");
        node_create!(parser, "p"); // 11
        node_create!(parser, "i");
        node_create!(parser, "b"); // 13

        assert_eq!(parser.find_furthest_block_idx(10.into()), Some(10));
    }
}
