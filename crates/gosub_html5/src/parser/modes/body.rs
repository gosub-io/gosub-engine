//! The in body insertion mode.

use crate::node::{HTML_NAMESPACE, MATHML_NAMESPACE, SVG_NAMESPACE};
use crate::parser::helper::is_special;
use crate::parser::{Html5Parser, InsertionMode, Scope};
use crate::tokenizer::state::State;
use crate::tokenizer::token::Token;
use cow_utils::CowUtils;
use gosub_interface::config::HasDocument;
use gosub_interface::document::Document;
use gosub_interface::node::NodeType;
use gosub_interface::node::QuirksMode;
use gosub_shared::attributes::Attributes;
use gosub_shared::node::NodeId;

impl<C: HasDocument> Html5Parser<'_, C> {
    /// Handle insertion mode "`in_body`"
    pub(crate) fn handle_in_body(&mut self, token: &mut Token) {
        match &*token {
            Token::Text { text: value, location } if token.is_mixed_null() => {
                let tokens = self.split_mixed_token_null(value, *location);
                self.tokenizer.insert_tokens_at_queue_start(tokens);
            }
            Token::Text { text: value, location } if token.is_null() => {
                self.null_parse_errors(value, *location, "null character not allowed in in body insertion mode");
                // ignore token
            }
            Token::Text { .. } => {
                self.reconstruct_formatting();

                self.insert_text_element(token);

                // If this mixed token does not have whitespace chars, set frameset_ok to false
                if !token.is_empty_or_white() {
                    self.frameset_ok = false;
                }
            }
            Token::Comment { .. } => {
                self.insert_comment_element(token, None);
            }
            Token::DocType { .. } => {
                self.parse_error("doctype not allowed in in body insertion mode");
                // ignore token
            }
            Token::StartTag { name, attributes, .. } if name == "html" => {
                self.parse_error("html tag not allowed in in body insertion mode");

                if self.open_elements_has("template") {
                    // ignore token
                    return;
                }

                // Add attributes to html element
                let Some(&first_node_id) = self.open_elements.first() else {
                    return;
                };
                if self.document.node_type(first_node_id) == NodeType::ElementNode {
                    for (key, value) in attributes {
                        if self.document.attribute(first_node_id, key).is_none() {
                            self.document.set_attribute(first_node_id, key, value);
                        }
                    }
                }
            }
            Token::StartTag { name, .. }
                if name == "base"
                    || name == "basefont"
                    || name == "bgsound"
                    || name == "link"
                    || name == "meta"
                    || name == "noframes"
                    || name == "script"
                    || name == "style"
                    || name == "template"
                    || name == "title" =>
            {
                self.handle_in_head(token);
            }
            Token::EndTag { name, .. } if name == "template" => {
                self.handle_in_head(token);
            }
            Token::StartTag { name, attributes, .. } if name == "body" => {
                self.parse_error("body tag not allowed in in body insertion mode");

                if self.open_elements.len() <= 1
                    || self.document.tag_name(self.open_elements[1]).unwrap_or_default() != "body"
                    || self.open_elements_has("template")
                {
                    // fragment case
                    // ignore token
                    return;
                }

                self.frameset_ok = false;

                let body_node_id = self.open_elements.iter().copied().find(|&node_id| {
                    self.document.tag_name(node_id).unwrap_or_default() == "body"
                        && self.document.namespace(node_id) == Some(HTML_NAMESPACE)
                });

                if let Some(body_node_id) = body_node_id {
                    if self.document.node_type(body_node_id) == NodeType::ElementNode {
                        for (key, value) in attributes {
                            if self.document.attribute(body_node_id, key).is_none() {
                                self.document.set_attribute(body_node_id, key, value);
                            }
                        }
                    }
                }
            }
            Token::StartTag { name, .. } if name == "frameset" => {
                self.parse_error("frameset tag not allowed in in body insertion mode");

                if self.open_elements.len() <= 1
                    || self.document.tag_name(self.open_elements[1]).unwrap_or_default() != "body"
                {
                    // ignore token
                    return;
                }

                if !self.frameset_ok {
                    // ignore token
                    return;
                }

                if self.open_elements.len() > 1 {
                    let second_node_id = self.open_elements[1];
                    if self.document.parent(second_node_id).is_some() {
                        self.document.detach(second_node_id);
                    }
                }

                // Everything up to, but not including, the root html element.
                self.open_elements.truncate(1);

                self.insert_html_element(token);

                self.insertion_mode = InsertionMode::InFrameset;
            }
            Token::Eof { .. } => {
                if self.template_insertion_mode.is_empty() {
                    self.check_open_elements_at_body_end();
                    self.stop_parsing();
                } else {
                    self.handle_in_template(token);
                }
            }
            Token::EndTag { name, .. } if name == "body" => {
                if !self.is_in_scope("body", HTML_NAMESPACE, Scope::Regular) {
                    self.parse_error("body end tag not in scope");
                    // ignore token
                    return;
                }

                self.check_open_elements_at_body_end();
                self.insertion_mode = InsertionMode::AfterBody;
            }
            Token::EndTag { name, .. } if name == "html" => {
                if !self.is_in_scope("body", HTML_NAMESPACE, Scope::Regular) {
                    self.parse_error("body end tag not in scope");
                    // ignore token
                    return;
                }

                self.check_open_elements_at_body_end();
                self.insertion_mode = InsertionMode::AfterBody;
                self.reprocess_token = true;
            }
            Token::StartTag { name, attributes, .. }
                if name == "address"
                    || name == "article"
                    || name == "aside"
                    || name == "blockquote"
                    || name == "center"
                    || name == "details"
                    || name == "dialog"
                    || name == "dir"
                    || name == "div"
                    || name == "dl"
                    || name == "fieldset"
                    || name == "figcaption"
                    || name == "figure"
                    || name == "footer"
                    || name == "header"
                    || name == "hgroup"
                    || name == "main"
                    || name == "menu"
                    || name == "nav"
                    || name == "ol"
                    || name == "p"
                    || name == "search"
                    || name == "section"
                    || name == "summary"
                    || name == "ul" =>
            {
                if self.is_in_scope("p", HTML_NAMESPACE, Scope::Button) {
                    self.close_p_element();
                }

                self.insert_html_element(token);
            }
            Token::StartTag { name, .. }
                if name == "h1" || name == "h2" || name == "h3" || name == "h4" || name == "h5" || name == "h6" =>
            {
                if self.is_in_scope("p", HTML_NAMESPACE, Scope::Button) {
                    self.close_p_element();
                }

                if ["h1", "h2", "h3", "h4", "h5", "h6"].contains(&node_tag!(self, current_node_id!(self))) {
                    self.parse_error("h1-h6 not allowed in in body insertion mode");
                    self.open_elements.pop();
                }

                self.insert_html_element(token);
            }
            Token::StartTag { name, .. } if name == "pre" || name == "listing" => {
                if self.is_in_scope("p", HTML_NAMESPACE, Scope::Button) {
                    self.close_p_element();
                }

                self.insert_html_element(token);

                self.ignore_lf = true;

                self.frameset_ok = false;
            }
            Token::StartTag { name, .. } if name == "form" => {
                if self.form_element.is_some() && !self.open_elements_has("template") {
                    self.parse_error("error with template, form shzzl");
                    // ignore token
                    return;
                }

                if self.is_in_scope("p", HTML_NAMESPACE, Scope::Button) {
                    self.close_p_element();
                }

                let node_id = self.insert_html_element(token);
                if !self.open_elements_has("template") {
                    self.form_element = Some(node_id);
                }
            }
            Token::StartTag { name, .. } if name == "li" => {
                self.frameset_ok = false;

                if self.open_elements.is_empty() {
                    return;
                }
                let mut idx = self.open_elements.len() - 1;
                loop {
                    let node_id = self.open_elements[idx];
                    let tag = self.document.tag_name(node_id).unwrap_or_default();

                    if tag == "li" {
                        self.generate_implied_end_tags(Some("li"), false);

                        if node_tag!(self, current_node_id!(self)) != "li" {
                            self.parse_error("li tag not at top of stack");
                        }

                        self.pop_until_named("li");
                        break;
                    }

                    if !["address", "div", "p"].contains(&tag) && is_special::<C>(self.document, node_id) {
                        break;
                    }

                    if idx == 0 {
                        break;
                    }
                    idx -= 1;
                }

                if self.is_in_scope("p", HTML_NAMESPACE, Scope::Button) {
                    self.close_p_element();
                }

                self.insert_html_element(token);
            }
            Token::StartTag { name, .. } if name == "dd" || name == "dt" => {
                self.frameset_ok = false;

                if self.open_elements.is_empty() {
                    return;
                }
                let mut idx = self.open_elements.len() - 1;
                loop {
                    let node_id = self.open_elements[idx];
                    let tag = self.document.tag_name(node_id).unwrap_or_default().to_owned();

                    if ["dd", "dt"].contains(&tag.as_str()) {
                        self.generate_implied_end_tags(Some(&tag), false);

                        if node_tag!(self, current_node_id!(self)) != tag.as_str() {
                            self.parse_error("{tag} tag not at top of stack");
                        }

                        self.pop_until_named(&tag);
                        break;
                    }

                    if !["address", "div", "p"].contains(&tag.as_str()) && is_special::<C>(self.document, node_id) {
                        break;
                    }

                    if idx == 0 {
                        break;
                    }
                    idx -= 1;
                }

                if self.is_in_scope("p", HTML_NAMESPACE, Scope::Button) {
                    self.close_p_element();
                }

                self.insert_html_element(token);
            }
            Token::StartTag { name, .. } if name == "plaintext" => {
                if self.is_in_scope("p", HTML_NAMESPACE, Scope::Button) {
                    self.close_p_element();
                }

                self.insert_html_element(token);

                self.tokenizer.state = State::PLAINTEXT;
            }
            Token::StartTag { name, .. } if name == "button" => {
                if self.is_in_scope("button", HTML_NAMESPACE, Scope::Regular) {
                    self.parse_error("button tag not allowed in in body insertion mode");
                    self.generate_implied_end_tags(None, false);
                    self.pop_until_named("button");
                }

                self.reconstruct_formatting();
                self.insert_html_element(token);
                self.frameset_ok = false;
            }
            Token::EndTag { name, .. }
                if name == "address"
                    || name == "article"
                    || name == "aside"
                    || name == "blockquote"
                    || name == "button"
                    || name == "center"
                    || name == "details"
                    || name == "dialog"
                    || name == "dir"
                    || name == "div"
                    || name == "dl"
                    || name == "fieldset"
                    || name == "figcaption"
                    || name == "figure"
                    || name == "footer"
                    || name == "header"
                    || name == "hgroup"
                    || name == "listing"
                    || name == "main"
                    || name == "menu"
                    || name == "nav"
                    || name == "ol"
                    || name == "pre"
                    || name == "search"
                    || name == "section"
                    || name == "summary"
                    || name == "ul" =>
            {
                if !self.is_in_scope(name, HTML_NAMESPACE, Scope::Regular) {
                    self.parse_error("end tag not in scope");
                    // ignore token
                    return;
                }

                self.generate_implied_end_tags(None, false);

                if node_tag!(self, current_node_id!(self)) != *name {
                    self.parse_error("end tag not at top of stack");
                }

                self.pop_until_named(name);
            }
            Token::EndTag { name, .. } if name == "form" => {
                if self.open_elements_has("template") {
                    if !self.is_in_scope(name, HTML_NAMESPACE, Scope::Regular) {
                        self.parse_error("end tag not in scope");
                        // ignore token
                        return;
                    }

                    self.generate_implied_end_tags(None, false);

                    if node_tag!(self, current_node_id!(self)) != *name {
                        self.parse_error("end tag not at top of stack");
                    }

                    self.pop_until_named(name);
                } else {
                    let node_id = self.form_element;
                    self.form_element = None;

                    // The pointer's own element, not any form: another form can be in scope
                    // while the pointer's form is not.
                    let Some(node_id) = node_id.filter(|&id| self.is_element_in_scope(id, Scope::Regular)) else {
                        self.parse_error("end tag not in scope");
                        // ignore token
                        return;
                    };

                    self.generate_implied_end_tags(None, false);

                    if node_tag!(self, current_node_id!(self)) != *name {
                        self.parse_error("end tag not at top of stack");
                    }

                    if node_id != current_node_id!(self) {
                        self.parse_error("end tag not at top of stack");
                    }
                    self.open_elements_remove(node_id);
                }
            }
            Token::EndTag { name, .. } if name == "p" => {
                if !self.is_in_scope(name, HTML_NAMESPACE, Scope::Button) {
                    self.parse_error("end tag not in scope");

                    let mut token = Token::StartTag {
                        name: "p".to_string(),
                        is_self_closing: false,
                        attributes: Attributes::new(),
                        location: token.get_location(),
                    };
                    self.insert_html_element(&mut token);
                }

                self.close_p_element();
            }
            Token::EndTag { name, .. } if name == "li" => {
                if !self.is_in_scope(name, HTML_NAMESPACE, Scope::ListItem) {
                    self.parse_error("end tag not in scope");
                    // ignore token
                    return;
                }

                self.generate_implied_end_tags(Some("li"), false);

                if node_tag!(self, current_node_id!(self)) != *name {
                    self.parse_error("end tag not at top of stack");
                }

                self.pop_until_named(name);
            }
            Token::EndTag { name, .. } if name == "dd" || name == "dt" => {
                if !self.is_in_scope(name, HTML_NAMESPACE, Scope::Regular) {
                    self.parse_error("end tag not in scope");
                    // ignore token
                    return;
                }

                self.generate_implied_end_tags(Some(name), false);

                if node_tag!(self, current_node_id!(self)) != *name {
                    self.parse_error("end tag not at top of stack");
                }

                self.pop_until_named(name);
            }
            Token::EndTag { name, .. }
                if name == "h1" || name == "h2" || name == "h3" || name == "h4" || name == "h5" || name == "h6" =>
            {
                if ["h1", "h2", "h3", "h4", "h5", "h6"]
                    .iter()
                    .any(|tag| self.is_in_scope(tag, HTML_NAMESPACE, Scope::Regular))
                {
                    self.generate_implied_end_tags(Some(name), false);

                    if node_tag!(self, current_node_id!(self)) != *name {
                        self.parse_error("end tag not at top of stack");
                    }

                    self.pop_until_any(&["h1", "h2", "h3", "h4", "h5", "h6"]);
                } else {
                    self.parse_error("end tag not in scope");
                    // ignore token
                }
            }
            Token::EndTag { name, .. } if name == "sarcasm" => {
                // Take a deep breath
                self.handle_in_body_any_other_end_tag(name);
            }
            Token::StartTag { name, .. } if name == "a" => {
                if let Some(node_id) = self.active_formatting_elements_has_until_marker("a") {
                    self.parse_error("a tag in active formatting elements");
                    self.adoption_agency_algorithm(token);

                    // Remove from lists if not done already by the adoption agency
                    self.open_elements_remove(node_id);
                    self.active_formatting_elements_remove(node_id);
                }

                self.reconstruct_formatting();

                let node_id = self.insert_html_element(token);
                self.active_formatting_elements_push(node_id);
            }
            Token::StartTag { name, .. }
                if name == "b"
                    || name == "big"
                    || name == "code"
                    || name == "em"
                    || name == "font"
                    || name == "i"
                    || name == "s"
                    || name == "small"
                    || name == "strike"
                    || name == "strong"
                    || name == "tt"
                    || name == "u" =>
            {
                self.reconstruct_formatting();

                let node_id = self.insert_html_element(token);
                self.active_formatting_elements_push(node_id);
            }
            Token::StartTag { name, .. } if name == "nobr" => {
                self.reconstruct_formatting();

                if self.is_in_scope("nobr", HTML_NAMESPACE, Scope::Regular) {
                    self.parse_error("nobr tag in scope");
                    self.adoption_agency_algorithm(token);
                    self.reconstruct_formatting();
                }

                let node_id = self.insert_html_element(token);
                self.active_formatting_elements_push(node_id);
            }
            Token::EndTag { name, .. }
                if name == "a"
                    || name == "b"
                    || name == "big"
                    || name == "code"
                    || name == "em"
                    || name == "font"
                    || name == "i"
                    || name == "nobr"
                    || name == "s"
                    || name == "small"
                    || name == "strike"
                    || name == "strong"
                    || name == "tt"
                    || name == "u" =>
            {
                self.adoption_agency_algorithm(token);

                #[cfg(all(feature = "debug_parser", test))]
                self.display_debug_info(token);
            }
            Token::StartTag { name, .. } if name == "applet" || name == "marquee" || name == "object" => {
                self.reconstruct_formatting();

                self.insert_html_element(token);

                self.active_formatting_elements_push_marker();
                self.frameset_ok = false;
            }
            Token::EndTag { name, .. } if name == "applet" || name == "marquee" || name == "object" => {
                if !self.is_in_scope(name, HTML_NAMESPACE, Scope::Regular) {
                    self.parse_error("end tag not in scope");
                    // ignore token
                    return;
                }

                self.generate_implied_end_tags(None, false);

                if node_tag!(self, current_node_id!(self)) != *name {
                    self.parse_error("end tag not at top of stack");
                }

                self.pop_until_named(name);
                self.active_formatting_elements_clear_until_marker();
            }
            Token::StartTag { name, .. } if name == "table" => {
                if self.document.quirks_mode() != QuirksMode::Quirks
                    && self.is_in_scope("p", HTML_NAMESPACE, Scope::Button)
                {
                    self.close_p_element();
                }

                self.insert_html_element(token);

                self.frameset_ok = false;
                self.insertion_mode = InsertionMode::InTable;
            }
            Token::EndTag { name, .. } if name == "br" => {
                self.parse_error("br end tag not allowed");
                self.reconstruct_formatting();

                // Remove attributes if any
                let mut br = token.clone();
                if let Token::StartTag { attributes, .. } = &mut br {
                    attributes.clear();
                }

                self.insert_html_element(&mut br);

                self.open_elements.pop();
                self.frameset_ok = false;
            }
            Token::StartTag {
                name, is_self_closing, ..
            } if name == "area"
                || name == "br"
                || name == "embed"
                || name == "img"
                || name == "keygen"
                || name == "wbr" =>
            {
                let is_self_closing = *is_self_closing;
                self.reconstruct_formatting();

                self.insert_html_element(token);
                self.open_elements.pop();

                self.acknowledge_closing_tag(is_self_closing);
                self.frameset_ok = false;
            }
            Token::StartTag {
                name,
                is_self_closing,
                attributes,
                ..
            } if name == "input" => {
                let is_self_closing = *is_self_closing;
                // Read before the insert: the element takes the token's attributes.
                let not_hidden = attributes.get("type").is_none_or(|t| t.cow_to_lowercase() != *"hidden");
                self.reconstruct_formatting();

                self.insert_html_element(token);
                self.open_elements.pop();

                self.acknowledge_closing_tag(is_self_closing);

                if not_hidden {
                    self.frameset_ok = false;
                }
            }
            Token::StartTag {
                name, is_self_closing, ..
            } if name == "param" || name == "source" || name == "track" => {
                let is_self_closing = *is_self_closing;
                self.insert_html_element(token);
                self.open_elements.pop();

                self.acknowledge_closing_tag(is_self_closing);
            }
            Token::StartTag {
                name, is_self_closing, ..
            } if name == "hr" => {
                let is_self_closing = *is_self_closing;
                if self.is_in_scope("p", HTML_NAMESPACE, Scope::Button) {
                    self.close_p_element();
                }

                self.insert_html_element(token);
                self.open_elements.pop();

                self.acknowledge_closing_tag(is_self_closing);
                self.frameset_ok = false;
            }
            Token::StartTag {
                name,
                is_self_closing,
                attributes,
                ..
            } if name == "image" => {
                self.parse_error("image tag not allowed");
                self.current_token = Token::StartTag {
                    name: "img".to_string(),
                    attributes: attributes.clone(),
                    is_self_closing: *is_self_closing,
                    location: token.get_location(),
                };
                // We replaced current_token in place; keep it (not the token moved out for
                // matching) so the reprocess below sees the rewritten <img> tag.
                self.current_token_rewritten = true;
                self.reprocess_token = true;
            }
            Token::StartTag { name, .. } if name == "textarea" => {
                self.insert_html_element(token);

                self.ignore_lf = true;

                self.tokenizer.state = State::RCDATA;
                self.original_insertion_mode = self.insertion_mode;
                self.frameset_ok = false;
                self.insertion_mode = InsertionMode::Text;
            }
            Token::StartTag { name, .. } if name == "xmp" => {
                if self.is_in_scope("p", HTML_NAMESPACE, Scope::Button) {
                    self.close_p_element();
                }

                self.reconstruct_formatting();

                self.frameset_ok = false;
                self.parse_raw_data(token);
            }
            Token::StartTag { name, .. } if name == "iframe" => {
                self.frameset_ok = false;
                self.parse_raw_data(token);
            }
            Token::StartTag { name, .. } if name == "noembed" => {
                self.parse_raw_data(token);
            }
            Token::StartTag { name, .. } if name == "noscript" && self.scripting_enabled => {
                self.parse_raw_data(token);
            }
            Token::StartTag { name, .. } if name == "select" => {
                self.reconstruct_formatting();

                self.insert_html_element(token);
                self.frameset_ok = false;

                if self.insertion_mode == InsertionMode::InTable
                    || self.insertion_mode == InsertionMode::InCaption
                    || self.insertion_mode == InsertionMode::InTableBody
                    || self.insertion_mode == InsertionMode::InRow
                    || self.insertion_mode == InsertionMode::InCell
                {
                    self.insertion_mode = InsertionMode::InSelectInTable;
                } else {
                    self.insertion_mode = InsertionMode::InSelect;
                }
            }
            Token::StartTag { name, .. } if name == "optgroup" || name == "option" => {
                if node_tag!(self, current_node_id!(self)) == "option" {
                    self.open_elements.pop();
                }

                self.reconstruct_formatting();

                self.insert_html_element(token);
            }
            Token::StartTag { name, .. } if name == "rb" || name == "rtc" => {
                if self.is_in_scope("ruby", HTML_NAMESPACE, Scope::Regular) {
                    self.generate_implied_end_tags(None, false);
                }

                if node_tag!(self, current_node_id!(self)) != "ruby" {
                    self.parse_error("rb or rtc not in scope");
                }

                self.insert_html_element(token);
            }
            Token::StartTag { name, .. } if name == "rp" || name == "rt" => {
                if self.is_in_scope("ruby", HTML_NAMESPACE, Scope::Regular) {
                    self.generate_implied_end_tags(Some("rtc"), false);
                }

                if node_tag!(self, current_node_id!(self)) != "rtc" && node_tag!(self, current_node_id!(self)) != "ruby"
                {
                    self.parse_error("rp or rt not in scope");
                }

                self.insert_html_element(token);
            }
            Token::StartTag {
                name, is_self_closing, ..
            } if name == "math" => {
                let is_self_closing = *is_self_closing;
                self.reconstruct_formatting();

                self.adjust_foreign_attributes(token, MATHML_NAMESPACE);

                self.insert_foreign_element(token, MATHML_NAMESPACE);

                if is_self_closing {
                    self.open_elements.pop();
                    self.acknowledge_closing_tag(is_self_closing);
                }
            }
            Token::StartTag {
                name, is_self_closing, ..
            } if name == "svg" => {
                let is_self_closing = *is_self_closing;
                self.reconstruct_formatting();

                self.adjust_foreign_attributes(token, SVG_NAMESPACE);
                self.insert_foreign_element(token, SVG_NAMESPACE);

                if is_self_closing {
                    self.open_elements.pop();
                    self.acknowledge_closing_tag(is_self_closing);
                }
            }
            Token::StartTag { name, .. }
                if name == "caption"
                    || name == "col"
                    || name == "colgroup"
                    || name == "frame"
                    || name == "head"
                    || name == "tbody"
                    || name == "td"
                    || name == "tfoot"
                    || name == "th"
                    || name == "thead"
                    || name == "tr" =>
            {
                self.parse_error("tag not allowed in in body insertion mode");
                // ignore token
            }
            Token::StartTag { .. } => {
                self.reconstruct_formatting();
                self.insert_html_element(token);
            }
            Token::EndTag { name, .. } => {
                self.handle_in_body_any_other_end_tag(name);
            }
        }
    }

    /// Handles and other end tag as found during the in-body insertion mode. This needs to be a
    /// separate function as this is also called during the adoption agency algorithm
    pub(crate) fn handle_in_body_any_other_end_tag(&mut self, tag_name: &str) {
        if self.open_elements.is_empty() {
            self.parse_error("no open elements");
            // ignore token
            return;
        }

        for idx in (0..self.open_elements.len()).rev() {
            let node_id = self.open_elements[idx];
            if self.document.tag_name(node_id).unwrap_or_default() == tag_name
                && self.document.namespace(node_id) == Some(HTML_NAMESPACE)
            {
                self.generate_implied_end_tags(Some(tag_name), false);

                // It might be possible that the last item is not our node_id. Emit parse error if so
                if current_node_id!(self) != node_id {
                    self.parse_error("end tag not at top of stack");
                }

                // Pop until we reach node_id
                while current_node_id!(self) != node_id {
                    self.open_elements.pop();
                }
                // Pop node_id as well
                self.open_elements.pop();

                break;
            }

            if is_special::<C>(self.document, node_id) {
                self.parse_error("special node");
                // ignore token
                return;
            }
        }
    }

    /// Close the p element that may or may not be on the open elements stack
    pub(crate) fn close_p_element(&mut self) {
        self.generate_implied_end_tags(Some("p"), false);

        if node_tag!(self, current_node_id!(self)) != "p" {
            self.parse_error("p element not at top of stack");
        }

        self.pop_until_named("p");
    }
}
