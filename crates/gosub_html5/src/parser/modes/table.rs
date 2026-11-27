//! The table insertion modes: in table, in table text, in caption, in column group, in table body,
//! in row and in cell.

use crate::node::HTML_NAMESPACE;
use crate::parser::{Html5Parser, InsertionMode, Scope};
use crate::tokenizer::token::Token;
use cow_utils::CowUtils;
use gosub_interface::config::HasDocument;
use gosub_interface::document::Document;
use gosub_shared::attributes::Attributes;
use gosub_shared::node::NodeId;

impl<C: HasDocument> Html5Parser<'_, C> {
    /// Handle insertion mode "`in_table`"
    pub(crate) fn handle_in_table(&mut self, token: &mut Token) {
        let mut anything_else = false;

        match &*token {
            Token::Text { .. }
                if ["table", "tbody", "template", "tfoot", "thead", "tr"]
                    .iter()
                    .any(|&node| node == node_tag!(self, current_node_id!(self))) =>
            {
                self.pending_table_character_tokens = String::new();
                self.original_insertion_mode = self.insertion_mode;
                self.insertion_mode = InsertionMode::InTableText;
                self.reprocess_token = true;
            }
            Token::Comment { .. } => {
                self.insert_comment_element(token, None);
            }
            Token::DocType { .. } => {
                self.parse_error("doctype not allowed in in table insertion mode");
                // ignore token
            }
            Token::StartTag { name, .. } if name == "caption" => {
                self.clear_stack_back_to_table_context();
                self.active_formatting_elements_push_marker();
                self.insert_html_element(token);
                self.insertion_mode = InsertionMode::InCaption;
            }
            Token::StartTag { name, .. } if name == "colgroup" => {
                self.clear_stack_back_to_table_context();
                self.insert_html_element(token);
                self.insertion_mode = InsertionMode::InColumnGroup;
            }
            Token::StartTag { name, .. } if name == "col" => {
                self.clear_stack_back_to_table_context();

                let mut token = Token::StartTag {
                    name: "colgroup".to_string(),
                    is_self_closing: false,
                    attributes: Attributes::new(),
                    location: token.get_location(),
                };
                self.insert_html_element(&mut token);

                self.insertion_mode = InsertionMode::InColumnGroup;
                self.reprocess_token = true;
            }
            Token::StartTag { name, .. } if name == "tbody" || name == "tfoot" || name == "thead" => {
                self.clear_stack_back_to_table_context();

                self.insert_html_element(token);

                self.insertion_mode = InsertionMode::InTableBody;
            }
            Token::StartTag { name, .. } if name == "td" || name == "th" || name == "tr" => {
                self.clear_stack_back_to_table_context();

                let mut token = Token::StartTag {
                    name: "tbody".to_string(),
                    is_self_closing: false,
                    attributes: Attributes::new(),
                    location: token.get_location(),
                };
                self.insert_html_element(&mut token);

                self.insertion_mode = InsertionMode::InTableBody;
                self.reprocess_token = true;
            }
            Token::StartTag { name, .. } if name == "table" => {
                self.parse_error("table tag not allowed in in table insertion mode");

                if !self.is_in_scope("table", HTML_NAMESPACE, Scope::Table) {
                    // ignore token
                    return;
                }

                self.pop_until_named("table");
                self.reset_insertion_mode();
                self.reprocess_token = true;
            }
            Token::EndTag { name, .. } if name == "table" => {
                if !self.is_in_scope("table", HTML_NAMESPACE, Scope::Table) {
                    self.parse_error("table end tag not allowed in in table insertion mode");
                    // ignore token
                    return;
                }

                self.pop_until_named("table");
                self.reset_insertion_mode();
            }
            Token::EndTag { name, .. }
                if name == "body"
                    || name == "caption"
                    || name == "col"
                    || name == "colgroup"
                    || name == "html"
                    || name == "tbody"
                    || name == "td"
                    || name == "tfoot"
                    || name == "th"
                    || name == "thead"
                    || name == "tr" =>
            {
                self.parse_error("end tag not allowed in in table insertion mode");
                // ignore token
                return;
            }
            Token::StartTag { name, .. } if name == "style" || name == "script" || name == "template" => {
                self.handle_in_head(token);
            }
            Token::EndTag { name, .. } if name == "template" => {
                self.handle_in_head(token);
            }
            Token::StartTag {
                name,
                is_self_closing,
                attributes,
                ..
            } if name == "input" => {
                if attributes.get("type").is_none_or(|t| t.cow_to_lowercase() != *"hidden") {
                    anything_else = true;
                } else {
                    self.parse_error("input tag not allowed in in table insertion mode");

                    self.acknowledge_closing_tag(*is_self_closing);

                    self.insert_html_element(token);
                    self.pop_check("input");
                }
            }
            Token::StartTag { name, .. } if name == "form" => {
                self.parse_error("form tag not allowed in in table insertion mode");

                if self.open_elements_has("template") || self.form_element.is_some() {
                    // ignore token
                    return;
                }

                let node_id = self.insert_html_element(token);
                self.form_element = Some(node_id);

                self.pop_check("form");
            }
            Token::Eof { .. } => {
                self.handle_in_body(token);
            }
            _ => anything_else = true,
        }

        if anything_else {
            self.parse_error("anything else not allowed in in table insertion mode");

            self.foster_parenting = true;
            self.handle_in_body(token);
            self.foster_parenting = false;
        }
    }

    /// Handle insertion mode "`in_table_text`"
    pub(crate) fn handle_in_table_text(&mut self, token: &mut Token) {
        match &*token {
            Token::Text { text: value, location } if token.is_mixed() => {
                let tokens = self.split_mixed_token(value, *location);
                self.tokenizer.insert_tokens_at_queue_start(tokens);
            }
            Token::Text { text: value, location } if token.is_null() => {
                self.null_parse_errors(
                    value,
                    *location,
                    "null character not allowed in in table text insertion mode",
                );
                // ignore token
            }
            Token::Text { text: value, location } => {
                if self.pending_table_character_tokens.is_empty() {
                    self.pending_table_character_location = *location;
                }
                self.pending_table_character_tokens.push_str(value);
            }
            _ => {
                let pending_chars = self.pending_table_character_tokens.clone();

                let mut process_as_intable_anything_else = false;

                for c in self.pending_table_character_tokens.chars() {
                    if !c.is_ascii_whitespace() {
                        self.parse_error("non whitespace character in pending table character tokens");
                        process_as_intable_anything_else = true;
                        break;
                    }
                }

                if process_as_intable_anything_else {
                    let tmp = token.clone();
                    self.foster_parenting = true;

                    let tokens = self.split_mixed_token(&pending_chars, self.pending_table_character_location);
                    for mut token in tokens {
                        self.current_token = token.clone();
                        self.handle_in_body(&mut token);
                    }

                    self.foster_parenting = false;
                    self.current_token = tmp;
                } else {
                    self.insert_text_element(&Token::Text {
                        text: pending_chars,
                        location: self.pending_table_character_location,
                    });
                }

                self.pending_table_character_tokens.clear();

                self.insertion_mode = self.original_insertion_mode;
                self.reprocess_token = true;
            }
        }
    }

    /// Handle insertion mode "`in_caption`"
    pub(crate) fn handle_in_caption(&mut self, token: &mut Token) {
        let mut process_incaption_body = false;

        match &*token {
            Token::EndTag { name, .. } if name == "caption" => {
                process_incaption_body = true;
            }
            Token::StartTag { name, .. }
                if [
                    "caption", "col", "colgroup", "tbody", "td", "tfoot", "th", "thead", "tr",
                ]
                .contains(&name.as_str()) =>
            {
                process_incaption_body = true;
                self.reprocess_token = true;
            }
            Token::EndTag { name, .. } if name == "table" => {
                process_incaption_body = true;
                self.reprocess_token = true;
            }
            Token::EndTag { name, .. }
                if name == "body"
                    || name == "col"
                    || name == "colgroup"
                    || name == "html"
                    || name == "tbody"
                    || name == "td"
                    || name == "tfoot"
                    || name == "th"
                    || name == "thead"
                    || name == "tr" =>
            {
                self.parse_error("end tag not allowed in in caption insertion mode");
                // ignore token
            }
            _ => self.handle_in_body(token),
        }

        if process_incaption_body {
            if !self.open_elements_has("caption") {
                // fragment case
                self.parse_error("caption end tag not allowed in in caption insertion mode");
                // ignore token
                self.reprocess_token = false;
                return;
            }

            self.generate_implied_end_tags(None, false);

            if node_tag!(self, current_node_id!(self)) != "caption" {
                self.parse_error("caption end tag not at top of stack");
            }

            self.pop_until_named("caption");
            self.active_formatting_elements_clear_until_marker();

            self.insertion_mode = InsertionMode::InTable;
        }
    }

    /// Handle insertion mode "`in_column_group`"
    pub(crate) fn handle_in_column_group(&mut self, token: &mut Token) {
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
                self.parse_error("doctype not allowed in column group insertion mode");
                // ignore token
            }
            Token::StartTag { name, .. } if name == "html" => {
                self.handle_in_body(token);
            }
            Token::StartTag {
                name, is_self_closing, ..
            } if name == "col" => {
                self.acknowledge_closing_tag(*is_self_closing);

                self.insert_html_element(token);
                self.open_elements.pop();
            }
            Token::StartTag { name, .. } if name == "template" => {
                self.handle_in_head(token);
            }
            Token::EndTag { name, .. } if name == "template" => {
                self.handle_in_head(token);
            }
            Token::Eof { .. } => {
                self.handle_in_body(token);
            }
            Token::EndTag { name, .. } if name == "colgroup" => {
                if node_tag!(self, current_node_id!(self)) != "colgroup" {
                    self.parse_error("colgroup end tag not at top of stack");
                    // ignore token
                    return;
                }

                self.open_elements.pop();
                self.insertion_mode = InsertionMode::InTable;
            }
            Token::EndTag { name, .. } if name == "col" => {
                self.parse_error("col end tag not allowed in column group insertion mode");
                // ignore token
            }
            _ => {
                if node_tag!(self, current_node_id!(self)) != "colgroup" {
                    self.parse_error("colgroup end tag not at top of stack");
                    // ignore token
                    return;
                }
                self.open_elements.pop();
                self.insertion_mode = InsertionMode::InTable;
                self.reprocess_token = true;
            }
        }
    }

    /// Handle insertion mode "`in_table_body`"
    pub(crate) fn handle_in_table_body(&mut self, token: &mut Token) {
        match &*token {
            Token::StartTag { name, .. } if name == "tr" => {
                self.clear_stack_back_to_table_body_context();

                self.insert_html_element(token);

                self.insertion_mode = InsertionMode::InRow;
            }
            Token::StartTag { name, .. } if name == "th" || name == "td" => {
                self.parse_error("th or td tag not allowed in in table body insertion mode");

                self.clear_stack_back_to_table_body_context();

                let mut implied = Token::StartTag {
                    name: "tr".to_string(),
                    is_self_closing: false,
                    attributes: Attributes::new(),
                    location: token.get_location(),
                };
                self.insert_html_element(&mut implied);

                self.insertion_mode = InsertionMode::InRow;
                self.reprocess_token = true;
            }
            Token::EndTag { name, .. } if name == "tbody" || name == "tfoot" || name == "thead" => {
                if !self.is_in_scope(name, HTML_NAMESPACE, Scope::Table) {
                    self.parse_error("tbody, tfoot or thead tag not allowed in in table body insertion mode");
                    // ignore token
                    return;
                }

                self.clear_stack_back_to_table_body_context();
                self.open_elements.pop();

                self.insertion_mode = InsertionMode::InTable;
            }
            Token::StartTag { name, .. }
                if ["caption", "col", "colgroup", "tbody", "tfoot", "thead"].contains(&name.as_str()) =>
            {
                if !self.is_in_scope("tbody", HTML_NAMESPACE, Scope::Table)
                    && !self.is_in_scope("tfoot", HTML_NAMESPACE, Scope::Table)
                    && !self.is_in_scope("thead", HTML_NAMESPACE, Scope::Table)
                {
                    self.parse_error(
                        "caption, col, colgroup, tbody, tfoot or thead tag not allowed in in table body insertion mode",
                    );
                    // ignore token
                    return;
                }

                self.clear_stack_back_to_table_body_context();
                self.open_elements.pop();

                self.insertion_mode = InsertionMode::InTable;
                self.reprocess_token = true;
            }
            Token::EndTag { name, .. } if name == "table" => {
                if !self.is_in_scope("tbody", HTML_NAMESPACE, Scope::Table)
                    && !self.is_in_scope("tfoot", HTML_NAMESPACE, Scope::Table)
                    && !self.is_in_scope("thead", HTML_NAMESPACE, Scope::Table)
                {
                    self.parse_error("table end tag not allowed in in table body insertion mode");
                    return;
                }

                self.clear_stack_back_to_table_body_context();
                self.open_elements.pop();

                self.insertion_mode = InsertionMode::InTable;
                self.reprocess_token = true;
            }
            Token::EndTag { name, .. }
                if ["body", "caption", "col", "colgroup", "html", "td", "th", "tr"].contains(&name.as_str()) =>
            {
                self.parse_error("end tag not allowed in in table body insertion mode");
                // ignore token
            }
            _ => {
                self.handle_in_table(token);
            }
        }
    }

    /// Handle insertion mode "`in_row`"
    pub(crate) fn handle_in_row(&mut self, token: &mut Token) {
        match &*token {
            Token::StartTag { name, .. } if name == "th" || name == "td" => {
                self.clear_stack_back_to_table_row_context();

                self.insert_html_element(token);

                self.insertion_mode = InsertionMode::InCell;
                self.active_formatting_elements_push_marker();
            }
            Token::EndTag { name, .. } if name == "tr" => {
                if !self.is_in_scope("tr", HTML_NAMESPACE, Scope::Table) {
                    self.parse_error("tr tag not allowed in in row insertion mode");
                    // ignore token
                    return;
                }

                self.clear_stack_back_to_table_row_context();
                self.pop_check("tr");

                self.insertion_mode = InsertionMode::InTableBody;
            }
            Token::StartTag { name, .. }
                if ["caption", "col", "colgroup", "tbody", "tfoot", "thead", "tr"].contains(&name.as_str()) =>
            {
                if !self.is_in_scope("tr", HTML_NAMESPACE, Scope::Table) {
                    self.parse_error(
                        "caption, col, colgroup, tbody, tfoot or thead tag not allowed in in row insertion mode",
                    );
                    // ignore token
                    return;
                }

                self.clear_stack_back_to_table_row_context();
                self.pop_check("tr");

                self.insertion_mode = InsertionMode::InTableBody;
                self.reprocess_token = true;
            }
            Token::EndTag { name, .. } if name == "table" => {
                if !self.is_in_scope("tr", HTML_NAMESPACE, Scope::Table) {
                    self.parse_error("table tag not allowed in in row insertion mode");
                    // ignore token
                    return;
                }

                self.clear_stack_back_to_table_row_context();
                self.pop_check("tr");

                self.insertion_mode = InsertionMode::InTableBody;
                self.reprocess_token = true;
            }
            Token::EndTag { name, .. } if name == "tbody" || name == "tfoot" || name == "thead" => {
                if !self.is_in_scope(name, HTML_NAMESPACE, Scope::Table) {
                    self.parse_error("tbody, tfoot or thead tag not allowed in in table body insertion mode");
                    // ignore token
                    return;
                }

                if !self.is_in_scope("tr", HTML_NAMESPACE, Scope::Table) {
                    // ignore token
                    return;
                }

                self.clear_stack_back_to_table_row_context();
                self.pop_check("tr");

                self.insertion_mode = InsertionMode::InTableBody;
                self.reprocess_token = true;
            }
            Token::EndTag { name, .. }
                if name == "body"
                    || name == "caption"
                    || name == "col"
                    || name == "colgroup"
                    || name == "html"
                    || name == "td"
                    || name == "th" =>
            {
                self.parse_error("end tag not allowed in in row insertion mode");
                // ignore token
            }
            _ => self.handle_in_table(token),
        }
    }

    /// Handle insertion mode "`in_cell`"
    pub(crate) fn handle_in_cell(&mut self, token: &mut Token) {
        match &*token {
            Token::EndTag { name, .. } if name == "th" || name == "td" => {
                let token_name = name.clone();

                if !self.is_in_scope(name.as_str(), HTML_NAMESPACE, Scope::Table) {
                    self.parse_error("th or td tag not allowed in in cell insertion mode");
                    // ignore token
                    return;
                }
                self.generate_implied_end_tags(None, false);

                if node_tag!(self, current_node_id!(self)) != token_name {
                    self.parse_error("current node should be th or td");
                }

                self.pop_until_named(&token_name);

                self.active_formatting_elements_clear_until_marker();

                self.insertion_mode = InsertionMode::InRow;
            }
            Token::StartTag { name, .. }
                if [
                    "caption", "col", "colgroup", "tbody", "td", "tfoot", "th", "thead", "tr",
                ]
                .contains(&name.as_str()) =>
            {
                if !self.is_in_scope("td", HTML_NAMESPACE, Scope::Table)
                    && !self.is_in_scope("th", HTML_NAMESPACE, Scope::Table)
                {
                    // fragment case
                    self.parse_error(
                        "caption, col, colgroup, tbody, tfoot or thead tag not allowed in in cell insertion mode",
                    );
                    // ignore token
                    return;
                }

                self.close_cell();
                self.reprocess_token = true;
            }
            Token::EndTag { name, .. }
                if name == "body" || name == "caption" || name == "col" || name == "colgroup" || name == "html" =>
            {
                self.parse_error("end tag not allowed in in cell insertion mode");
                // ignore token
            }
            Token::EndTag { name, .. }
                if name == "table" || name == "tbody" || name == "tfoot" || name == "thead" || name == "tr" =>
            {
                if !self.is_in_scope(name.as_str(), HTML_NAMESPACE, Scope::Table) {
                    self.parse_error("tbody, tfoot or thead tag not allowed in in table body insertion mode");
                    // ignore token
                    return;
                }

                self.close_cell();
                self.reprocess_token = true;
            }
            _ => self.handle_in_body(token),
        }
    }

    /// Closes a table cell and switches the insertion mode to `InRow`
    pub(crate) fn close_cell(&mut self) {
        self.generate_implied_end_tags(None, false);

        let tag = self.html_tag_name(current_node_id!(self));
        if tag != "td" && tag != "th" {
            self.parse_error("current node should be td or th");
        }

        self.pop_until_any(&["td", "th"]);

        self.active_formatting_elements_clear_until_marker();
        self.insertion_mode = InsertionMode::InRow;
    }

    /// Pop all elements back to a table context
    pub(crate) fn clear_stack_back_to_table_context(&mut self) {
        while !self.open_elements.is_empty() {
            if ["table", "template", "html"].contains(&node_tag!(self, current_node_id!(self))) {
                return;
            }
            self.open_elements.pop();
        }
    }

    /// Pop all elements back to a table context
    pub(crate) fn clear_stack_back_to_table_body_context(&mut self) {
        while !self.open_elements.is_empty() {
            if ["tbody", "tfoot", "thead", "template", "html"].contains(&node_tag!(self, current_node_id!(self))) {
                return;
            }
            self.open_elements.pop();
        }
    }

    /// Pop all elements back to a table row context
    pub(crate) fn clear_stack_back_to_table_row_context(&mut self) {
        while !self.open_elements.is_empty() {
            let cn_id = current_node_id!(self);
            if ["tr", "template", "html"].contains(&node_tag!(self, cn_id)) {
                return;
            }
            self.open_elements.pop();
        }
    }
}
