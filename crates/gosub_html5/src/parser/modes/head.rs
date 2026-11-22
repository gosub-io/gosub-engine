//! The in head, in head noscript and after head insertion modes.

use crate::node::HTML_NAMESPACE;
use crate::parser::{meta_charset_encoding, Html5Parser, InsertionMode};
use crate::tokenizer::state::State;
use crate::tokenizer::token::Token;
use gosub_interface::config::HasDocument;
use gosub_interface::document::Document;
use gosub_interface::node::NodeType;
use gosub_shared::attributes::Attributes;
use gosub_shared::node::NodeId;

impl<C: HasDocument> Html5Parser<'_, C> {
    /// Handle insertion mode "`in_head`"
    pub(crate) fn handle_in_head(&mut self, token: &mut Token) {
        let mut anything_else = false;

        match &*token {
            Token::Text { text: value, location } if token.is_mixed() => {
                let tokens = self.split_mixed_token(value, *location);
                self.tokenizer.insert_tokens_at_queue_start(tokens);
                return;
            }
            Token::Text { .. } if token.is_empty_or_white() => {
                self.insert_text_element(token);
            }
            Token::Comment { .. } => {
                self.insert_comment_element(token, None);
            }
            Token::DocType { .. } => {
                self.parse_error("doctype not allowed in before head insertion mode");
                // ignore token
            }
            Token::StartTag { name, .. } if name == "html" => {
                self.handle_in_body(token);
            }
            Token::StartTag {
                name,
                is_self_closing,
                attributes,
                ..
            } if name == "base" || name == "basefont" || name == "bgsound" || name == "link" => {
                if name == "link" {
                    // Handle link elements, as it depends on rel/itemprop attributes and other factors
                    self.handle_link_element(attributes);
                }

                self.acknowledge_closing_tag(*is_self_closing);

                self.insert_html_element(token);
                self.open_elements.pop();
            }
            Token::StartTag {
                name, is_self_closing, ..
            } if name == "meta" => {
                self.acknowledge_closing_tag(*is_self_closing);

                // Read before the insert: the element takes the token's attributes.
                let encoding = meta_charset_encoding(token);
                self.insert_html_element(token);
                self.open_elements.pop();

                // The speculative parser is not implemented, so we always proceed.
                // Change the encoding if the meta element declares one. The stream only
                // does so while its confidence is tentative and makes it certain after,
                // so later declarations are ignored (WHATWG HTML 13.2.3.4).
                if let Some(enc) = encoding {
                    self.tokenizer.stream.change_encoding(enc);
                }
            }
            Token::StartTag { name, .. } if name == "title" => {
                self.parse_rcdata(token);
            }
            Token::StartTag { name, .. } if name == "noscript" && self.scripting_enabled => {
                self.parse_raw_data(token);
            }
            Token::StartTag { name, .. } if name == "noframes" => {
                self.parse_raw_data(token);
            }
            Token::StartTag { name, .. } if name == "style" => {
                self.parse_raw_data(token);
            }

            Token::StartTag { name, .. } if name == "noscript" && !self.scripting_enabled => {
                self.insert_html_element(token);
                self.insertion_mode = InsertionMode::InHeadNoscript;
            }
            Token::StartTag { name, attributes, .. } if name == "script" => {
                // An inline script always blocks: `async` and `defer` say nothing about a
                // script with no `src` to fetch. One with a `src` blocks
                // only when it carries neither.
                self.current_script_blocks = match attributes.get("src") {
                    None => true,
                    Some(_) => !attributes.contains_key("async") && !attributes.contains_key("defer"),
                };

                let insert_position = self.appropriate_place_insert(None);
                let node_id = self.create_node(token, HTML_NAMESPACE);
                self.insert_element_helper(node_id, insert_position);

                // TODO Set the element's parser document to the Document, and set the element's force async to false.

                if self.is_fragment_case {
                    // fragment case
                    self.script_already_started = true;
                }

                // TODO if the parser was invoked by document.write/writeln, set script's element already started flag to true

                self.open_elements.push(node_id);

                self.tokenizer.state = State::ScriptData;
                self.original_insertion_mode = self.insertion_mode;
                self.insertion_mode = InsertionMode::Text;
            }
            Token::EndTag { name, .. } if name == "head" => {
                self.pop_check("head");
                self.insertion_mode = InsertionMode::AfterHead;
            }
            Token::EndTag { name, .. } if name == "body" || name == "html" || name == "br" => {
                anything_else = true;
            }
            Token::StartTag {
                name,
                attributes,
                location,
                ..
            } if name == "template" => {
                self.active_formatting_elements_push_marker();
                self.frameset_ok = false;
                self.insertion_mode = InsertionMode::InTemplate;
                self.template_insertion_mode.push(InsertionMode::InTemplate);

                if let Some(shadow_root_id) = self.attach_declarative_shadow_root(attributes, *location) {
                    // The template declared a shadow root, so it never enters the tree itself -
                    // only the shadow root it created does. The element is still created and
                    // pushed onto the stack of open elements, because everything downstream keys
                    // off it: the `</template>` end tag, "in template" insertion mode, and the
                    // template-contents redirection in `appropriate_place_insert`. Pointing its
                    // template contents at the shadow root is what routes the children in.
                    let node_id = self.create_node(token, HTML_NAMESPACE);
                    self.open_elements.push(node_id);
                    self.document.set_template_contents(node_id, shadow_root_id);
                } else {
                    let node_id = self.insert_html_element(token);

                    // An ordinary template holds its own contents: the redirection above then
                    // lands back on the template element itself.
                    let cn_id = current_node_id!(self);
                    if self.document.node_type(node_id) == NodeType::ElementNode {
                        self.document.set_template_contents(node_id, cn_id);
                    }
                }
            }
            Token::EndTag { name, .. } if name == "template" => {
                if !self.open_elements_has("template") {
                    self.parse_error("could not find template tag in open element stack");
                    // ignore token
                    return;
                }

                self.generate_implied_end_tags(None, true);

                if node_tag!(self, current_node_id!(self)) != "template" {
                    self.parse_error("template end tag not at top of stack");
                }

                self.pop_until_named("template");
                self.active_formatting_elements_clear_until_marker();
                self.template_insertion_mode.pop();
                self.reset_insertion_mode();
            }
            Token::StartTag { name, .. } if name == "head" => {
                self.parse_error("head tag not allowed in in head insertion mode");
                // ignore token
                return;
            }
            Token::EndTag { .. } => {
                self.parse_error("end tag not allowed in in head insertion mode");
                // ignore token
                return;
            }
            _ => {
                anything_else = true;
            }
        }
        if anything_else {
            self.pop_check("head");
            self.insertion_mode = InsertionMode::AfterHead;
            self.reprocess_token = true;
        }
    }

    /// Handle insertion mode "`in_head_noscript`"
    pub(crate) fn handle_in_head_noscript(&mut self, token: &mut Token) {
        let mut anything_else = false;

        match &*token {
            Token::DocType { .. } => {
                self.parse_error("doctype not allowed in 'head no script' insertion mode");
                // ignore token
                return;
            }
            Token::StartTag { name, .. } if name == "html" => {
                self.handle_in_body(token);
            }
            Token::EndTag { name, .. } if name == "noscript" => {
                self.pop_check("noscript");
                self.insertion_mode = InsertionMode::InHead;
            }
            Token::Text { text: value, location } if token.is_mixed() => {
                let tokens = self.split_mixed_token(value, *location);
                self.tokenizer.insert_tokens_at_queue_start(tokens);
                return;
            }
            Token::Text { .. } if token.is_empty_or_white() => {
                self.handle_in_head(token);
            }
            Token::Comment { .. } => {
                self.handle_in_head(token);
            }
            Token::StartTag { name, .. }
                if name == "basefont"
                    || name == "bgsound"
                    || name == "link"
                    || name == "meta"
                    || name == "noframes"
                    || name == "style" =>
            {
                self.handle_in_head(token);
            }
            Token::EndTag { name, .. } if name == "br" => {
                anything_else = true;
            }
            Token::StartTag { name, .. } if name == "head" || name == "noscript" => {
                self.parse_error("head or noscript tag not allowed in after head insertion mode");
                // ignore token
            }
            Token::EndTag { .. } => {
                self.parse_error("end tag not allowed in after head insertion mode");
                // ignore token
            }
            _ => {
                anything_else = true;
            }
        }
        if anything_else {
            self.parse_error("anything else not allowed in after head insertion mode");

            self.pop_check("noscript");

            self.insertion_mode = InsertionMode::InHead;
            self.reprocess_token = true;
        }
    }

    /// Handle insertion mode "`after_head`"
    pub(crate) fn handle_after_head(&mut self, token: &mut Token) {
        let mut anything_else = false;

        match &*token {
            Token::Text { text: value, location } if token.is_mixed() => {
                let tokens = self.split_mixed_token(value, *location);
                self.tokenizer.insert_tokens_at_queue_start(tokens);
                return;
            }
            Token::Text { .. } if token.is_empty_or_white() => {
                self.insert_text_element(token);
            }
            Token::Comment { .. } => {
                self.insert_comment_element(token, None);
            }
            Token::DocType { .. } => {
                self.parse_error("doctype not allowed in after head insertion mode");
                // ignore token
            }
            Token::StartTag { name, .. } if name == "html" => {
                self.handle_in_body(token);
            }
            Token::StartTag { name, .. } if name == "body" => {
                self.insert_html_element(token);

                self.frameset_ok = false;
                self.insertion_mode = InsertionMode::InBody;
            }
            Token::StartTag { name, .. } if name == "frameset" => {
                self.insert_html_element(token);

                self.insertion_mode = InsertionMode::InFrameset;
            }
            Token::StartTag { name, .. }
                if [
                    "base", "basefont", "bgsound", "link", "meta", "noframes", "script", "style", "template", "title",
                ]
                .contains(&name.as_str()) =>
            {
                self.parse_error("invalid start tag in after head insertion mode");

                if let Some(node_id) = self.head_element {
                    self.open_elements.push(node_id);
                }

                self.handle_in_head(token);

                // Remove the node pointed to by the head element pointer from the stack of open elements (might not be current node at this point)
                if let Some(node_id) = self.head_element {
                    self.open_elements_remove(node_id);
                }
            }
            Token::EndTag { name, .. } if name == "template" => {
                self.handle_in_head(token);
            }
            Token::EndTag { name, .. } if name == "body" || name == "html" || name == "br" => {
                anything_else = true;
            }
            Token::StartTag { name, .. } if name == "head" => {
                self.parse_error("head tag not allowed in after head insertion mode");
                // ignore token
            }
            Token::EndTag { .. } => {
                self.parse_error("end tag not allowed in after head insertion mode");
                // Ignore token
            }
            _ => {
                anything_else = true;
            }
        }

        if anything_else {
            let mut implied = Token::StartTag {
                name: "body".to_string(),
                is_self_closing: false,
                attributes: Attributes::new(),
                location: token.get_location(),
            };
            self.insert_html_element(&mut implied);

            self.insertion_mode = InsertionMode::InBody;
            self.reprocess_token = true;
        }
    }
}
