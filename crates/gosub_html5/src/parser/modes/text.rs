//! The text insertion mode.

use crate::parser::Html5Parser;
use crate::tokenizer::token::Token;
use gosub_interface::config::HasDocument;
use gosub_interface::css3::{CssOrigin, CssSystem};
use gosub_interface::document::Document;
use gosub_shared::node::NodeId;

impl<C: HasDocument> Html5Parser<'_, C> {
    /// Handle insertion mode "`text`"
    pub(crate) fn handle_text(&mut self, token: &mut Token) {
        match &*token {
            Token::Text { .. } => {
                self.insert_text_element(token);
            }
            Token::Eof { .. } => {
                self.parse_error("eof not allowed in text insertion mode");

                let cn_id = current_node_id!(self);
                if node_tag!(self, cn_id) == "script" {
                    self.script_already_started = true;
                }
                self.open_elements.pop();
                self.insertion_mode = self.original_insertion_mode;
                self.reprocess_token = true;
            }
            Token::EndTag { name, .. } if name == "style" => {
                // Fetch first child node id. This should be the inline stylesheet text
                let style_node_id = current_node_id!(self);
                let Some(&style_text_node_id) = self.document.children(style_node_id).first() else {
                    self.open_elements.pop();
                    self.insertion_mode = self.original_insertion_mode;
                    return;
                };

                // Load stylesheet from text node
                if let Some(mut stylesheet) = self.load_inline_stylesheet(CssOrigin::Author, style_text_node_id) {
                    if let Some(media) = self.document.attribute(style_node_id, "media") {
                        C::CssSystem::set_stylesheet_media(&mut stylesheet, media);
                    }
                    self.add_stylesheet_for(style_node_id, stylesheet);
                }

                self.open_elements.pop();
                self.insertion_mode = self.original_insertion_mode;
            }
            Token::EndTag { name, .. } if name == "script" => {
                // @todo: If the active speculative HTML parser is null and the JavaScript execution context stack is empty, then perform a microtask checkpoint.

                self.open_elements.pop();
                self.insertion_mode = self.original_insertion_mode;

                let old_insertion_point = self.insertion_point;
                self.insertion_point = Some(self.tokenizer.get_location().offset);

                // Nothing runs the script yet, but a blocking script still waits for
                // the stylesheets before it.
                if self.current_script_blocks {
                    self.settle_stylesheets();
                }

                self.script_nesting_level += 1;

                // @todo: run the script here.

                self.script_nesting_level -= 1;
                if self.script_nesting_level == 0 {
                    self.parser_pause_flag = false;
                }

                self.insertion_point = old_insertion_point;
            }
            _ => {
                self.open_elements.pop();
                self.insertion_mode = self.original_insertion_mode;
            }
        }
    }
}
