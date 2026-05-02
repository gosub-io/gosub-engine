use crate::node::elements::{SPECIAL_HTML_ELEMENTS, SPECIAL_MATHML_ELEMENTS, SPECIAL_SVG_ELEMENTS};
use crate::node::{HTML_NAMESPACE, MATHML_NAMESPACE, SVG_NAMESPACE};
use crate::parser::{ActiveElement, Html5Parser, Scope};
use crate::tokenizer::token::Token;
use gosub_interface::config::HasDocument;
use gosub_interface::document::Document;
use gosub_shared::node::NodeId;

const ADOPTION_AGENCY_OUTER_LOOP_DEPTH: usize = 8;
const ADOPTION_AGENCY_INNER_LOOP_DEPTH: usize = 3;

#[derive(Debug)]
pub enum InsertionPositionMode<NodeId> {
    LastChild { parent_id: NodeId },
    Sibling { parent_id: NodeId, before_id: NodeId },
}

pub enum BookMark<NodeId> {
    Replace(NodeId),
    InsertAfter(NodeId),
}

// ── free helper functions (work purely from the Document trait) ─────────────

/// Returns true when the element identified by `id` is a "special" element
/// (as defined in the HTML parsing spec).
pub(crate) fn is_special<C: HasDocument>(doc: &C::Document, id: NodeId) -> bool {
    let tag = doc.tag_name(id).unwrap_or_default();
    let ns = doc.namespace(id);
    match ns {
        Some(ns) if ns == HTML_NAMESPACE => SPECIAL_HTML_ELEMENTS.contains(&tag),
        Some(ns) if ns == MATHML_NAMESPACE => SPECIAL_MATHML_ELEMENTS.contains(&tag),
        Some(ns) if ns == SVG_NAMESPACE => SPECIAL_SVG_ELEMENTS.contains(&tag),
        None => SPECIAL_HTML_ELEMENTS.contains(&tag), // treat None as HTML
        _ => false,
    }
}

/// Returns true when the element is a MathML text integration point.
pub(crate) fn is_mathml_integration_point<C: HasDocument>(doc: &C::Document, id: NodeId) -> bool {
    let ns = doc.namespace(id).unwrap_or_default();
    let tag = doc.tag_name(id).unwrap_or_default();
    ns == MATHML_NAMESPACE && ["mi", "mo", "mn", "ms", "mtext"].contains(&tag)
}

/// Returns true when the element is an HTML integration point.
pub(crate) fn is_html_integration_point<C: HasDocument>(doc: &C::Document, id: NodeId) -> bool {
    let ns = doc.namespace(id).unwrap_or_default();
    let tag = doc.tag_name(id).unwrap_or_default();
    if ns == MATHML_NAMESPACE && tag == "annotation-xml" {
        let encoding = doc.attribute(id, "encoding").unwrap_or_default();
        return encoding.eq_ignore_ascii_case("text/html") || encoding.eq_ignore_ascii_case("application/xhtml+xml");
    }
    ns == SVG_NAMESPACE && ["foreignObject", "desc", "title"].contains(&tag)
}

/// Returns true when node `a` and node `b` have the same tag name, namespace, and attributes
/// (order-independent).
pub(crate) fn matches_tag_and_attrs_without_order<C: HasDocument>(doc: &C::Document, a: NodeId, b: NodeId) -> bool {
    let tag_a = doc.tag_name(a);
    let tag_b = doc.tag_name(b);
    if tag_a != tag_b {
        return false;
    }
    let ns_a = doc.namespace(a);
    let ns_b = doc.namespace(b);
    if ns_a != ns_b {
        return false;
    }
    doc.attributes(a) == doc.attributes(b)
}

// ── impl block ──────────────────────────────────────────────────────────────

impl<C: HasDocument> Html5Parser<'_, C> {
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
            .find_map(|(i, &elem)| {
                if let ActiveElement::Node(node_id) = elem {
                    if self.document.tag_name(node_id).unwrap_or_default() == subject {
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
                if is_special::<C>(self.document, node_id) {
                    Some((i, node_id))
                } else {
                    None
                }
            })
    }

    pub fn insert_element_helper(&mut self, node_id: NodeId, position: InsertionPositionMode<NodeId>) {
        match position {
            InsertionPositionMode::Sibling { parent_id, before_id } => {
                let pos = self.document.children(parent_id).iter().position(|&x| x == before_id);
                self.document.attach(node_id, parent_id, pos);
            }
            InsertionPositionMode::LastChild { parent_id } => {
                self.document.attach(node_id, parent_id, None);
            }
        }
    }

    pub fn insert_text_helper(&mut self, position: InsertionPositionMode<NodeId>, token: &Token) {
        match position {
            InsertionPositionMode::Sibling { parent_id, before_id } => {
                let children: Vec<NodeId> = self.document.children(parent_id).to_vec();
                let pos = children.iter().position(|&x| x == before_id);
                match pos {
                    None | Some(0) => {
                        let node_id = self.create_node(token, HTML_NAMESPACE);
                        self.document.attach(node_id, parent_id, pos);
                    }
                    Some(index) => {
                        let last_node_id = children[index - 1];
                        if self.document.text_value(last_node_id).is_some() {
                            let cur = self.document.text_value(last_node_id).unwrap_or_default().to_owned();
                            self.document.set_text_value(last_node_id, &(cur + &token.to_string()));
                            return;
                        }
                        let node_id = self.create_node(token, HTML_NAMESPACE);
                        self.document.attach(node_id, parent_id, Some(index));
                    }
                }
            }
            InsertionPositionMode::LastChild { parent_id } => {
                let last_child_id = self.document.children(parent_id).last().copied();
                if let Some(last_node_id) = last_child_id {
                    if self.document.text_value(last_node_id).is_some() {
                        let cur = self.document.text_value(last_node_id).unwrap_or_default().to_owned();
                        self.document.set_text_value(last_node_id, &(cur + &token.to_string()));
                        return;
                    }
                }
                let node_id = self.create_node(token, HTML_NAMESPACE);
                self.document.attach(node_id, parent_id, None);
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
        let node_id = self.create_node(token, namespace.unwrap_or(HTML_NAMESPACE));
        self.insert_element(node_id, override_node)
    }

    pub fn insert_element_from_node(&mut self, org_node_id: NodeId, override_node: Option<NodeId>) -> NodeId {
        let new_node_id = self.document.duplicate_node(org_node_id);
        self.insert_element(new_node_id, override_node)
    }

    pub fn insert_element(&mut self, node_id: NodeId, override_node: Option<NodeId>) -> NodeId {
        let insert_position = self.appropriate_place_insert(override_node);
        self.insert_element_helper(node_id, insert_position);

        //     if parser not created as part of html fragment parsing algorithm
        //       pop the top element queue from the relevant agent custom element reactions stack (???)

        self.open_elements.push(node_id);
        node_id
    }

    pub fn insert_doctype_element(&mut self, token: &Token) {
        let node_id = self.create_node(token, HTML_NAMESPACE);
        self.document.attach(node_id, NodeId::root(), None);
    }

    pub fn insert_document_element(&mut self, token: &Token) {
        let node_id = self.create_node(token, HTML_NAMESPACE);
        self.document.attach(node_id, NodeId::root(), None);
        self.open_elements.push(node_id);
    }

    pub fn insert_comment_element(&mut self, token: &Token, insert_position: Option<NodeId>) {
        let node_id = self.create_node(token, HTML_NAMESPACE);
        if let Some(position) = insert_position {
            self.document.attach(node_id, position, None);
            return;
        }

        let insert_position = self.appropriate_place_insert(None);
        self.insert_element_helper(node_id, insert_position);
    }

    pub fn insert_text_element(&mut self, token: &Token) {
        if let Token::Text { text, .. } = token {
            if text.is_empty() {
                return;
            }
        }

        let insertion_position = self.appropriate_place_insert(None);
        self.insert_text_helper(insertion_position, token);
    }

    // Implements the "appropriate place for inserting a node" algorithm.
    // https://html.spec.whatwg.org/multipage/parsing.html#appropriate-place-for-inserting-a-node
    //
    // The "no last table" sub-step (fragment case) is handled by the fallthrough at the
    // bottom of this function, which returns the first element in the open elements stack.
    pub fn appropriate_place_insert(&self, override_node: Option<NodeId>) -> InsertionPositionMode<NodeId> {
        let current_node_id = *self.open_elements.last().unwrap_or(&NodeId::root());
        let target_id = override_node.unwrap_or(current_node_id);

        let target_tag = self.document.tag_name(target_id).unwrap_or_default();
        let target_ns = self.document.namespace(target_id);

        if !(self.foster_parenting && ["table", "tbody", "thead", "tfoot", "tr"].contains(&target_tag)) {
            if target_tag == "template" && target_ns == Some(HTML_NAMESPACE) {
                // Spec step 3: redirect inside template to its template contents.
                if let Some(contents_id) = self.document.template_contents(target_id) {
                    return InsertionPositionMode::LastChild { parent_id: contents_id };
                }
            } else {
                return InsertionPositionMode::LastChild { parent_id: target_id };
            }
        }

        // Foster-parenting path: scan open elements from top, looking for the last
        // template or table element (whichever is more recently pushed).
        let mut iter = self.open_elements.iter().rev().peekable();
        while let Some(node_id) = iter.next() {
            let node_tag = self.document.tag_name(*node_id).unwrap_or_default();

            if node_tag == "template" {
                // Spec step 3: redirect inside template to its template contents.
                if let Some(contents_id) = self.document.template_contents(*node_id) {
                    return InsertionPositionMode::LastChild { parent_id: contents_id };
                }
            } else if node_tag == "table" {
                if let Some(parent_id) = self.document.parent(*node_id) {
                    return InsertionPositionMode::Sibling {
                        parent_id,
                        before_id: *node_id,
                    };
                }
                return InsertionPositionMode::LastChild {
                    parent_id: *(*iter.peek().unwrap()),
                };
            }
        }
        // No table found: use the first element in the stack (the html element, or in the
        // fragment case, the context element). This covers spec sub-step 4.
        InsertionPositionMode::LastChild {
            parent_id: *self.open_elements.first().unwrap(),
        }
    }

    pub fn adoption_agency_algorithm(&mut self, token: &Token) {
        // step 1
        let subject = match token {
            Token::StartTag { name, .. } | Token::EndTag { name, .. } => name,
            _ => panic!("un reached"),
        };
        let current_node_id = *self.open_elements.last().unwrap_or(&NodeId::root());
        let current_tag = self.document.tag_name(current_node_id).unwrap_or_default();
        let current_ns = self.document.namespace(current_node_id);

        // step 2
        if current_tag == subject.as_str()
            && current_ns == Some(HTML_NAMESPACE)
            && self.find_position_in_active_format(current_node_id).is_none()
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
            let (format_elem_idx, format_elem_node_id) = match self.find_format_element_index(subject) {
                None => {
                    return self.handle_in_body_any_other_end_tag(subject);
                }
                Some((idx, node_id)) => (idx, node_id),
            };

            let format_tag = self
                .document
                .tag_name(format_elem_node_id)
                .unwrap_or_default()
                .to_owned();
            let format_ns = self.document.namespace(format_elem_node_id).map(|s| s.to_owned());
            let format_attrs = self
                .document
                .attributes(format_elem_node_id)
                .cloned()
                .unwrap_or_default();

            let format_ele_stack_position = match self.open_elements.iter().rposition(|&x| x == format_elem_node_id) {
                // step 4.4
                None => {
                    self.parse_error("not found format_element_node in open_elements");
                    self.active_formatting_elements.remove(format_elem_idx);
                    return;
                }
                Some(idx) => idx,
            };

            // step 4.5
            if !self.is_in_scope(&format_tag, HTML_NAMESPACE, Scope::Regular) {
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
                let Some(node_active_position) = self.find_position_in_active_format(node_id) else {
                    self.open_elements.remove(node_idx);
                    continue;
                };

                // step 4.13.6 — duplicate the node (shallow copy) using trait methods
                let replace_node_id = self.document.duplicate_node(node_id);

                self.active_formatting_elements[node_active_position] = ActiveElement::Node(replace_node_id);
                self.open_elements[node_idx] = replace_node_id;

                node_id = replace_node_id;

                // step 4.13.7
                if last_node_id == further_block_node_id {
                    bookmark_node_id = BookMark::InsertAfter(node_id);
                }

                // step 4.13.8
                self.document.detach(last_node_id);
                self.document.attach(last_node_id, replace_node_id, None);

                // step 4.13.9
                last_node_id = node_id;
            }

            // step 4.14
            self.document.detach(last_node_id);
            let insert_position = self.appropriate_place_insert(Some(common_ancestor));
            self.insert_element_helper(last_node_id, insert_position);

            // step 4.15 — create new format node as shallow copy of format_elem
            let new_node_id = self.document.create_element(
                &format_tag,
                format_ns.as_deref(),
                format_attrs,
                gosub_shared::byte_stream::Location::default(),
            );

            // step 4.16 — move children of further_block to new_node
            let children: Vec<NodeId> = self.document.children(further_block_node_id).to_vec();
            for child in children {
                self.document.relocate_node(child, new_node_id);
            }

            // step 4.17
            self.document.attach(new_node_id, further_block_node_id, None);

            // step 4.18
            match bookmark_node_id {
                BookMark::Replace(current) => {
                    let index = self.find_position_in_active_format(current).expect("node not found");
                    self.active_formatting_elements[index] = ActiveElement::Node(new_node_id);
                }
                BookMark::InsertAfter(previous) => {
                    let index = self.find_position_in_active_format(previous).expect("node not found") + 1;
                    self.active_formatting_elements
                        .insert(index, ActiveElement::Node(new_node_id));
                    let position = self.find_position_in_active_format(format_elem_node_id);
                    self.active_formatting_elements.remove(position.unwrap());
                }
            }

            // step 4.19
            self.open_elements.retain(|x| x != &format_elem_node_id);
            let position = self.find_position_in_open_element(further_block_node_id).unwrap();
            self.open_elements.insert(position + 1, new_node_id);
        }
    }
}
