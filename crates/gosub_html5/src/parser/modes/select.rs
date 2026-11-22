//! The in select and in select in table insertion modes.

use crate::node::HTML_NAMESPACE;
use crate::parser::{Html5Parser, Scope};
use crate::tokenizer::token::Token;
use gosub_interface::config::HasDocument;
use gosub_interface::document::Document;
use gosub_shared::node::NodeId;

impl<C: HasDocument> Html5Parser<'_, C> {
    /// Handle insertion mode "`in_select`"
    pub(crate) fn handle_in_select(&mut self, token: &mut Token) {
        match &*token {
            Token::Text { text: value, location } if token.is_mixed() => {
                let tokens = self.split_mixed_token(value, *location);
                self.tokenizer.insert_tokens_at_queue_start(tokens);
            }
            Token::Text { text: value, location } if token.is_null() => {
                self.null_parse_errors(
                    value,
                    *location,
                    "null character not allowed in in select insertion mode",
                );
                // ignore token
            }
            Token::Text { .. } => {
                self.insert_text_element(token);
            }
            Token::Comment { .. } => {
                self.insert_comment_element(token, None);
            }
            Token::DocType { .. } => {
                self.parse_error("doctype not allowed in in select insertion mode");
                // ignore token
            }
            Token::StartTag { name, .. } if name == "html" => {
                self.handle_in_body(token);
            }
            Token::StartTag { name, .. } if name == "option" => {
                if node_tag!(self, current_node_id!(self)) == "option" {
                    self.open_elements.pop();
                }

                self.insert_html_element(token);
            }
            Token::StartTag { name, .. } if name == "optgroup" => {
                if node_tag!(self, current_node_id!(self)) == "option" {
                    self.open_elements.pop();
                }

                if node_tag!(self, current_node_id!(self)) == "optgroup" {
                    self.open_elements.pop();
                }

                self.insert_html_element(token);
            }
            Token::StartTag {
                name, is_self_closing, ..
            } if name == "hr" => {
                if node_tag!(self, current_node_id!(self)) == "option" {
                    self.open_elements.pop();
                }

                if node_tag!(self, current_node_id!(self)) == "optgroup" {
                    self.open_elements.pop();
                }

                self.acknowledge_closing_tag(*is_self_closing);

                self.insert_html_element(token);
                self.open_elements.pop();
            }
            Token::EndTag { name, .. } if name == "optgroup" => {
                if node_tag!(self, current_node_id!(self)) == "option"
                    && self.open_elements.len() > 1
                    && self
                        .document
                        .tag_name(self.open_elements[self.open_elements.len() - 2])
                        .unwrap_or_default()
                        == "optgroup"
                {
                    self.open_elements.pop();
                }

                if node_tag!(self, current_node_id!(self)) == "optgroup" {
                    self.open_elements.pop();
                } else {
                    self.parse_error("optgroup end tag not allowed in in select insertion mode");
                    // ignore token
                }
            }
            Token::EndTag { name, .. } if name == "option" => {
                if node_tag!(self, current_node_id!(self)) == "option" {
                    self.open_elements.pop();
                } else {
                    self.parse_error("option end tag not allowed in in select insertion mode");
                    // ignore token
                }
            }
            Token::EndTag { name, .. } if name == "select" => {
                if !self.is_in_scope("select", HTML_NAMESPACE, Scope::Select) {
                    // fragment case
                    self.parse_error("select end tag not allowed in in select insertion mode");
                    // ignore token
                    return;
                }

                self.pop_until_named("select");
                self.reset_insertion_mode();
            }
            Token::StartTag { name, .. } if name == "select" => {
                self.parse_error("select tag not allowed in in select insertion mode");

                if !self.is_in_scope("select", HTML_NAMESPACE, Scope::Select) {
                    // fragment case
                    // ignore token
                    return;
                }

                self.pop_until_named("select");
                self.reset_insertion_mode();
            }
            Token::StartTag { name, .. } if name == "input" || name == "keygen" || name == "textarea" => {
                self.parse_error("input, keygen or textarea tag not allowed in in select insertion mode");

                if !self.is_in_scope("select", HTML_NAMESPACE, Scope::Select) {
                    // fragment case
                    // ignore token
                    return;
                }

                self.pop_until_named("select");
                self.reset_insertion_mode();
                self.reprocess_token = true;
            }

            Token::StartTag { name, .. } if name == "script" || name == "template" => {
                self.handle_in_head(token);
            }
            Token::EndTag { name, .. } if name == "template" => {
                self.handle_in_head(token);
            }
            Token::Eof { .. } => {
                self.handle_in_body(token);
            }
            _ => {
                self.parse_error("anything else not allowed in in select insertion mode");
                // ignore token
            }
        }
    }

    /// Handle insertion mode "`in_select_in_table`"
    pub(crate) fn handle_in_select_in_table(&mut self, token: &mut Token) {
        match &*token {
            Token::StartTag { name, .. }
                if name == "caption"
                    || name == "table"
                    || name == "tbody"
                    || name == "tfoot"
                    || name == "thead"
                    || name == "tr"
                    || name == "td"
                    || name == "th" =>
            {
                self.parse_error("caption, table, tbody, tfoot, thead, tr, td or th tag not allowed in in select in table insertion mode");

                self.pop_until_named("select");
                self.reset_insertion_mode();
                self.reprocess_token = true;
            }
            Token::EndTag { name, .. }
                if name == "caption"
                    || name == "table"
                    || name == "tbody"
                    || name == "tfoot"
                    || name == "thead"
                    || name == "tr"
                    || name == "td"
                    || name == "th" =>
            {
                self.parse_error("caption, table, tbody, tfoot, thead, tr, td or th tag not allowed in in select in table insertion mode");

                if !self.is_in_scope(name, HTML_NAMESPACE, Scope::Table) {
                    // ignore token
                    return;
                }

                self.pop_until_named("select");
                self.reset_insertion_mode();
                self.reprocess_token = true;
            }
            _ => self.handle_in_select(token),
        }
    }
}
