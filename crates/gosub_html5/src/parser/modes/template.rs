//! The in template insertion mode.

use crate::parser::{Html5Parser, InsertionMode};
use crate::tokenizer::token::Token;
use gosub_interface::config::HasDocument;

impl<C: HasDocument> Html5Parser<'_, C> {
    /// Handle insertion mode "`in_template`"
    pub(crate) fn handle_in_template(&mut self, token: &mut Token) {
        match &*token {
            Token::Text { .. } | Token::Comment { .. } | Token::DocType { .. } => {
                self.handle_in_body(token);
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
            Token::StartTag { name, .. }
                if name == "caption" || name == "colgroup" || name == "tbody" || name == "tfoot" || name == "thead" =>
            {
                self.template_insertion_mode.pop();
                self.template_insertion_mode.push(InsertionMode::InTable);

                self.insertion_mode = InsertionMode::InTable;
                self.reprocess_token = true;
            }
            Token::StartTag { name, .. } if name == "col" => {
                self.template_insertion_mode.pop();
                self.template_insertion_mode.push(InsertionMode::InColumnGroup);
                self.insertion_mode = InsertionMode::InColumnGroup;
                self.reprocess_token = true;
            }
            Token::StartTag { name, .. } if name == "tr" => {
                self.template_insertion_mode.pop();
                self.template_insertion_mode.push(InsertionMode::InTableBody);
                self.insertion_mode = InsertionMode::InTableBody;
                self.reprocess_token = true;
            }
            Token::StartTag { name, .. } if name == "td" || name == "th" => {
                self.template_insertion_mode.pop();
                self.template_insertion_mode.push(InsertionMode::InRow);
                self.insertion_mode = InsertionMode::InRow;
                self.reprocess_token = true;
            }
            Token::StartTag { .. } => {
                self.template_insertion_mode.pop();
                self.template_insertion_mode.push(InsertionMode::InBody);
                self.insertion_mode = InsertionMode::InBody;
                self.reprocess_token = true;
            }
            Token::EndTag { .. } => {
                self.parse_error("end tag not allowed in in template insertion mode");
                // ignore token
            }
            Token::Eof { .. } => {
                if !self.open_elements_has("template") {
                    // fragment case
                    self.stop_parsing();
                    return;
                }

                self.parse_error("eof not allowed in in template insertion mode");

                self.pop_until_named("template");
                self.active_formatting_elements_clear_until_marker();
                // Pop the closed template's mode first: the reset reads the one below it.
                self.template_insertion_mode.pop();
                self.reset_insertion_mode();
                self.reprocess_token = true;
            }
        }
    }
}
