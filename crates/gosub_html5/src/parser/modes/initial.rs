//! The initial, before html and before head insertion modes.

use crate::parser::errors::ParserError;
use crate::parser::{Html5Parser, InsertionMode};
use crate::tokenizer::token::Token;
use gosub_interface::config::HasDocument;
use gosub_interface::node::QuirksMode;
use gosub_shared::attributes::Attributes;
use gosub_shared::node::NodeId;

impl<C: HasDocument> Html5Parser<'_, C> {
    /// Handle insertion mode "`initial`"
    pub(crate) fn handle_initial(&mut self, token: &mut Token) {
        let mut anything_else = false;

        match &*token {
            Token::Text { text: value, location } if token.is_mixed() => {
                let tokens = self.split_mixed_token(value, *location);
                self.tokenizer.insert_tokens_at_queue_start(tokens);
                return;
            }
            Token::Text { .. } if token.is_empty_or_white() => {
                // ignore token
            }
            Token::Comment { .. } => {
                self.insert_comment_element(token, Some(NodeId::root()));
            }
            Token::DocType {
                name,
                pub_identifier,
                sys_identifier,
                force_quirks,
                ..
            } => {
                if name.as_deref().is_some_and(|name| name != "html")
                    || pub_identifier.is_some()
                    || sys_identifier.as_deref().is_some_and(|id| id != "about:legacy-compat")
                {
                    self.parse_error("doctype not allowed in initial insertion mode");
                }

                self.insert_doctype_element(token);

                if !self.is_iframesrcdoc() && !self.parser_cannot_change_mode {
                    self.set_quirks_mode(self.identify_quirks_mode(
                        name,
                        pub_identifier.clone(),
                        sys_identifier.clone(),
                        *force_quirks,
                    ));
                }

                self.insertion_mode = InsertionMode::BeforeHtml;
            }
            Token::StartTag { .. } => {
                if !self.is_iframesrcdoc() {
                    self.parse_error(ParserError::ExpectedDocTypeButGotStartTag.as_str());
                }
                anything_else = true;
            }
            Token::EndTag { .. } => {
                if !self.is_iframesrcdoc() {
                    self.parse_error(ParserError::ExpectedDocTypeButGotEndTag.as_str());
                }
                anything_else = true;
            }
            Token::Text { .. } => {
                if !self.is_iframesrcdoc() {
                    self.parse_error(ParserError::ExpectedDocTypeButGotChars.as_str());
                }
                anything_else = true;
            }
            Token::Eof { .. } => anything_else = true,
        }

        if anything_else {
            // An iframe srcdoc document without a DOCTYPE is neither an error nor quirks.
            if !self.is_iframesrcdoc() && !self.parser_cannot_change_mode {
                self.set_quirks_mode(QuirksMode::Quirks);
            }

            self.insertion_mode = InsertionMode::BeforeHtml;
            self.reprocess_token = true;
        }
    }

    /// Handle insertion mode "`before_html`"
    pub(crate) fn handle_before_html(&mut self, token: &mut Token) {
        let mut anything_else = false;

        match &*token {
            Token::DocType { .. } => {
                self.parse_error("doctype not allowed in before html insertion mode");
            }
            Token::Comment { .. } => {
                self.insert_comment_element(token, Some(NodeId::root()));
            }
            Token::Text { text: value, location } if token.is_mixed() => {
                let tokens = self.split_mixed_token(value, *location);
                self.tokenizer.insert_tokens_at_queue_start(tokens);
                return;
            }
            Token::Text { .. } if token.is_empty_or_white() => {
                // ignore token
            }
            Token::StartTag { name, .. } if name == "html" => {
                self.insert_document_element(token);

                self.insertion_mode = InsertionMode::BeforeHead;
            }
            Token::EndTag { name, .. } if name == "head" || name == "body" || name == "html" || name == "br" => {
                anything_else = true;
            }
            Token::EndTag { .. } => {
                self.parse_error("end tag not allowed in before html insertion mode");
            }
            _ => {
                anything_else = true;
            }
        }

        if anything_else {
            let mut implied = Token::StartTag {
                name: "html".to_string(),
                is_self_closing: false,
                attributes: Attributes::new(),
                location: token.get_location(),
            };
            self.insert_document_element(&mut implied);

            self.insertion_mode = InsertionMode::BeforeHead;
            self.reprocess_token = true;
        }
    }

    /// Handle insertion mode "`before_head`"
    pub(crate) fn handle_before_head(&mut self, token: &mut Token) {
        let mut anything_else = false;

        match &*token {
            Token::Text { text: value, location } if token.is_mixed() => {
                let tokens = self.split_mixed_token(value, *location);
                self.tokenizer.insert_tokens_at_queue_start(tokens);
                return;
            }
            Token::Text { .. } if token.is_empty_or_white() => {
                // ignore token
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
            Token::StartTag { name, .. } if name == "head" => {
                let node_id = self.insert_html_element(token);
                self.head_element = Some(node_id);
                self.insertion_mode = InsertionMode::InHead;
            }
            Token::EndTag { name, .. } if name == "head" || name == "body" || name == "html" || name == "br" => {
                anything_else = true;
            }
            Token::EndTag { .. } => {
                self.parse_error("end tag not allowed in before head insertion mode");
                // ignore token
            }
            _ => {
                anything_else = true;
            }
        }
        if anything_else {
            let mut implied = Token::StartTag {
                name: "head".to_string(),
                is_self_closing: false,
                attributes: Attributes::new(),
                location: token.get_location(),
            };
            let node_id = self.insert_html_element(&mut implied);
            self.head_element = Some(node_id);
            self.insertion_mode = InsertionMode::InHead;
            self.reprocess_token = true;
        }
    }
}
