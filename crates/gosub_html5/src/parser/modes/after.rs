//! The insertion modes after the body: after body, in frameset, after frameset, after after body
//! and after after frameset.

use crate::parser::{Html5Parser, InsertionMode};
use crate::tokenizer::token::Token;
use gosub_interface::config::HasDocument;
use gosub_interface::document::Document;
use gosub_shared::node::NodeId;

impl<C: HasDocument> Html5Parser<'_, C> {
    /// Handle insertion mode "`after_body`"
    pub(crate) fn handle_after_body(&mut self, token: &mut Token) {
        match &*token {
            Token::Text { text: value, location } if token.is_mixed() => {
                let tokens = self.split_mixed_token(value, *location);
                self.tokenizer.insert_tokens_at_queue_start(tokens);
            }
            Token::Text { .. } if token.is_empty_or_white() => {
                self.handle_in_body(token);
            }
            Token::Comment { .. } => {
                let html_node_id = self.open_elements.first().unwrap_or_default();
                self.insert_comment_element(token, Some(*html_node_id));
            }
            Token::DocType { .. } => {
                self.parse_error("doctype not allowed in after body insertion mode");
                // ignore token
            }
            Token::StartTag { name, .. } if name == "html" => {
                self.handle_in_body(token);
            }
            Token::EndTag { name, .. } if name == "html" => {
                if self.is_fragment_case {
                    // fragment case
                    self.parse_error("html end tag not allowed in after body insertion mode");
                    // ignore token
                    return;
                }
                self.insertion_mode = InsertionMode::AfterAfterBody;
            }
            Token::Eof { .. } => {
                self.stop_parsing();
            }
            _ => {
                self.parse_error("anything else not allowed in after body insertion mode");
                self.insertion_mode = InsertionMode::InBody;
                self.reprocess_token = true;
            }
        }
    }

    /// Handle insertion mode "`in_frameset`"
    pub(crate) fn handle_in_frameset(&mut self, token: &mut Token) {
        match &*token {
            Token::Text { text: value, location } if token.is_mixed() => {
                let tokens = self.split_mixed_token(value, *location);
                self.tokenizer.insert_tokens_at_queue_start(tokens);
            }
            Token::Text { .. } if token.is_empty_or_white() => {
                self.insert_text_element(token);
            }
            Token::Comment { .. } => {
                self.insert_comment_element(token, None);
            }
            Token::DocType { .. } => {
                self.parse_error("doctype not allowed in frameset insertion mode");
                // ignore token
            }
            Token::StartTag { name, .. } if name == "html" => {
                self.handle_in_body(token);
            }
            Token::StartTag { name, .. } if name == "frameset" => {
                self.insert_html_element(token);
            }
            Token::EndTag { name, .. } if name == "frameset" => {
                if node_tag!(self, current_node_id!(self)) == "html" {
                    // fragment case
                    self.parse_error("frameset tag not allowed in frameset insertion mode");
                    // ignore token
                    return;
                }

                self.open_elements.pop();

                if !self.is_fragment_case && node_tag!(self, current_node_id!(self)) != "frameset" {
                    // fragment case
                    self.insertion_mode = InsertionMode::AfterFrameset;
                }
            }
            Token::StartTag {
                name, is_self_closing, ..
            } if name == "frame" => {
                self.acknowledge_closing_tag(*is_self_closing);

                self.insert_html_element(token);
                self.open_elements.pop();
            }
            Token::StartTag { name, .. } if name == "noframes" => {
                self.handle_in_head(token);
            }
            Token::Eof { .. } => {
                if node_tag!(self, current_node_id!(self)) != "html" {
                    self.parse_error("eof not allowed in frameset insertion mode");
                }
                self.stop_parsing();
            }
            _ => {
                self.parse_error("anything else not allowed in frameset insertion mode");
                // ignore token
            }
        }
    }

    /// Handle insertion mode "`after_frameset`"
    pub(crate) fn handle_after_frameset(&mut self, token: &mut Token) {
        match &*token {
            Token::Text { text: value, location } if token.is_mixed() => {
                let tokens = self.split_mixed_token(value, *location);
                self.tokenizer.insert_tokens_at_queue_start(tokens);
            }
            Token::Text { .. } if token.is_empty_or_white() => {
                self.insert_text_element(token);
            }
            Token::Comment { .. } => {
                self.insert_comment_element(token, None);
            }
            Token::DocType { .. } => {
                self.parse_error("doctype not allowed in frameset insertion mode");
                // ignore token
            }
            Token::StartTag { name, .. } if name == "html" => {
                self.handle_in_body(token);
            }
            Token::EndTag { name, .. } if name == "html" => {
                self.insertion_mode = InsertionMode::AfterAfterFrameset;
            }
            Token::StartTag { name, .. } if name == "noframes" => {
                self.handle_in_head(token);
            }
            Token::Eof { .. } => {
                self.stop_parsing();
            }
            _ => {
                self.parse_error("anything else not allowed in after frameset insertion mode");
                // ignore token
            }
        }
    }

    /// Handle insertion mode "`after_after_body`"
    pub(crate) fn handle_after_after_body(&mut self, token: &mut Token) {
        match &*token {
            Token::Comment { .. } => {
                self.insert_comment_element(token, Some(NodeId::root()));
            }
            Token::DocType { .. } => {
                self.handle_in_body(token);
            }
            Token::Text { text: value, location } if token.is_mixed() => {
                let tokens = self.split_mixed_token(value, *location);
                self.tokenizer.insert_tokens_at_queue_start(tokens);
            }
            Token::Text { .. } if token.is_empty_or_white() => {
                self.handle_in_body(token);
            }
            Token::StartTag { name, .. } if name == "html" => {
                self.handle_in_body(token);
            }
            Token::Eof { .. } => {
                self.stop_parsing();
            }
            _ => {
                self.parse_error("anything else not allowed in after after body insertion mode");
                self.insertion_mode = InsertionMode::InBody;
                self.reprocess_token = true;
            }
        }
    }

    /// Handle insertion mode "`after_after_frameset`"
    pub(crate) fn handle_after_after_frameset(&mut self, token: &mut Token) {
        match &*token {
            Token::Comment { .. } => {
                self.insert_comment_element(token, Some(NodeId::root()));
            }
            Token::DocType { .. } => {
                self.handle_in_body(token);
            }
            Token::Text { text: value, location } if token.is_mixed() => {
                let tokens = self.split_mixed_token(value, *location);
                self.tokenizer.insert_tokens_at_queue_start(tokens);
            }
            Token::Text { .. } if token.is_empty_or_white() => {
                self.handle_in_body(token);
            }
            Token::StartTag { name, .. } if name == "html" => {
                self.handle_in_body(token);
            }
            Token::Eof { .. } => {
                self.stop_parsing();
            }
            Token::StartTag { name, .. } if name == "noframes" => {
                self.handle_in_head(token);
            }
            _ => {
                self.parse_error("anything else not allowed in after after frameset insertion mode");
                // ignore token
            }
        }
    }
}
