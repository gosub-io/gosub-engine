use crate::element_class::ElementClass;
use crate::node::data::text::TextData;
use crate::node::{Node, NodeData, NodeId, HTML_NAMESPACE};
use crate::parser::{ActiveElement, Html5Parser, Scope};
use crate::tokenizer::token::Token;
use std::collections::HashMap;

use super::document::{Document, DocumentHandle};

const ADOPTION_AGENCY_OUTER_LOOP_DEPTH: usize = 8;
const ADOPTION_AGENCY_INNER_LOOP_DEPTH: usize = 3;

#[derive(Debug)]
pub enum InsertionPositionMode<NodeId> {
    LastChild {
        handle: DocumentHandle,
        parent: NodeId,
    },
    Sibling {
        handle: DocumentHandle,
        parent: NodeId,
        before: NodeId,
    },
}

pub enum BookMark<NodeId> {
    Replace(NodeId),
    InsertAfter(NodeId),
}

impl Html5Parser<'_> {
    fn find_position_in_active_format(&self, node_id: NodeId) -> Option<usize> {
        self.active_formatting_elements
            .iter()
            .position(|&x| x == ActiveElement::Node(node_id))
    }

    fn find_position_in_open_element(&self, node_id: NodeId) -> Option<usize> {
        self.open_elements.iter().position(|&x| x == node_id)
    }

    fn find_format_element_index(&self, subject: &str) -> Option<(usize, NodeId)> {
        self.active_formatting_elements
            .iter()
            .enumerate()
            .rev()
            .find_map(|(i, &node_id)| {
                if let ActiveElement::Node(node_id) = node_id {
                    if get_node_by_id!(self.document, node_id).name == subject {
                        Some((i, node_id))
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
    }

    fn find_further_block_index(&self, format_ele_position: usize) -> Option<(usize, NodeId)> {
        self.open_elements
            .iter()
            .enumerate()
            .skip(format_ele_position)
            .find_map(|(i, &node_id)| {
                if get_node_by_id!(self.document, node_id).is_special() {
                    Some((i, node_id))
                } else {
                    None
                }
            })
    }

    pub fn insert_element_helper(&mut self, node: NodeId, position: InsertionPositionMode<NodeId>) {
        match position {
            InsertionPositionMode::Sibling {
                handle,
                parent,
                before,
            } => {
                let mut doc = handle;
                let parent_node = get_node_by_id!(doc, parent);
                let position = parent_node.children.iter().position(|&x| x == before);
                doc.attach_node_to_parent(node, parent, position);
            }
            InsertionPositionMode::LastChild { handle, parent } => {
                let mut doc = handle;
                doc.attach_node_to_parent(node, parent, None);
            }
        }
    }

    pub fn insert_text_helper(&mut self, position: InsertionPositionMode<NodeId>, token: &Token) {
        match position {
            InsertionPositionMode::Sibling {
                handle,
                parent,
                before,
            } => {
                let mut doc = handle;
                let parent_node = get_node_by_id!(doc, parent);
                let position = parent_node.children.iter().position(|&x| x == before);
                match position {
                    None | Some(0) => {
                        let node = self.create_node(token, HTML_NAMESPACE);
                        doc.add_node(node, parent, position);
                    }
                    Some(index) => {
                        let last_node_id = parent_node.children[index - 1];
                        if let NodeData::Text(TextData { ref mut value, .. }) = doc
                            .get_mut()
                            .get_node_by_id_mut(last_node_id)
                            .expect("node not found")
                            .data
                        {
                            value.push_str(&token.to_string());
                            return;
                        };

                        let node = self.create_node(token, HTML_NAMESPACE);
                        doc.add_node(node, parent, Some(index));
                    }
                }
            }
            InsertionPositionMode::LastChild { handle, parent } => {
                let mut doc = handle;
                let parent_node = get_node_by_id!(doc, parent);
                if let Some(last_node_id) = parent_node.children.last() {
                    if let NodeData::Text(TextData { ref mut value, .. }) = self
                        .document
                        .get_mut()
                        .get_node_by_id_mut(*last_node_id)
                        .expect("node not found")
                        .data
                    {
                        value.push_str(&token.to_string());
                        return;
                    };
                    let node = self.create_node(token, HTML_NAMESPACE);
                    doc.add_node(node, parent, None);
                    return;
                }

                let node = self.create_node(token, HTML_NAMESPACE);
                doc.add_node(node, parent, None);
            }
        }
    }

    pub fn insert_html_element(&mut self, token: &Token) -> NodeId {
        self.insert_element_from_token(token, None, Some(HTML_NAMESPACE))
    }

    pub fn insert_foreign_element(&mut self, token: &Token, namespace: &str) -> NodeId {
        self.insert_element_from_token(token, None, Some(namespace))
    }

    pub fn insert_element_from_token(
        &mut self,
        token: &Token,
        override_node: Option<NodeId>,
        namespace: Option<&str>,
    ) -> NodeId {
        let mut node = self.create_node(token, namespace.unwrap_or(HTML_NAMESPACE));
        // add CSS classes from class attribute in element
        // e.g., <div class="one two three">
        // TODO: this will be refactored later in ElementAttributes to do this
        // when inserting a "class" attribute. Similar to "id" to attach it to the DOM
        // named_id_list. Although this will require some shared pointers

        if let NodeData::Element(ref mut element) = node.data {
            if element.attributes.contains_key("class") {
                if let Some(class_string) = element.attributes.get("class") {
                    element.classes = ElementClass::from(class_string.as_str());
                }
            }
        }
        self.insert_element(node, override_node)
    }

    pub fn insert_element_from_node(
        &mut self,
        org_node: &Node,
        override_node: Option<NodeId>,
    ) -> NodeId {
        // Create a node, but without children and push it onto the open elements stack (if needed)
        let mut new_node = org_node.clone();
        new_node.children = Vec::new();
        new_node.parent = None;
        new_node.is_registered = false;

        if let NodeData::Element(ref mut element) = new_node.data {
            if element.attributes.contains_key("class") {
                if let Some(class_string) = element.attributes.get("class") {
                    element.classes = ElementClass::from(class_string.as_str());
                }
            }
        }
        self.insert_element(new_node, override_node)
    }

    pub fn insert_element(&mut self, node: Node, override_node: Option<NodeId>) -> NodeId {
        let node_id = self.document.get_mut().add_new_node(node);
        let insert_position = self.appropriate_place_insert(override_node);
        self.insert_element_helper(node_id, insert_position);

        //     if parser not created as part of html fragment parsing algorithm
        //       pop the top element queue from the relevant agent custom element reactions stack (???)

        // push element onto the stack of open elements so that is the new current node
        self.open_elements.push(node_id);

        // return element
        node_id
    }

    pub fn insert_doctype_element(&mut self, token: &Token) {
        let node = self.create_node(token, HTML_NAMESPACE);
        self.document.get_mut().add_node(node, NodeId::root(), None);
    }

    pub fn insert_document_element(&mut self, token: &Token) {
        let node = self.create_node(token, HTML_NAMESPACE);
        let node_id = self.document.get_mut().add_node(node, NodeId::root(), None);
        self.open_elements.push(node_id);
    }

    pub fn insert_comment_element(&mut self, token: &Token, insert_position: Option<NodeId>) {
        let node = self.create_node(token, HTML_NAMESPACE);
        if let Some(position) = insert_position {
            self.document.get_mut().add_node(node, position, None);
        } else {
            let node_id = self.document.get_mut().add_new_node(node);
            let insert_position = self.appropriate_place_insert(None);
            self.insert_element_helper(node_id, insert_position);
        }
    }

    pub fn insert_text_element(&mut self, token: &Token) {
        // Skip empty text nodes
        if let Token::Text { text, .. } = token {
            if text.is_empty() {
                return;
            }
        }

        let insertion_position = self.appropriate_place_insert(None);
        // TODO, for text element, if the insertion_position is Document, should not do next step.
        self.insert_text_helper(insertion_position, token);
    }

    // @todo: where is the fragment case handled? (substep 4: https://html.spec.whatwg.org/multipage/parsing.html#appropriate-place-for-inserting-a-node)
    pub fn appropriate_place_insert(
        &self,
        override_node: Option<NodeId>,
    ) -> InsertionPositionMode<NodeId> {
        let current_node_id = current_node!(self).id;
        let target_id = override_node.unwrap_or(current_node_id);
        let target_node = get_node_by_id!(self.document, target_id);
        if !(self.foster_parenting
            && ["table", "tbody", "thead", "tfoot", "tr"].contains(&target_node.name.as_str()))
        {
            if target_node.name == "template" && target_node.is_namespace(HTML_NAMESPACE) {
                if let NodeData::Element(element) = target_node.data {
                    if let Some(template_contents) = element.template_contents {
                        return InsertionPositionMode::LastChild {
                            handle: Document::clone(&template_contents.doc),
                            parent: target_id,
                        };
                    }
                }
            } else {
                return InsertionPositionMode::LastChild {
                    handle: Document::clone(&self.document),
                    parent: target_id,
                };
            }
        }
        let mut iter = self.open_elements.iter().rev().peekable();
        while let Some(node_id) = iter.next() {
            let node = get_node_by_id!(self.document, *node_id);
            if node.name == "template" {
                if let NodeData::Element(element) = node.data {
                    if let Some(template_contents) = &element.template_contents {
                        return InsertionPositionMode::LastChild {
                            handle: Document::clone(&template_contents.doc),
                            parent: *node_id,
                        };
                    }
                }
            } else if node.name == "table" {
                if node.parent.is_some() {
                    return InsertionPositionMode::Sibling {
                        handle: Document::clone(&self.document),
                        parent: node.parent.unwrap(),
                        before: *node_id,
                    };
                }
                // TODO has some question? can reached?
                return InsertionPositionMode::LastChild {
                    handle: Document::clone(&self.document),
                    parent: *(*iter.peek().unwrap()),
                };
            }
        }
        return InsertionPositionMode::LastChild {
            handle: Document::clone(&self.document),
            parent: *self.open_elements.first().unwrap(),
        };
    }

    pub fn adoption_agency_algorithm(&mut self, token: &Token) {
        // step 1
        let subject = match token {
            Token::StartTag { name, .. } | Token::EndTag { name, .. } => name,
            _ => panic!("un reached"),
        };
        let current_node = current_node!(self);
        let current_node_id = current_node.id;

        // step 2
        if current_node.name == *subject
            && current_node.is_namespace(HTML_NAMESPACE)
            && self
                .find_position_in_active_format(current_node_id)
                .is_none()
        {
            self.open_elements.pop();
            return;
        }

        // step 3
        let mut outer_loop_counter = 0;

        // step 4
        loop {
            // step 4.1
            if outer_loop_counter >= ADOPTION_AGENCY_OUTER_LOOP_DEPTH {
                return;
            }

            // step 4.2
            outer_loop_counter += 1;

            // step 4.3
            let (format_elem_idx, format_elem_node_id) =
                match self.find_format_element_index(subject) {
                    None => {
                        return self.handle_in_body_any_other_end_tag(subject);
                    }
                    Some((idx, node_id)) => (idx, node_id),
                };
            let format_elem_node = get_node_by_id!(self.document, format_elem_node_id);
            let format_ele_stack_position = match self
                .open_elements
                .iter()
                .rposition(|&x| x == format_elem_node_id)
            {
                // step 4.4
                None => {
                    self.parse_error("not found format_element_node in open_elements");
                    self.active_formatting_elements.remove(format_elem_idx);
                    return;
                }
                Some(idx) => idx,
            };

            // step 4.5
            if !self.is_in_scope(&format_elem_node.name, HTML_NAMESPACE, Scope::Regular) {
                self.parse_error("format_element_node not in regular scope");
                return;
            }

            // step 4.6
            if format_elem_node_id != current_node_id {
                self.parse_error("format_element_node not current_node");
            }

            // step 4.7
            let (further_block_idx, further_block_node_id) =
                match self.find_further_block_index(format_ele_stack_position) {
                    // step 4.8
                    None => {
                        self.open_elements.truncate(format_ele_stack_position);
                        self.active_formatting_elements.remove(format_elem_idx);
                        return;
                    }
                    Some((idx, node_id)) => (idx, node_id),
                };

            // step 4.9
            let common_ancestor = self.open_elements[format_ele_stack_position - 1];

            // step 4.10
            let mut bookmark_node_id = BookMark::Replace(format_elem_node_id);

            // step 4.11
            let mut node_id;
            let mut last_node_id = further_block_node_id;
            let mut node_idx = further_block_idx;

            // step 4.12
            let mut inner_loop_counter = 0;

            // step 4.13
            loop {
                // step 4.13.1
                inner_loop_counter += 1;

                // step 4.13.2
                node_idx -= 1;
                node_id = self.open_elements[node_idx];

                // step 4.13.3
                if node_id == format_elem_node_id {
                    break;
                }

                // step 4.13.4
                if inner_loop_counter > ADOPTION_AGENCY_INNER_LOOP_DEPTH {
                    self.find_position_in_active_format(node_id)
                        .map(|position| self.active_formatting_elements.remove(position));
                    self.open_elements.remove(node_idx);
                    continue;
                }
                // step 4.13.5
                let Some(node_active_position) = self.find_position_in_active_format(node_id)
                else {
                    self.open_elements.remove(node_idx);
                    continue;
                };

                // step 4.13.6
                let element = get_node_by_id!(self.document, node_id);
                let node_attributes = match element.data {
                    NodeData::Element(element) => element.attributes.clone(),
                    _ => HashMap::new(),
                };
                let replacement_node = Node::new_element(
                    &self.document,
                    &element.name,
                    node_attributes,
                    HTML_NAMESPACE,
                    element.location.clone(),
                );
                let replace_node_id = self.document.get_mut().add_new_node(replacement_node);

                self.active_formatting_elements[node_active_position] =
                    ActiveElement::Node(replace_node_id);

                self.open_elements[node_idx] = replace_node_id;

                node_id = replace_node_id;

                // step 4.13.7
                if last_node_id == further_block_node_id {
                    bookmark_node_id = BookMark::InsertAfter(node_id);
                }

                // step 4.13.8
                self.document.detach_node_from_parent(last_node_id);
                self.document
                    .attach_node_to_parent(last_node_id, replace_node_id, None);

                // step 4.13.9
                last_node_id = node_id;
            }

            // step 4.14
            self.document.detach_node_from_parent(last_node_id);
            let insert_position = self.appropriate_place_insert(Some(common_ancestor));
            self.insert_element_helper(last_node_id, insert_position);

            // step 4.15
            let format_elem_attributes = match format_elem_node.data {
                NodeData::Element(element) => element.attributes.clone(),
                _ => HashMap::new(),
            };
            let new_format_node: Node = Node::new_element(
                &self.document,
                &format_elem_node.name,
                format_elem_attributes,
                HTML_NAMESPACE,
                format_elem_node.location.clone(),
            );

            // step 4.16
            let new_node_id = self
                .document
                .get_mut()
                .add_new_node(new_format_node.clone());
            let further_block_node = get_node_by_id!(self.document, further_block_node_id);
            for child in &further_block_node.children {
                self.document.get_mut().relocate(*child, new_node_id);
            }

            // step 4.17
            self.document
                .get_mut()
                .attach_node_to_parent(new_node_id, further_block_node_id, None);

            // step 4.18
            match bookmark_node_id {
                BookMark::Replace(current) => {
                    let index = self
                        .find_position_in_active_format(current)
                        .expect("node not found");
                    self.active_formatting_elements[index] = ActiveElement::Node(new_node_id);
                }
                BookMark::InsertAfter(previous) => {
                    let index = self
                        .find_position_in_active_format(previous)
                        .expect("node not foudn")
                        + 1;
                    self.active_formatting_elements
                        .insert(index, ActiveElement::Node(new_node_id));
                    let position = self.find_position_in_active_format(format_elem_node_id);
                    self.active_formatting_elements.remove(position.unwrap());
                }
            }

            // step 4.19
            self.open_elements.retain(|x| x != &format_elem_node_id);
            let position = self
                .find_position_in_open_element(further_block_node_id)
                .unwrap();
            self.open_elements.insert(position + 1, new_node_id);
        }
    }
}
