//! Tree construction in HTML content: one handler per insertion mode.

use crate::parser::{location_after, Html5Parser, InsertionMode};
use crate::tokenizer::token::Token;
use gosub_interface::config::HasDocument;
use gosub_interface::document::Document;

mod after;
mod body;
mod head;
mod initial;
mod select;
mod table;
mod template;
mod text;

impl<C: HasDocument> Html5Parser<'_, C> {
    /// Process a token in HTML content
    pub(crate) fn process_html_content(&mut self) {
        if self.ignore_lf {
            if let Token::Text { text: value, location } = &mut self.current_token {
                if value.starts_with('\n') {
                    // We don't need to skip 1 char, but we can skip 1 byte, as we just checked for \n
                    value.remove(0);
                    *location = location_after(*location, "\n");
                }
            }
            self.ignore_lf = false;
        }

        // Move the token out for matching instead of deep-cloning it: tokens can carry
        // large script/style/rawtext strings and this runs for every token. While
        // processing, `self.current_token` is only read for its location (via
        // `parse_error`), so we leave a location-only placeholder. It is restored below
        // when the token must be reprocessed.
        let loc = self.current_token.get_location();
        let mut current_token = std::mem::replace(&mut self.current_token, Token::Eof { location: loc });
        self.current_token_rewritten = false;

        match self.insertion_mode {
            InsertionMode::Initial => self.handle_initial(&mut current_token),
            InsertionMode::BeforeHtml => self.handle_before_html(&mut current_token),
            InsertionMode::BeforeHead => self.handle_before_head(&mut current_token),
            InsertionMode::InHead => self.handle_in_head(&mut current_token),
            InsertionMode::InHeadNoscript => self.handle_in_head_noscript(&mut current_token),
            InsertionMode::AfterHead => self.handle_after_head(&mut current_token),
            InsertionMode::InBody => self.handle_in_body(&mut current_token),
            InsertionMode::Text => self.handle_text(&mut current_token),
            InsertionMode::InTable => self.handle_in_table(&mut current_token),
            InsertionMode::InTableText => self.handle_in_table_text(&mut current_token),
            InsertionMode::InCaption => self.handle_in_caption(&mut current_token),
            InsertionMode::InColumnGroup => self.handle_in_column_group(&mut current_token),
            InsertionMode::InTableBody => self.handle_in_table_body(&mut current_token),
            InsertionMode::InRow => self.handle_in_row(&mut current_token),
            InsertionMode::InCell => self.handle_in_cell(&mut current_token),
            InsertionMode::InSelect => self.handle_in_select(&mut current_token),
            InsertionMode::InSelectInTable => self.handle_in_select_in_table(&mut current_token),
            InsertionMode::InTemplate => self.handle_in_template(&mut current_token),
            InsertionMode::AfterBody => self.handle_after_body(&mut current_token),
            InsertionMode::InFrameset => self.handle_in_frameset(&mut current_token),
            InsertionMode::AfterFrameset => self.handle_after_frameset(&mut current_token),
            InsertionMode::AfterAfterBody => self.handle_after_after_body(&mut current_token),
            InsertionMode::AfterAfterFrameset => self.handle_after_after_frameset(&mut current_token),
        }

        // If this token is to be reprocessed, restore it so the next iteration re-reads
        // the real token rather than the placeholder we swapped in. When an arm installed
        // a replacement in place (the `<image>` -> `<img>` rewrite), that replacement must
        // survive instead, so we skip the restore in that case.
        if self.reprocess_token && !self.current_token_rewritten {
            self.current_token = current_token;
        }
    }

    /// Reset insertion mode based on all kind of rules
    pub(crate) fn reset_insertion_mode(&mut self) {
        if self.open_elements.is_empty() {
            self.insertion_mode = InsertionMode::InBody;
            return;
        }

        let mut last = false;
        let mut idx = self.open_elements.len() - 1;

        loop {
            let mut node_id = self.open_elements[idx];
            if idx == 0 {
                last = true;

                // fragment case: use context node (13.2.4). The context node is set whenever
                // fragment parsing starts, so the `else` cannot be reached - but asking for it
                // rather than asserting it means a parser built some other way degrades to the
                // node it already had instead of aborting the process.
                if self.is_fragment_case {
                    if let Some(context_node_id) = self.context_node_id {
                        node_id = context_node_id;
                    }
                }
            }
            match self.document.tag_name(node_id).unwrap_or_default() {
                "select" => {
                    if last {
                        self.insertion_mode = InsertionMode::InSelect;
                        return;
                    }

                    let mut ancestor_idx = idx;
                    loop {
                        if ancestor_idx == 0 {
                            self.insertion_mode = InsertionMode::InSelect;
                            return;
                        }

                        ancestor_idx -= 1;
                        let ancestor_id = self.open_elements[ancestor_idx];
                        match self.document.tag_name(ancestor_id).unwrap_or_default() {
                            "template" => {
                                self.insertion_mode = InsertionMode::InSelect;
                                return;
                            }
                            "table" => {
                                self.insertion_mode = InsertionMode::InSelectInTable;
                                return;
                            }
                            _ => {}
                        }
                    }
                }
                "td" | "th" if !last => {
                    self.insertion_mode = InsertionMode::InCell;
                    return;
                }
                "tr" => {
                    self.insertion_mode = InsertionMode::InRow;
                    return;
                }
                "tbody" | "thead" | "tfoot" => {
                    self.insertion_mode = InsertionMode::InTableBody;
                    return;
                }
                "caption" => {
                    self.insertion_mode = InsertionMode::InCaption;
                    return;
                }
                "colgroup" => {
                    self.insertion_mode = InsertionMode::InColumnGroup;
                    return;
                }
                "table" => {
                    self.insertion_mode = InsertionMode::InTable;
                    return;
                }
                "template" => {
                    self.insertion_mode = *self
                        .template_insertion_mode
                        .last()
                        .unwrap_or(&InsertionMode::InTemplate);
                    return;
                }
                "head" if !last => {
                    self.insertion_mode = InsertionMode::InHead;
                    return;
                }
                "body" => {
                    self.insertion_mode = InsertionMode::InBody;
                    return;
                }
                "frameset" => {
                    // fragment case
                    self.insertion_mode = InsertionMode::InFrameset;
                    return;
                }
                "html" => {
                    if self.head_element.is_none() {
                        // fragment case
                        self.insertion_mode = InsertionMode::BeforeHead;
                        return;
                    }
                    self.insertion_mode = InsertionMode::AfterHead;
                    return;
                }
                _ => {}
            }

            if last {
                // fragment case
                self.insertion_mode = InsertionMode::InBody;
                return;
            }

            idx -= 1;
        }
    }
}
