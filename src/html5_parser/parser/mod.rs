mod attr_replacements;
pub mod document;
mod quirks;

// ------------------------------------------------------------

use crate::html5_parser::error_logger::{ErrorLogger, ParseError, ParserError};
use crate::html5_parser::input_stream::InputStream;
use crate::html5_parser::node::{Node, NodeData, HTML_NAMESPACE, MATHML_NAMESPACE, SVG_NAMESPACE};
use crate::html5_parser::parser::attr_replacements::{
    MATHML_ADJUSTMENTS, SVG_ADJUSTMENTS, XML_ADJUSTMENTS,
};
use crate::html5_parser::parser::document::{Document, DocumentType};
use crate::html5_parser::parser::quirks::QuirksMode;
use crate::html5_parser::tokenizer::state::State;
use crate::html5_parser::tokenizer::token::Token;
use crate::html5_parser::tokenizer::{Tokenizer, CHAR_NUL};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

// Insertion modes as defined in 13.2.4.1
#[derive(Debug, Copy, Clone, PartialEq)]
enum InsertionMode {
    Initial,
    BeforeHtml,
    BeforeHead,
    InHead,
    InHeadNoscript,
    AfterHead,
    InBody,
    Text,
    InTable,
    InTableText,
    InCaption,
    InColumnGroup,
    InTableBody,
    InRow,
    InCell,
    InSelect,
    InSelectInTable,
    InTemplate,
    AfterBody,
    InFrameset,
    AfterFrameset,
    AfterAfterBody,
    AfterAfterFrameset,
}

// Additional extensions to the Vec type so we can do some stack operations
trait VecExtensions<T> {
    fn pop_until<F>(&mut self, f: F)
    where
        F: FnMut(&T) -> bool;
    fn pop_check<F>(&mut self, f: F) -> bool
    where
        F: FnMut(&T) -> bool;
}

impl VecExtensions<usize> for Vec<usize> {
    fn pop_until<F>(&mut self, mut f: F)
    where
        F: FnMut(&usize) -> bool,
    {
        while let Some(top) = self.last() {
            if f(top) {
                break;
            }
            self.pop();
        }
    }

    fn pop_check<F>(&mut self, mut f: F) -> bool
    where
        F: FnMut(&usize) -> bool,
    {
        match self.pop() {
            Some(popped_value) => f(&popped_value),
            None => false,
        }
    }
}

macro_rules! acknowledge_closing_tag {
    ($self:expr, $is_self_closing:expr) => {
        if $is_self_closing {
            $self.ack_self_closing = true;
        }
    };
}

// Pops the last element from the open elements until we reach $name
macro_rules! pop_until {
    ($self:expr, $name:expr) => {
        loop {
            $self.open_elements.pop();
            if current_node!($self).name != $name {
                break;
            }
        }
        // $self.open_elements.pop_until(|node_id| $self.document.get_node_by_id(*node_id).expect("node not found").name == $name);
    };
}

// Pops the last element from the open elements until we reach any of the elements in $arr
macro_rules! pop_until_any {
    ($self:expr, $arr:expr) => {
        $self.open_elements.pop_until(|node_id| {
            $arr.contains(
                &$self
                    .document
                    .get_node_by_id(*node_id)
                    .expect("node not found")
                    .name
                    .as_str(),
            )
        });
    };
}

// Remove the given node_id from the open elements stack
macro_rules! open_elements_remove {
    ($self:expr, $target_node_id: expr) => {
        $self.open_elements.retain(|&node_id| node_id != $target_node_id);
    };
}

// Pops the last element from the open elements, and panics if it is not $name
macro_rules! pop_check {
    ($self:expr, $name:expr) => {
        if !$self.open_elements.pop_check(|&node_id| {
            $self
                .document
                .get_node_by_id(node_id)
                .expect("node not found")
                .name
                == $name
        }) {
            panic!("{} tag should be popped from open elements", $name);
        }
    };
}

// Checks if the last element on the open elements is $name, and panics if not
macro_rules! check_last_element {
    ($self:expr, $name:expr) => {
        let node_id = $self.open_elements.last().unwrap_or(&0);
        if $self
            .document
            .get_node_by_id(*node_id)
            .expect("node not found")
            .name
            != "$name"
        {
            panic!("$name tag should be last element in open elements");
        }
    };
}

// Get the idx element from the open elements stack
macro_rules! open_elements_get {
    ($self:expr, $idx:expr) => {
        $self
            .document
            .get_node_by_id($self.open_elements[$idx])
            .expect("Open element not found")
    };
}

// Returns true when the open elements has $name
macro_rules! open_elements_has {
    ($self:expr, $name:expr) => {
        $self.open_elements.iter().rev().any(|node_id| {
            $self
                .document
                .get_node_by_id(*node_id)
                .expect("node not found")
                .name
                == $name
        })
    };
}

// Returns the current node: the last node in the open elements list
macro_rules! current_node {
    ($self:expr) => {{
        let current_node_idx = $self.open_elements.last().unwrap_or(&0);
        $self
            .document
            .get_node_by_id(*current_node_idx)
            .expect("Current node not found")
    }};
}

// Returns the current node as a mutable reference
macro_rules! current_node_mut {
    ($self:expr) => {{
        let current_node_idx = $self.open_elements.last().unwrap_or(&0);
        $self
            .document
            .get_mut_node_by_id(*current_node_idx)
            .expect("Current node not found")
    }};
}

#[macro_use]
mod adoption_agency;

// Active formatting elements, which could be a regular node(id), or a marker
#[derive(PartialEq, Clone, Copy)]
enum ActiveElement {
    NodeId(usize),
    Marker,
}

impl ActiveElement {
    fn node_id(&self) -> Option<usize> {
        match self {
            ActiveElement::NodeId(id) => Some(*id),
            _ => None,
        }
    }
}


// The main parser object
pub struct Html5Parser<'a> {
    tokenizer: Tokenizer<'a>,                       // tokenizer object
    insertion_mode: InsertionMode,                  // current insertion mode
    original_insertion_mode: InsertionMode,         // original insertion mode (used for text mode)
    template_insertion_mode: Vec<InsertionMode>,    // template insertion mode stack
    parser_cannot_change_mode: bool,                // ??
    current_token: Token,                           // Current token from the tokenizer
    reprocess_token: bool, // If true, the current token should be processed again
    open_elements: Vec<usize>, // Stack of open elements
    head_element: Option<usize>, // Current head element
    form_element: Option<usize>, // Current form element
    scripting_enabled: bool, // If true, scripting is enabled
    frameset_ok: bool,     // if true, we can insert a frameset
    foster_parenting: bool, // Foster parenting flag
    script_already_started: bool, // If true, the script engine has already started
    pending_table_character_tokens: Vec<char>, // Pending table character tokens
    ack_self_closing: bool, // Acknowledge self closing tags
    active_formatting_elements: Vec<ActiveElement>, // List of active formatting elements or markers
    is_fragment_case: bool,                         // Is the current parsing a fragment case
    document: Document,                             // A reference to the document we are parsing
    error_logger: Rc<RefCell<ErrorLogger>>,         // Error logger, which is shared with the tokenizer
}

// Defines the scopes for in_scope()
enum Scope {
    Regular,
    ListItem,
    Button,
    Table,
    Select,
}

impl<'a> Html5Parser<'a> {
    // Creates a new parser object with the given input stream
    pub fn new(stream: &'a mut InputStream) -> Self {
        // Create a new error logger that will be used in both the tokenizer and the parser
        let error_logger = Rc::new(RefCell::new(ErrorLogger::new()));

        let tokenizer = Tokenizer::new(stream, None, error_logger.clone());

        Html5Parser {
            tokenizer,
            insertion_mode: InsertionMode::Initial,
            original_insertion_mode: InsertionMode::Initial,
            template_insertion_mode: vec![],
            parser_cannot_change_mode: false,
            current_token: Token::EofToken,
            reprocess_token: false,
            open_elements: Vec::new(),
            head_element: None,
            form_element: None,
            scripting_enabled: true,
            frameset_ok: true,
            foster_parenting: false,
            script_already_started: false,
            pending_table_character_tokens: vec![],
            ack_self_closing: false,
            active_formatting_elements: vec![],
            is_fragment_case: false,
            error_logger,
            document: Document::new(),
        }
    }

    // Parses the input stream into a Node tree
    pub fn parse(&mut self) -> (&Document, Vec<ParseError>) {
        loop {
            // If reprocess_token is true, we should process the same token again
            if !self.reprocess_token {
                self.current_token = self.tokenizer.next_token();
            }
            self.reprocess_token = false;

            // Break when we reach the end of the token stream
            if self.current_token.is_eof() {
                break;
            }

            self.display_debug_info();

            // println!("Token: {}", self.current_token);

            match self.insertion_mode {
                // Checked: 1
                InsertionMode::Initial => {
                    let mut anything_else = false;

                    match &self.current_token.clone() {
                        Token::TextToken { .. } if self.current_token.is_empty_or_white() => {
                            // ignore token
                            continue;
                        }
                        Token::CommentToken { .. } => {
                            let node = self.create_node(&self.current_token, HTML_NAMESPACE);
                            // add to end of the document(node)
                            self.document.add_node(node, 0);
                        }
                        Token::DocTypeToken {
                            name,
                            pub_identifier,
                            sys_identifier,
                            force_quirks,
                        } => {
                            if name.is_some() && name.as_ref().unwrap() != "html"
                                || pub_identifier.is_some()
                                || (sys_identifier.is_some()
                                    && sys_identifier.as_ref().unwrap() != "about:legacy-compat")
                            {
                                self.parse_error("doctype not allowed in initial insertion mode");
                            }

                            self.insert_html_element(&self.current_token.clone());

                            if self.document.doctype != DocumentType::IframeSrcDoc
                                && self.parser_cannot_change_mode
                            {
                                self.document.quirks_mode = self.identify_quirks_mode(
                                    name,
                                    pub_identifier.clone(),
                                    sys_identifier.clone(),
                                    *force_quirks,
                                );
                            }

                            self.insertion_mode = InsertionMode::BeforeHtml;
                        }
                        Token::StartTagToken { .. } => {
                            if self.document.doctype != DocumentType::IframeSrcDoc {
                                self.parse_error(
                                    ParserError::ExpectedDocTypeButGotStartTag.as_str(),
                                );
                            }
                            anything_else = true;
                        }
                        Token::EndTagToken { .. } => {
                            if self.document.doctype != DocumentType::IframeSrcDoc {
                                self.parse_error(ParserError::ExpectedDocTypeButGotEndTag.as_str());
                            }
                            anything_else = true;
                        }
                        Token::TextToken { .. } => {
                            if self.document.doctype != DocumentType::IframeSrcDoc {
                                self.parse_error(ParserError::ExpectedDocTypeButGotChars.as_str());
                            }
                            anything_else = true;
                        }
                        _ => anything_else = true,
                    }

                    if anything_else {
                        if self.parser_cannot_change_mode {
                            self.document.quirks_mode = QuirksMode::Quirks;
                        }

                        self.insertion_mode = InsertionMode::BeforeHtml;
                        self.reprocess_token = true;
                    }
                }
                // Checked: 1
                InsertionMode::BeforeHtml => {
                    let mut anything_else = false;

                    match &self.current_token {
                        Token::DocTypeToken { .. } => {
                            self.parse_error("doctype not allowed in before html insertion mode");
                        }
                        Token::CommentToken { .. } => {
                            let node = self.create_node(&self.current_token, HTML_NAMESPACE);
                            self.document.add_node(node, 0);
                        }
                        Token::TextToken { .. } if self.current_token.is_empty_or_white() => {
                            // ignore token
                        }
                        Token::StartTagToken { name, .. } if name == "html" => {
                            self.insert_html_element(&self.current_token.clone());

                            self.insertion_mode = InsertionMode::BeforeHead;
                        }
                        Token::EndTagToken { name, .. }
                            if name == "head"
                                || name == "body"
                                || name == "html"
                                || name == "br" =>
                        {
                            anything_else = true;
                        }
                        Token::EndTagToken { .. } => {
                            self.parse_error("end tag not allowed in before html insertion mode");
                        }
                        _ => {
                            anything_else = true;
                        }
                    }

                    if anything_else {
                        let token = Token::StartTagToken {
                            name: "html".to_string(),
                            is_self_closing: false,
                            attributes: HashMap::new(),
                        };
                        self.insert_html_element(&token);

                        self.insertion_mode = InsertionMode::BeforeHead;
                        self.reprocess_token = true;
                    }
                }
                // Checked: 1
                InsertionMode::BeforeHead => {
                    let mut anything_else = false;

                    match &self.current_token {
                        Token::TextToken { .. } if self.current_token.is_empty_or_white() => {
                            // ignore token
                        }
                        Token::CommentToken { .. } => {
                            let node = self.create_node(&self.current_token, HTML_NAMESPACE);
                            self.document.add_node(node, current_node!(self).id);
                        }
                        Token::DocTypeToken { .. } => {
                            self.parse_error("doctype not allowed in before head insertion mode");
                            // ignore token
                        }
                        Token::StartTagToken { name, .. } if name == "html" => {
                            self.handle_in_body();
                        }
                        Token::StartTagToken { name, .. } if name == "head" => {
                            let node_id = self.insert_html_element(&self.current_token.clone());
                            self.head_element = Some(node_id);
                            self.insertion_mode = InsertionMode::InHead;
                        }
                        Token::EndTagToken { name, .. }
                            if name == "head"
                                || name == "body"
                                || name == "html"
                                || name == "br" =>
                        {
                            anything_else = true;
                        }
                        Token::EndTagToken { .. } => {
                            self.parse_error("end tag not allowed in before head insertion mode");
                            // ignore token
                        }
                        _ => {
                            anything_else = true;
                        }
                    }
                    if anything_else {
                        let token = Token::StartTagToken {
                            name: "head".to_string(),
                            is_self_closing: false,
                            attributes: HashMap::new(),
                        };
                        let node_id = self.insert_html_element(&token);
                        self.head_element = Some(node_id);
                        self.insertion_mode = InsertionMode::InHead;
                        self.reprocess_token = true;
                    }
                }
                // Checked: 1
                InsertionMode::InHead => self.handle_in_head(),
                // Checked: 1
                InsertionMode::InHeadNoscript => {
                    let mut anything_else = false;

                    match &self.current_token {
                        Token::DocTypeToken { .. } => {
                            self.parse_error(
                                "doctype not allowed in 'head no script' insertion mode",
                            );
                            // ignore token
                            continue;
                        }
                        Token::StartTagToken { name, .. } if name == "html" => {
                            self.handle_in_body();
                        }
                        Token::EndTagToken { name, .. } if name == "noscript" => {
                            pop_check!(self, "noscript");
                            check_last_element!(self, "head");
                            self.insertion_mode = InsertionMode::InHead;
                        }
                        Token::TextToken { .. } if self.current_token.is_empty_or_white() => {
                            self.handle_in_head();
                        }
                        Token::CommentToken { .. } => {
                            self.handle_in_head();
                        }
                        Token::StartTagToken { name, .. }
                            if name == "basefont"
                                || name == "bgsound"
                                || name == "link"
                                || name == "meta"
                                || name == "noframes"
                                || name == "style" =>
                        {
                            self.handle_in_head();
                        }
                        Token::EndTagToken { name, .. } if name == "br" => {
                            anything_else = true;
                        }
                        Token::StartTagToken { name, .. }
                            if name == "head" || name == "noscript" =>
                        {
                            self.parse_error(
                                "head or noscript tag not allowed in after head insertion mode",
                            );
                            // ignore token
                            continue;
                        }
                        Token::EndTagToken { .. } => {
                            self.parse_error("end tag not allowed in after head insertion mode");
                            // ignore token
                            continue;
                        }
                        _ => {
                            anything_else = true;
                        }
                    }
                    if anything_else {
                        self.parse_error("anything else not allowed in after head insertion mode");

                        pop_check!(self, "noscript");
                        check_last_element!(self, "head");

                        self.insertion_mode = InsertionMode::InHead;
                        self.reprocess_token = true;
                    }
                }
                // Checked: 1
                InsertionMode::AfterHead => {
                    let mut anything_else = false;

                    match &self.current_token {
                        Token::TextToken { .. } if self.current_token.is_empty_or_white() => {
                            let node = self.create_node(&self.current_token, HTML_NAMESPACE);
                            self.document.add_node(node, current_node!(self).id);
                        }
                        Token::CommentToken { .. } => {
                            let node = self.create_node(&self.current_token, HTML_NAMESPACE);
                            self.document.add_node(node, current_node!(self).id);
                        }
                        Token::DocTypeToken { .. } => {
                            self.parse_error("doctype not allowed in after head insertion mode");
                            // ignore token
                        }
                        Token::StartTagToken { name, .. } if name == "html" => {
                            self.handle_in_body();
                        }
                        Token::StartTagToken { name, .. } if name == "body" => {
                            self.insert_html_element(&self.current_token.clone());

                            self.frameset_ok = false;
                            self.insertion_mode = InsertionMode::InBody;
                        }
                        Token::StartTagToken { name, .. } if name == "frameset" => {
                            self.insert_html_element(&self.current_token.clone());

                            self.insertion_mode = InsertionMode::InFrameset;
                        }
                        Token::StartTagToken { name, .. }
                            if [
                                "base",
                                "basefront",
                                "bgsound",
                                "link",
                                "meta",
                                "noframes",
                                "script",
                                "style",
                                "template",
                                "title",
                            ]
                            .contains(&name.as_str()) =>
                        {
                            self.parse_error("invalid start tag in after head insertion mode");

                            if self.head_element.is_none() {
                                panic!("Head element should not be None");
                            }

                            if let Some(node_id) = self.head_element {
                                self.open_elements.push(node_id);
                            }

                            self.handle_in_head();

                            // Remove the node pointed to by the head element pointer from the stack of open elements (might not be current node at this point)
                            if let Some(node_id) = self.head_element {
                                self.open_elements.retain(|&x| x != node_id);
                            }
                        }
                        Token::EndTagToken { name, .. } if name == "template" => {
                            self.handle_in_head();
                        }
                        Token::EndTagToken { name, .. }
                            if name == "body" || name == "html" || name == "br" =>
                        {
                            anything_else = true;
                        }
                        Token::StartTagToken { name, .. } if name == "head" => {
                            self.parse_error("head tag not allowed in after head insertion mode");
                            // ignore token
                        }
                        Token::EndTagToken { .. } => {
                            self.parse_error("end tag not allowed in after head insertion mode");
                            // Ignore token
                        }
                        _ => {
                            anything_else = true;
                        }
                    }

                    if anything_else {
                        let token = Token::StartTagToken {
                            name: "body".to_string(),
                            is_self_closing: false,
                            attributes: HashMap::new(),
                        };
                        self.insert_html_element(&token);

                        self.insertion_mode = InsertionMode::InBody;
                        self.reprocess_token = true;
                    }
                }
                // Checked:
                InsertionMode::InBody => self.handle_in_body(),
                // Checked: 1
                InsertionMode::Text => {
                    match &self.current_token {
                        Token::TextToken { .. } => {
                            let node = self.create_node(&self.current_token, HTML_NAMESPACE);
                            self.document.add_node(node, current_node!(self).id);
                        }
                        Token::EofToken => {
                            self.parse_error("eof not allowed in text insertion mode");

                            if current_node!(self).name == "script" {
                                self.script_already_started = true;
                            }
                            self.open_elements.pop();
                            self.insertion_mode = self.original_insertion_mode;
                            self.reprocess_token = true;
                        }
                        Token::EndTagToken { name, .. } if name == "script" => {
                            // @TODO: do script stuff!!!!
                        }
                        _ => {
                            self.open_elements.pop();
                            self.insertion_mode = self.original_insertion_mode;
                        }
                    }
                }
                // Checked: 1
                InsertionMode::InTable => self.handle_in_table(),
                // Checked: 1
                InsertionMode::InTableText => {
                    match &self.current_token {
                        Token::TextToken { .. } if self.current_token.is_null() => {
                            self.parse_error(
                                "null character not allowed in in table text insertion mode",
                            );
                            // ignore token
                        }
                        Token::TextToken { value, .. } => {
                            for c in value.chars() {
                                if c == CHAR_NUL {
                                    self.parse_error(
                                        "null character not allowed in in table insertion mode",
                                    );
                                } else {
                                    self.pending_table_character_tokens.push(c);
                                }
                            }
                        }
                        _ => {
                            // @TODO: this needs to check if there are any non-whitespaces, if so then reprocess using anything_else in "in_table"
                            self.flush_pending_table_character_tokens();

                            self.insertion_mode = self.original_insertion_mode;
                            self.reprocess_token = true;
                        }
                    }
                }
                // Checked: 1
                InsertionMode::InCaption => {
                    let mut process_incaption_body = false;

                    match &self.current_token {
                        Token::EndTagToken { name, .. } if name == "caption" => {
                            process_incaption_body = true;
                        }
                        Token::StartTagToken { name, .. }
                            if [
                                "caption", "col", "colgroup", "tbody", "td", "tfoot", "th",
                                "thead", "tr",
                            ]
                            .contains(&name.as_str()) =>
                        {
                            process_incaption_body = true;
                            self.reprocess_token = true;
                        }
                        Token::EndTagToken { name, .. } if name == "table" => {
                            process_incaption_body = true;
                            self.reprocess_token = true;
                        }
                        Token::EndTagToken { name, .. }
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
                        _ => self.handle_in_body(),
                    }

                    if process_incaption_body {
                        if !open_elements_has!(self, "caption") {
                            self.parse_error(
                                "caption end tag not allowed in in caption insertion mode",
                            );
                            // ignore token
                            self.reprocess_token = false;
                            continue;

                            // @TODO: check what fragment case means
                        }

                        self.generate_all_implied_end_tags(None, false);

                        if current_node!(self).name != "caption" {
                            self.parse_error("caption end tag not at top of stack");
                            continue;
                        }

                        pop_until!(self, "caption");
                        self.active_formatting_elements_clear_until_marker();

                        self.insertion_mode = InsertionMode::InTable;
                    }
                }
                // Checked: 1
                InsertionMode::InColumnGroup => {
                    match &self.current_token {
                        Token::TextToken { .. } if self.current_token.is_empty_or_white() => {
                            let node = self.create_node(&self.current_token, HTML_NAMESPACE);
                            self.document.add_node(node, current_node!(self).id);
                        }
                        Token::CommentToken { .. } => {
                            let node = self.create_node(&self.current_token, HTML_NAMESPACE);
                            self.document.add_node(node, current_node!(self).id);
                        }
                        Token::DocTypeToken { .. } => {
                            self.parse_error("doctype not allowed in column group insertion mode");
                            // ignore token
                        }
                        Token::StartTagToken { name, .. } if name == "html" => {
                            self.handle_in_body();
                        }
                        Token::StartTagToken {
                            name,
                            is_self_closing,
                            ..
                        } if name == "col" => {
                            acknowledge_closing_tag!(self, *is_self_closing);

                            self.insert_html_element(&self.current_token.clone());
                            self.open_elements.pop();
                        }
                        Token::StartTagToken { name, .. } if name == "template" => {
                            self.handle_in_head();
                        }
                        Token::EndTagToken { name, .. } if name == "template" => {
                            self.handle_in_head();
                        }
                        Token::EndTagToken { name, .. } if name == "colgroup" => {
                            if current_node!(self).name != "colgroup" {
                                self.parse_error("colgroup end tag not at top of stack");
                                // ignore token
                                continue;
                            }

                            self.open_elements.pop();
                            self.insertion_mode = InsertionMode::InTable;
                        }
                        Token::EndTagToken { name, .. } if name == "col" => {
                            self.parse_error(
                                "col end tag not allowed in column group insertion mode",
                            );
                            // ignore token
                        }
                        _ => {
                            if current_node!(self).name != "colgroup" {
                                self.parse_error("colgroup end tag not at top of stack");
                                // ignore token
                                continue;
                            }
                            self.open_elements.pop();
                            self.insertion_mode = InsertionMode::InTable;
                            self.reprocess_token = true;
                        }
                    }

                    //     Token::StartTagToken { name, .. } if name == "frameset" => {
                    //         self.insert_html_element(&self.current_token);
                    //
                    //         self.insertion_mode = InsertionMode::InFrameset;
                    //     },
                    //
                    //     Token::StartTagToken { name, .. } if ["base", "basefront", "bgsound", "link", "meta", "noframes", "script", "style", "template", "title"].contains(&name.as_str()) => {
                    //         self.parse_error("invalid start tag in after head insertion mode");
                    //
                    //         if let Some(ref value) = self.head_element {
                    //             self.open_elements.push(value.clone());
                    //         }
                    //
                    //         self.handle_in_head();
                    //
                    //         // remove the node pointed to by the head element pointer from the stack of open elements (might not be current node at this point)
                    //     }
                    //     Token::EndTagToken { name, .. } if name == "template" => {
                    //         self.handle_in_head();
                    //     }
                    //     Token::EndTagToken { name, .. } if name == "body" || name == "html" || name == "br"=> {
                    //         anything_else = true;
                    //     }
                    //     Token::StartTagToken { name, .. } if name == "head" => {
                    //         self.parse_error("head tag not allowed in after head insertion mode");
                    //     }
                    //     Token::EndTagToken { .. }  => {
                    //         self.parse_error("end tag not allowed in after head insertion mode");
                    //     }
                    //     _ => {
                    //         anything_else = true;
                    //     }
                    // }
                    //
                    // if anything_else {
                    //     let token = Token::StartTagToken { name: "body".to_string(), is_self_closing: false, attributes: HashMap::new() };
                    //     self.insert_html_element(&token);
                    //
                    //     self.insertion_mode = InsertionMode::InBody;
                    //     self.reprocess_token = true;
                    // }
                }
                // Checked: 1
                InsertionMode::InTableBody => {
                    match &self.current_token {
                        Token::StartTagToken { name, .. } if name == "tr" => {
                            self.clear_stack_back_to_table_body_context();

                            self.insert_html_element(&self.current_token.clone());

                            self.insertion_mode = InsertionMode::InRow;
                        }
                        Token::StartTagToken { name, .. } if name == "th" || name == "td" => {
                            self.parse_error(
                                "th or td tag not allowed in in table body insertion mode",
                            );

                            self.clear_stack_back_to_table_body_context();

                            let token = Token::StartTagToken {
                                name: "tr".to_string(),
                                is_self_closing: false,
                                attributes: HashMap::new(),
                            };
                            self.insert_html_element(&token);

                            self.insertion_mode = InsertionMode::InRow;
                            self.reprocess_token = true;
                        },
                        Token::StartTagToken { name, .. } if name == "tbody" || name == "tfoot" || name == "thead" => {
                            if ! self.is_in_scope(name, Scope::Table) {
                                self.parse_error("tbody, tfoot or thead tag not allowed in in table body insertion mode");
                                // ignore token
                                continue;
                            }

                            self.clear_stack_back_to_table_body_context();
                            self.open_elements.pop();

                            self.insertion_mode = InsertionMode::InTable;
                        },
                        Token::StartTagToken { name, .. } if ["caption", "col", "colgroup", "tbody", "tfoot", "thead"].contains(&name.as_str()) => {
                            if ! self.is_in_scope("tbody", Scope::Table) && ! self.is_in_scope("tfoot", Scope::Table) && ! self.is_in_scope("thead", Scope::Table) {
                                self.parse_error("caption, col, colgroup, tbody, tfoot or thead tag not allowed in in table body insertion mode");
                                // ignore token
                                continue;
                            }

                            self.clear_stack_back_to_table_body_context();
                            self.open_elements.pop();

                            self.insertion_mode = InsertionMode::InTable;
                            self.reprocess_token = true;
                        }
                        Token::EndTagToken { name, .. } if name == "table" => {
                            if ! self.is_in_scope("tbody", Scope::Table) && ! self.is_in_scope("tfoot", Scope::Table) && ! self.is_in_scope("thead", Scope::Table) {
                                self.parse_error("caption, col, colgroup, tbody, tfoot or thead tag not allowed in in table body insertion mode");
                                continue;
                            }

                            self.clear_stack_back_to_table_body_context();
                            self.open_elements.pop();

                            self.insertion_mode = InsertionMode::InTable;
                            self.reprocess_token = true;
                        }
                        Token::EndTagToken { name, .. }
                            if [
                                "body", "caption", "col", "colgroup", "html", "td", "th", "tr",
                            ]
                            .contains(&name.as_str()) =>
                        {
                            self.parse_error("end tag not allowed in in table body insertion mode");
                            // ignore token
                        }
                        _ => {
                            self.handle_in_table();
                        }
                    }
                }
                // Checked: 1
                InsertionMode::InRow => {
                    match &self.current_token {
                        Token::StartTagToken { name, .. } if name == "th" || name == "td" => {
                            self.clear_stack_back_to_table_row_context();

                            self.insert_html_element(&self.current_token.clone());

                            self.insertion_mode = InsertionMode::InCell;
                            self.active_formatting_elements_push_marker();
                        },
                        Token::EndTagToken { name, .. } if name == "tr" => {
                            if ! self.is_in_scope("tr", Scope::Table) {
                                self.parse_error("tr tag not allowed in in row insertion mode");
                                // ignore token
                                continue;
                            }

                            self.clear_stack_back_to_table_row_context();
                            pop_check!(self, "tr");

                            self.insertion_mode = InsertionMode::InTableBody;
                        }
                        Token::StartTagToken { name, .. } if ["caption", "col", "colgroup", "tbody", "tfoot", "thead", "tr"].contains(&name.as_str()) => {
                            if ! self.is_in_scope("tr", Scope::Table) {
                                self.parse_error("caption, col, colgroup, tbody, tfoot or thead tag not allowed in in row insertion mode");
                                // ignore token
                                continue;
                            }

                            self.clear_stack_back_to_table_row_context();
                            pop_check!(self, "tr");

                            self.insertion_mode = InsertionMode::InTableBody;
                            self.reprocess_token = true;
                        }
                        Token::EndTagToken { name, .. } if name == "table" => {
                         if ! self.is_in_scope("tr", Scope::Table) {
                                self.parse_error("table tag not allowed in in row insertion mode");
                                // ignore token
                                continue;
                            }

                            self.clear_stack_back_to_table_row_context();
                            pop_check!(self, "tr");

                            self.insertion_mode = InsertionMode::InTableBody;
                            self.reprocess_token = true;
                        }
                        Token::EndTagToken { name, .. } if name == "tbody" || name == "tfoot" || name == "thead" => {
                            if ! self.is_in_scope(name, Scope::Table) {
                                self.parse_error("tbody, tfoot or thead tag not allowed in in table body insertion mode");
                                // ignore token
                                continue;
                            }

                            if ! self.is_in_scope("tr", Scope::Table) {
                                // ignore token
                                continue;
                            }

                            self.clear_stack_back_to_table_row_context();
                            pop_check!(self, "tr");

                            self.insertion_mode = InsertionMode::InTableBody;
                            self.reprocess_token = true;
                        }
                        Token::EndTagToken { name, .. }
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
                            continue;
                        }
                        _ => self.handle_in_table(),
                    }
                }
                // Checked: 1
                InsertionMode::InCell => {
                    match &self.current_token {
                        Token::StartTagToken { name, .. } if name == "th" || name == "td" => {
                            let token_name = name.clone();

                            if ! self.is_in_scope(name.as_str(), Scope::Table) {
                                self.parse_error("th or td tag not allowed in in cell insertion mode");
                                // ignore token
                                continue;
                            }

                            self.generate_all_implied_end_tags(None, false);

                            if current_node!(self).name != token_name {
                                self.parse_error("current node should be th or td");
                            }

                            pop_until!(self, token_name);

                            self.active_formatting_elements_clear_until_marker();

                            self.insertion_mode = InsertionMode::InRow;
                        },
                        Token::StartTagToken { name, .. } if ["caption", "col", "colgroup", "tbody", "td", "tfoot", "th", "thead", "tr"].contains(&name.as_str()) => {
                            if ! self.is_in_scope("td", Scope::Table) && ! self.is_in_scope("th", Scope::Table) {
                                self.parse_error("caption, col, colgroup, tbody, tfoot or thead tag not allowed in in cell insertion mode");
                                // ignore token (fragment case?)
                                continue;
                            }

                            self.close_cell();
                            self.reprocess_token = true;
                        }
                        Token::EndTagToken { name, .. }
                            if name == "body"
                                || name == "caption"
                                || name == "col"
                                || name == "colgroup"
                                || name == "html" =>
                        {
                            self.parse_error("end tag not allowed in in cell insertion mode");
                            // ignore token
                        }
                        Token::EndTagToken { name, .. } if name == "table" || name == "tbody" || name == "tfoot" || name == "thead" || name == "tr" => {
                            if ! self.is_in_scope(name.as_str(), Scope::Table) {
                                self.parse_error("tbody, tfoot or thead tag not allowed in in table body insertion mode");
                                // ignore token
                                continue;
                            }

                            self.close_cell();
                            self.reprocess_token = true;
                        }
                        _ => self.handle_in_body(),
                    }
                }
                // Checked: 1
                InsertionMode::InSelect => {
                    match &self.current_token {
                        Token::TextToken { .. } if self.current_token.is_null() => {
                            self.parse_error(
                                "null character not allowed in in select insertion mode",
                            );
                            // ignore token
                        }
                        Token::TextToken { .. } => {
                            let node = self.create_node(&self.current_token, HTML_NAMESPACE);
                            self.document.add_node(node, current_node!(self).id);
                        }
                        Token::CommentToken { .. } => {
                            let node = self.create_node(&self.current_token, HTML_NAMESPACE);
                            self.document.add_node(node, current_node!(self).id);
                        }
                        Token::DocTypeToken { .. } => {
                            self.parse_error("doctype not allowed in in select insertion mode");
                            // ignore token
                        }
                        Token::StartTagToken { name, .. } if name == "html" => {
                            self.handle_in_body();
                        }
                        Token::StartTagToken { name, .. } if name == "option" => {
                            if current_node!(self).name == "option" {
                                self.open_elements.pop();
                            }

                            self.insert_html_element(&self.current_token.clone());
                        }
                        Token::StartTagToken { name, .. } if name == "optgroup" => {
                            if current_node!(self).name == "option" {
                                self.open_elements.pop();
                            }

                            if current_node!(self).name == "optgroup" {
                                self.open_elements.pop();
                            }

                            self.insert_html_element(&self.current_token.clone());
                        }
                        Token::StartTagToken {
                            name,
                            is_self_closing,
                            ..
                        } if name == "hr" => {
                            if current_node!(self).name == "option" {
                                self.open_elements.pop();
                            }

                            if current_node!(self).name == "optgroup" {
                                self.open_elements.pop();
                            }

                            acknowledge_closing_tag!(self, *is_self_closing);

                            self.insert_html_element(&self.current_token.clone());
                            self.open_elements.pop();
                        }
                        Token::EndTagToken { name, .. } if name == "optgroup" => {
                            if current_node!(self).name == "option"
                                && self.open_elements.len() > 1
                                && open_elements_get!(self, self.open_elements.len() - 1).name
                                    == "optgroup"
                            {
                                self.open_elements.pop();
                            }

                            if current_node!(self).name == "optgroup" {
                                self.open_elements.pop();
                            } else {
                                self.parse_error(
                                    "optgroup end tag not allowed in in select insertion mode",
                                );
                                // ignore token
                                continue;
                            }
                        }
                        Token::EndTagToken { name, .. } if name == "option" => {
                            if current_node!(self).name == "option" {
                                self.open_elements.pop();
                            } else {
                                self.parse_error(
                                    "option end tag not allowed in in select insertion mode",
                                );
                                // ignore token
                                continue;
                            }
                        }
                        Token::EndTagToken { name, .. } if name == "select" => {
                            if !self.is_in_scope("select", Scope::Select) {
                                self.parse_error("select end tag not allowed in in select insertion mode");
                                // ignore token
                                continue;
                            }

                            pop_until!(self, "select");
                            self.reset_insertion_mode();
                        }
                        Token::StartTagToken { name, .. } if name == "select" => {
                            self.parse_error("select tag not allowed in in select insertion mode");

                            if !self.is_in_scope("select", Scope::Select) {
                                // ignore token (fragment case?)
                                continue;
                            }

                            pop_until!(self, "select");
                            self.reset_insertion_mode();
                        }
                        Token::StartTagToken { name, .. }
                            if name == "input" || name == "keygen" || name == "textarea" =>
                        {
                            self.parse_error("input, keygen or textarea tag not allowed in in select insertion mode");

                            if !self.is_in_scope("select", Scope::Select) {
                                // ignore token (fragment case)
                                continue;
                            }

                            pop_until!(self, "select");
                            self.reset_insertion_mode();
                            self.reprocess_token = true;
                        }

                        Token::StartTagToken { name, .. }
                            if name == "script" || name == "template" =>
                        {
                            self.handle_in_head();
                        }
                        Token::EndTagToken { name, .. } if name == "template" => {
                            self.handle_in_head();
                        }
                        Token::EofToken => {
                            self.handle_in_body();
                        }
                        _ => {
                            self.parse_error(
                                "anything else not allowed in in select insertion mode",
                            );
                            // ignore token
                        }
                    }
                }
                // Checked: 1
                InsertionMode::InSelectInTable => {
                    match &self.current_token {
                        Token::StartTagToken { name, .. }
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

                            pop_until!(self, "select");
                            self.reset_insertion_mode();
                            self.reprocess_token = true;
                        }
                        Token::EndTagToken { name, .. }
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

                            if !self.is_in_scope(name, Scope::Select) {
                                // ignore token
                                continue;
                            }

                            pop_until!(self, "select");
                            self.reset_insertion_mode();
                            self.reprocess_token = true;
                        }
                        _ => self.handle_in_select(),
                    }
                }
                // Checked: 1
                InsertionMode::InTemplate => {
                    match &self.current_token {
                        Token::TextToken { .. } => {
                            self.handle_in_body();
                        }
                        Token::CommentToken { .. } => {
                            self.handle_in_body();
                        }
                        Token::DocTypeToken { .. } => {
                            self.handle_in_body();
                        }
                        Token::StartTagToken { name, .. }
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
                            self.handle_in_head();
                        }
                        Token::EndTagToken { name, .. } if name == "template" => {
                            self.handle_in_head();
                        }
                        Token::StartTagToken { name, .. }
                            if name == "caption"
                                || name == "colgroup"
                                || name == "tbody"
                                || name == "tfoot"
                                || name == "thead" =>
                        {
                            self.template_insertion_mode.pop();
                            self.template_insertion_mode.push(InsertionMode::InTable);

                            self.insertion_mode = InsertionMode::InTable;
                            self.reprocess_token = true;
                        }
                        Token::StartTagToken { name, .. } if name == "col" => {
                            self.template_insertion_mode.pop();
                            self.template_insertion_mode
                                .push(InsertionMode::InColumnGroup);

                            self.insertion_mode = InsertionMode::InColumnGroup;
                            self.reprocess_token = true;
                        }
                        Token::StartTagToken { name, .. } if name == "tr" => {
                            self.template_insertion_mode.pop();
                            self.template_insertion_mode
                                .push(InsertionMode::InTableBody);

                            self.insertion_mode = InsertionMode::InTableBody;
                            self.reprocess_token = true;
                        }
                        Token::StartTagToken { name, .. } if name == "td" || name == "th" => {
                            self.template_insertion_mode.pop();
                            self.template_insertion_mode.push(InsertionMode::InRow);

                            self.insertion_mode = InsertionMode::InRow;
                            self.reprocess_token = true;
                        }
                        Token::StartTagToken { .. } => {
                            self.template_insertion_mode.pop();
                            self.template_insertion_mode.push(InsertionMode::InBody);

                            self.insertion_mode = InsertionMode::InBody;
                            self.reprocess_token = true;
                        }
                        Token::EndTagToken { .. } => {
                            self.parse_error("end tag not allowed in in template insertion mode");
                            // ignore token
                            continue;
                        }
                        Token::EofToken => {
                            if !open_elements_has!(self, "template") {
                                self.stop_parsing();
                                continue;
                            }

                            self.parse_error("eof not allowed in in template insertion mode");

                            pop_until!(self, "template");
                            self.active_formatting_elements_clear_until_marker();
                            self.template_insertion_mode.pop();
                            self.reset_insertion_mode();
                            self.reprocess_token = true;
                        }
                    }
                }
                // Checked: 1
                InsertionMode::AfterBody => {
                    match &self.current_token {
                        Token::TextToken { .. } if self.current_token.is_empty_or_white() => {
                            self.handle_in_body();
                        }
                        Token::CommentToken { .. } => {
                            let node = self.create_node(&self.current_token, HTML_NAMESPACE);
                            let html_node_id = self.open_elements.first().unwrap_or(&0);
                            self.document.add_node(node, *html_node_id);
                        }
                        Token::DocTypeToken { .. } => {
                            self.parse_error("doctype not allowed in after body insertion mode");
                            // ignore token
                        }
                        Token::StartTagToken { name, .. } if name == "html" => {
                            self.handle_in_body();
                        }
                        Token::EndTagToken { name, .. } if name == "html" => {
                            // @TODO: something with fragment case
                            self.insertion_mode = InsertionMode::AfterAfterBody;
                        }
                        Token::EofToken => {
                            self.stop_parsing();
                            continue;
                        }
                        _ => {
                            self.parse_error(
                                "anything else not allowed in after body insertion mode",
                            );
                            self.insertion_mode = InsertionMode::InBody;
                            self.reprocess_token = true;
                        }
                    }
                }
                // Checked: 1
                InsertionMode::InFrameset => {
                    match &self.current_token {
                        Token::TextToken { .. } if self.current_token.is_empty_or_white() => {
                            let node = self.create_node(&self.current_token, HTML_NAMESPACE);
                            self.document.add_node(node, current_node!(self).id);
                        }
                        Token::CommentToken { .. } => {
                            let node = self.create_node(&self.current_token, HTML_NAMESPACE);
                            self.document.add_node(node, current_node!(self).id);
                        }
                        Token::DocTypeToken { .. } => {
                            self.parse_error("doctype not allowed in frameset insertion mode");
                            // ignore token
                        }
                        Token::StartTagToken { name, .. } if name == "html" => {
                            self.handle_in_body();
                        }
                        Token::StartTagToken { name, .. } if name == "frameset" => {
                            self.insert_html_element(&self.current_token.clone());
                        }
                        Token::EndTagToken { name, .. } if name == "frameset" => {
                            if current_node!(self).name == "html" {
                                self.parse_error(
                                    "frameset tag not allowed in frameset insertion mode",
                                );
                                // ignore token
                                continue;
                            }

                            self.open_elements.pop();

                            if !self.is_fragment_case && current_node!(self).name != "frameset" {
                                self.insertion_mode = InsertionMode::AfterFrameset;
                            }
                        }
                        Token::StartTagToken {
                            name,
                            is_self_closing,
                            ..
                        } if name == "frame" => {
                            acknowledge_closing_tag!(self, *is_self_closing);

                            self.insert_html_element(&self.current_token.clone());
                            self.open_elements.pop();
                        }
                        Token::StartTagToken { name, .. } if name == "noframes" => {
                            self.handle_in_head();
                        }
                        Token::EofToken => {
                            if current_node!(self).name != "html" {
                                self.parse_error("eof not allowed in frameset insertion mode");
                            }
                            self.stop_parsing();
                            continue;
                        }
                        _ => {
                            self.parse_error(
                                "anything else not allowed in frameset insertion mode",
                            );
                            // ignore token
                        }
                    }
                }
                // Checked: 1
                InsertionMode::AfterFrameset => {
                    match &self.current_token {
                        Token::TextToken { .. } if self.current_token.is_empty_or_white() => {
                            let node = self.create_node(&self.current_token, HTML_NAMESPACE);
                            self.document.add_node(node, current_node!(self).id);
                        }
                        Token::CommentToken { .. } => {
                            let node = self.create_node(&self.current_token, HTML_NAMESPACE);
                            self.document.add_node(node, current_node!(self).id);
                        }
                        Token::DocTypeToken { .. } => {
                            self.parse_error("doctype not allowed in frameset insertion mode");
                            // ignore token
                        }
                        Token::StartTagToken { name, .. } if name == "html" => {
                            self.handle_in_body();
                        }
                        Token::EndTagToken { name, .. } if name == "html" => {
                            self.insertion_mode = InsertionMode::AfterAfterFrameset;
                        }
                        Token::StartTagToken { name, .. } if name == "noframes" => {
                            self.handle_in_head();
                        }
                        Token::EofToken => {
                            self.stop_parsing();
                        }
                        _ => {
                            self.parse_error(
                                "anything else not allowed in after frameset insertion mode",
                            );
                            // ignore token
                        }
                    }
                }
                // Checked: 1
                InsertionMode::AfterAfterBody => match &self.current_token {
                    Token::CommentToken { .. } => {
                        let node = self.create_node(&self.current_token, HTML_NAMESPACE);
                        self.document.add_node(node, 0);
                    }
                    Token::DocTypeToken { .. } => {
                        self.handle_in_body();
                    }
                    Token::TextToken { .. } if self.current_token.is_empty_or_white() => {
                        self.handle_in_body();
                    }
                    Token::StartTagToken { name, .. } if name == "html" => {
                        self.handle_in_body();
                    }
                    Token::EofToken => {
                        self.stop_parsing();
                    }
                    _ => {
                        self.parse_error(
                            "anything else not allowed in after after body insertion mode",
                        );
                        self.insertion_mode = InsertionMode::InBody;
                        self.reprocess_token = true;
                    }
                },
                // Checked: 1
                InsertionMode::AfterAfterFrameset => {
                    match &self.current_token {
                        Token::CommentToken { .. } => {
                            let node = self.create_node(&self.current_token, HTML_NAMESPACE);
                            self.document.add_node(node, 0);
                        }
                        Token::DocTypeToken { .. } => {
                            self.handle_in_body();
                        }
                        Token::TextToken { .. } if self.current_token.is_empty_or_white() => {
                            self.handle_in_body();
                        }
                        Token::StartTagToken { name, .. } if name == "html" => {
                            self.handle_in_body();
                        }
                        Token::EofToken => {
                            self.stop_parsing();
                        }
                        Token::StartTagToken { name, .. } if name == "noframes" => {
                            self.handle_in_head();
                        }
                        _ => {
                            self.parse_error(
                                "anything else not allowed in after after frameset insertion mode",
                            );
                            // ignore token
                        }
                    }
                }
            }
        }

        (
            &self.document,
            self.error_logger.borrow().get_errors().clone(),
        )
    }

    // Retrieves a list of all errors generated by the parser/tokenizer
    pub fn get_parse_errors(&self) -> Vec<ParseError> {
        self.error_logger.borrow().get_errors().clone()
    }

    // Send a parse error to the error logger
    fn parse_error(&self, message: &str) {
        self.error_logger
            .borrow_mut()
            .add_error(self.tokenizer.get_position(), message);
    }

    // Create a new node that is not connected or attached to the document arena
    fn create_node(&self, token: &Token, namespace: &str) -> Node {
        let val: String;
        match token {
            Token::DocTypeToken {
                name,
                pub_identifier,
                sys_identifier,
                force_quirks,
            } => {
                val = format!(
                    "doctype[{} {} {} {}]",
                    name.as_deref().unwrap_or(""),
                    pub_identifier.as_deref().unwrap_or(""),
                    sys_identifier.as_deref().unwrap_or(""),
                    force_quirks
                );

                return Node::new_element(val.as_str(), HashMap::new(), namespace);
            }
            Token::StartTagToken {
                name, attributes, ..
            } => Node::new_element(name, attributes.clone(), namespace),
            Token::EndTagToken { name, .. } => Node::new_element(name, HashMap::new(), namespace),
            Token::CommentToken { value } => Node::new_comment(value),
            Token::TextToken { value } => Node::new_text(value.to_string().as_str()),
            Token::EofToken => {
                panic!("EOF token not allowed");
            }
        }
    }

    fn flush_pending_table_character_tokens(&self) {
        todo!()
    }

    // This function will pop elements off the stack until it reaches the first element that matches
    // our condition (which can be changed with the except and thoroughly parameters)
    fn generate_all_implied_end_tags(&mut self, except: Option<&str>, thoroughly: bool) {
        while !self.open_elements.is_empty() {
            let val = current_node!(self).name.clone();

            if except.is_some() && except.unwrap() == val {
                return;
            }

            if thoroughly && !["tbody", "td", "tfoot", "th", "thead", "tr"].contains(&val.as_str())
            {
                return;
            }

            if ![
                "dd", "dt", "li", "option", "optgroup", "p", "rb", "rp", "rt", "rtc",
            ]
            .contains(&val.as_str())
            {
                return;
            }

            self.open_elements.pop();
        }
    }

    // Reset insertion mode based on all kind of rules
    fn reset_insertion_mode(&mut self) {
        let mut last = false;
        let mut idx = self.open_elements.len() - 1;

        loop {
            let node = open_elements_get!(self, idx);
            if idx == 0 {
                last = true;
                // @TODO:
                // if fragment_case {
                //   node = context element !???
                // }
            }

            if node.name == "select" {
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
                    let ancestor = open_elements_get!(self, ancestor_idx);

                    if ancestor.name == "template" {
                        self.insertion_mode = InsertionMode::InSelect;
                        return;
                    }

                    if ancestor.name == "table" {
                        self.insertion_mode = InsertionMode::InSelectInTable;
                        return;
                    }
                }
            }

            if (node.name == "td" || node.name == "th") && !last {
                self.insertion_mode = InsertionMode::InCell;
                return;
            }
            if node.name == "tr" {
                self.insertion_mode = InsertionMode::InRow;
                return;
            }
            if ["tbody", "thead", "tfoot"]
                .iter()
                .any(|&elem| elem == node.name)
            {
                self.insertion_mode = InsertionMode::InTableBody;
                return;
            }
            if node.name == "caption" {
                self.insertion_mode = InsertionMode::InCaption;
                return;
            }
            if node.name == "colgroup" {
                self.insertion_mode = InsertionMode::InColumnGroup;
                return;
            }
            if node.name == "table" {
                self.insertion_mode = InsertionMode::InTable;
                return;
            }
            if node.name == "template" {
                self.insertion_mode = *self.template_insertion_mode.last().unwrap();
                return;
            }
            if node.name == "head" && !last {
                self.insertion_mode = InsertionMode::InHead;
                return;
            }
            if node.name == "body" {
                self.insertion_mode = InsertionMode::InBody;
                return;
            }
            if node.name == "frameset" {
                self.insertion_mode = InsertionMode::InFrameset;
                return;
            }
            if node.name == "html" {
                if self.head_element.is_none() {
                    self.insertion_mode = InsertionMode::BeforeHead;
                    return;
                }
                self.insertion_mode = InsertionMode::AfterHead;
                return;
            }
            if last {
                self.insertion_mode = InsertionMode::InBody;
                return;
            }

            idx -= 1;
        }
    }

    // Pop all elements back to a table context
    fn clear_stack_back_to_table_context(&mut self) {
        while !self.open_elements.is_empty() {
            if ["table", "template", "html"].contains(&current_node!(self).name.as_str()) {
                return;
            }
            self.open_elements.pop();
        }
    }

    // Pop all elements back to a table context
    fn clear_stack_back_to_table_body_context(&mut self) {
        while !self.open_elements.is_empty() {
            if ["tbody", "tfoot", "thead", "template", "html"].contains(&current_node!(self).name.as_str()) {
                return;
            }
            self.open_elements.pop();
        }
    }

    // Pop all elements back to a table row context
    fn clear_stack_back_to_table_row_context(&mut self) {
        while !self.open_elements.is_empty() {
            let val = current_node!(self).name.clone();
            if ["tr", "template", "html"].contains(&val.as_str()) {
                return;
            }
            self.open_elements.pop();
        }
    }

    // Checks if the given element is in given scope
    fn is_in_scope(&self, tag: &str, scope: Scope) -> bool {
        let mut idx = self.open_elements.len() - 1;
        loop {
            let node = open_elements_get!(self, idx);
            if node.name == tag {
                return true;
            }

            match scope {
                Scope::Regular => {
                    if [
                        "applet", "caption", "html", "table", "td", "th", "marquee", "object",
                        "template",
                    ]
                    .contains(&node.name.as_str())
                    {
                        return false;
                    }
                }
                Scope::ListItem => {
                    if [
                        "applet", "caption", "html", "table", "td", "th", "marquee", "object",
                        "template", "ol", "ul",
                    ]
                    .contains(&node.name.as_str())
                    {
                        return false;
                    }
                }
                Scope::Button => {
                    if [
                        "applet", "caption", "html", "table", "td", "th", "marquee", "object",
                        "template", "button",
                    ]
                    .contains(&node.name.as_str())
                    {
                        return false;
                    }
                }
                Scope::Table => {
                    if ["html", "table", "template"].contains(&node.name.as_str()) {
                        return false;
                    }
                }
                Scope::Select => {
                    // Note: NOT contains instead of contains
                    if !["optgroup", "option"].contains(&node.name.as_str()) {
                        return false;
                    }
                }
            }

            idx -= 1;
        }
    }

    // Closes a table cell and switches the insertion mode to InRow
    fn close_cell(&mut self) {
        self.generate_all_implied_end_tags(None, false);

        let tag = current_node!(self).name.clone();
        if tag != "td" && tag != "th" {
            self.parse_error("current node should be td or th");
        }

        pop_until_any!(self, ["td", "th"]);

        self.active_formatting_elements_clear_until_marker();
        self.insertion_mode = InsertionMode::InRow;
    }

    // Handle insertion mode "in_body"
    fn handle_in_body(&mut self) {
        let mut any_other_end_tag = false;

        match &self.current_token.clone() {
            Token::TextToken { .. } if self.current_token.is_null() => {
                self.parse_error("null character not allowed in in body insertion mode");
                // ignore token
            }
            Token::TextToken { .. } if self.current_token.is_empty_or_white() => {
                self.reconstruct_formatting();

                let node = self.create_node(&self.current_token, HTML_NAMESPACE);
                self.document.add_node(node, current_node!(self).id);
            }
            Token::TextToken { .. } => {
                self.reconstruct_formatting();

                let node = self.create_node(&self.current_token, HTML_NAMESPACE);
                self.document.add_node(node, current_node!(self).id);

                self.frameset_ok = false;
            }
            Token::CommentToken { .. } => {
                let node = self.create_node(&self.current_token, HTML_NAMESPACE);
                self.document.add_node(node, current_node!(self).id);
            }
            Token::DocTypeToken { .. } => {
                self.parse_error("doctype not allowed in in body insertion mode");
                // ignore token
            }
            Token::StartTagToken {
                name, attributes, ..
            } if name == "html" => {
                self.parse_error("html tag not allowed in in body insertion mode");

                if open_elements_has!(self, "template") {
                    // ignore token
                    return;
                }

                // Add attributes to html element
                if let NodeData::Element {
                    attributes: node_attributes,
                    ..
                } = &mut current_node_mut!(self).data
                {
                    for (key, value) in attributes {
                        if !node_attributes.contains_key(key) {
                            node_attributes.insert(key.clone(), value.clone());
                        }
                    }
                };
            }
            Token::StartTagToken { name, .. }
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
                self.handle_in_head();
            }
            Token::EndTagToken { name, .. } if name == "template" => {
                self.handle_in_head();
            }
            Token::StartTagToken { name, .. } if name == "body" => {
                self.parse_error("body tag not allowed in in body insertion mode");

                if self.open_elements.len() > 1 || open_elements_get!(self, 1).name != "body" {
                    // ignore token
                    return;
                }

                if open_elements_has!(self, "template") {
                    // ignore token
                    return;
                }

                self.frameset_ok = false;

                // Add attributes to body element
                // @TODO add body attributes
            },
            Token::StartTagToken { name, .. } if name == "frameset" => {
                self.parse_error("frameset tag not allowed in in body insertion mode");

                if self.open_elements.len() == 1 || open_elements_get!(self, 1).name != "body" {
                    // ignore token
                    return;
                }

                if !self.frameset_ok {
                    // ignore token
                    return;
                }

                self.open_elements.remove(1);

                while current_node!(self).name != "html" {
                    self.open_elements.pop();
                }

                self.insert_html_element(&self.current_token.clone());

                self.insertion_mode = InsertionMode::InFrameset;
            }
            Token::EofToken => {
                if !self.template_insertion_mode.is_empty() {
                    self.handle_in_template();
                } else {
                    // @TODO: do stuff
                    self.stop_parsing();
                }
            }
            Token::EndTagToken { name, .. } if name == "body" => {
                if !self.is_in_scope("body", Scope::Regular) {
                    self.parse_error("body end tag not in scope");
                    // ignore token
                    return;
                }

                // @TODO: Other stuff

                self.insertion_mode = InsertionMode::AfterBody;
            }
            Token::EndTagToken { name, .. } if name == "html" => {
                if !self.is_in_scope("body", Scope::Regular) {
                    self.parse_error("body end tag not in scope");
                    // ignore token
                    return;
                }

                // @TODO: Other stuff

                self.insertion_mode = InsertionMode::AfterBody;
                self.reprocess_token = true;
            },
            Token::StartTagToken { name, .. } if name == "address" || name == "article" || name == "aside" || name == "blockquote" || name == "center" || name == "details" || name == "dialog" || name == "dir" || name == "div" || name == "dl" || name == "fieldset" || name == "figcaption" || name == "figure" || name == "footer" || name == "header" || name == "hgroup" || name == "main" || name == "menu" || name == "nav" || name == "ol" || name == "p" || name == "section" || name == "summary" || name == "ul" => {
                if self.is_in_scope("p", Scope::Button) {
                    self.close_p_element();
                }

                self.insert_html_element(&self.current_token.clone());
            },
            Token::StartTagToken { name, .. } if name == "h1" || name == "h2" || name == "h3" || name == "h4" || name == "h5" || name == "h6" => {
                if self.is_in_scope("p", Scope::Button) {
                    self.close_p_element();
                }

                if ["h1", "h2", "h3", "h4", "h5", "h6"].contains(&current_node!(self).name.as_str()) {
                    self.parse_error("h1-h6 not allowed in in body insertion mode");
                    self.open_elements.pop();
                }

                self.insert_html_element(&self.current_token.clone());
            },
            Token::StartTagToken { name, .. } if name == "pre" || name == "listing" => {
                if self.is_in_scope("p", Scope::Button) {
                    self.close_p_element();
                }

                self.insert_html_element(&self.current_token.clone());

                // @TODO: Next token is LF, ignore and move on to the next one

                self.frameset_ok = false;
            }
            Token::StartTagToken { name, .. } if name == "form" => {
                {
                    if self.form_element.is_some() && !open_elements_has!(self, "template") {
                        self.parse_error("error with template, form shzzl");
                        // ignore token
                    }

                    if self.is_in_scope("p", Scope::Button) {
                        self.close_p_element();
                    }
                }

                let node_id = self.insert_html_element(&self.current_token.clone());
                if !open_elements_has!(self, "template") {
                    self.form_element = Some(node_id);
                }
            }
            Token::StartTagToken { name, .. } if name == "li" => {
            }
            Token::StartTagToken { name, .. } if name == "dd" || name == "dt" => {
            }
            Token::StartTagToken { name, .. } if name == "plaintext" => {
                if self.is_in_scope("p", Scope::Button) {
                    self.close_p_element();
                }

                self.insert_html_element(&self.current_token.clone());

                self.tokenizer.state = State::PlaintextState;
            }
            Token::StartTagToken { name, .. } if name == "button" => {
                if self.is_in_scope("button", Scope::Regular) {
                    self.parse_error("button tag not allowed in in body insertion mode");
                    self.generate_all_implied_end_tags(None, false);
                    pop_until!(self, "button");
                }

                self.reconstruct_formatting();
                self.insert_html_element(&self.current_token.clone());
                self.frameset_ok = false;
            }
            Token::EndTagToken { name, .. } if name == "address" || name == "article" || name == "aside" || name == "blockquote" || name == "button" || name == "center" || name == "details" || name == "dialog" || name == "dir" || name == "div" || name == "dl" || name == "fieldset" || name == "figcaption" || name == "figure" || name == "footer" || name == "header" || name == "hgroup" || name == "listing" || name == "main" || name == "menu" || name == "nav" || name == "ol" || name == "pre" || name == "section" || name == "summary" || name == "ul" => {
                if ! self.is_in_scope(name, Scope::Regular) {
                    self.parse_error("end tag not in scope");
                    // ignore token
                    return;
                }

                self.generate_all_implied_end_tags(None, false);

                let cn = current_node!(self);
                if cn.name != *name {
                    self.parse_error("end tag not at top of stack");
                }

                pop_until!(self, *name);
            }
            Token::EndTagToken { name, .. } if name == "form" => {
                if !open_elements_has!(self, "template") {
                    let node_id = self.form_element;
                    self.form_element = None;

                    if node_id.is_none() || !self.is_in_scope(name, Scope::Regular) {
                        self.parse_error("end tag not in scope");
                        // ignore token
                        return;
                    }
                    let node_id = node_id.unwrap();

                    self.generate_all_implied_end_tags(None, false);

                    let cn = current_node!(self);
                    if cn.name != *name {
                        self.parse_error("end tag not at top of stack");
                    }

                    if node_id != cn.id {
                        self.parse_error("end tag not at top of stack");
                    }
                } else {
                    if ! self.is_in_scope(name, Scope::Regular) {
                        self.parse_error("end tag not in scope");
                        // ignore token
                        return;
                    }

                    self.generate_all_implied_end_tags(None, false);

                    let cn = current_node!(self);
                    if cn.name != *name {
                        self.parse_error("end tag not at top of stack");
                    }

                    pop_until!(self, *name);
                }
            }
            Token::EndTagToken { name, .. } if name == "p" => {
                if ! self.is_in_scope(name, Scope::Button) {
                    self.parse_error("end tag not in scope");

                    self.insert_html_element(&self.current_token.clone());

                    return;
                }

                self.close_p_element();
            }
            Token::EndTagToken { name, .. } if name == "li" => {
                if ! self.is_in_scope(name, Scope::ListItem) {
                    self.parse_error("end tag not in scope");
                    // ignore token
                    return;
                }

                self.generate_all_implied_end_tags(Some("li"), false);

                if current_node!(self).name != *name {
                    self.parse_error("end tag not at top of stack");
                }

                pop_until!(self, *name);
            }
            Token::EndTagToken { name, .. } if name == "dd" || name == "dt" => {
                if ! self.is_in_scope(name, Scope::Regular) {
                    self.parse_error("end tag not in scope");
                    // ignore token
                    return;
                }

                self.generate_all_implied_end_tags(Some(name), false);

                if current_node!(self).name != *name {
                    self.parse_error("end tag not at top of stack");
                }

                pop_until!(self, *name);
            }
            Token::EndTagToken { name, .. } if name == "h1" || name == "h2" || name == "h3" || name == "h4" || name == "h5" || name == "h6" => {
                if ! self.is_in_scope("h1", Scope::Regular) ||
                    ! self.is_in_scope("h2", Scope::Regular) ||
                    ! self.is_in_scope("h3", Scope::Regular) ||
                    ! self.is_in_scope("h4", Scope::Regular) ||
                    ! self.is_in_scope("h5", Scope::Regular) ||
                    ! self.is_in_scope("h6", Scope::Regular) {
                    self.parse_error("end tag not in scope");
                    // ignore token
                    return;
                }

                self.generate_all_implied_end_tags(Some(name), false);

                if current_node!(self).name != *name {
                    self.parse_error("end tag not at top of stack");
                }

                pop_until_any!(self, ["h1", "h2", "h3", "h4", "h5", "h6"]);
            }
            Token::EndTagToken { name, .. } if name == "sarcasm" => {
                // Take a deep breath
                any_other_end_tag = true;
            }
            Token::StartTagToken { name, .. } if name == "a" => {
                if let Some(node_id) = self.active_formatting_elements_has_until_marker("a") {
                    self.parse_error("a tag in active formatting elements");
                    self.run_adoption_agency(&self.current_token.clone());

                    // Remove from lists if not done already by the adoption agency
                    open_elements_remove!(self, node_id);
                    self.active_formatting_elements_remove(node_id);
                }

                self.reconstruct_formatting();

                let node_id = self.insert_html_element(&self.current_token.clone());
                self.active_formatting_elements_push(node_id);
            }
            Token::StartTagToken { name, .. }
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

                let node_id = self.insert_html_element(&self.current_token.clone());
                self.active_formatting_elements_push(node_id);
            }
            Token::StartTagToken { name, .. } if name == "nobr" => {
                self.reconstruct_formatting();

                if self.is_in_scope("nobr", Scope::Regular) {
                    self.parse_error("nobr tag in scope");
                    self.run_adoption_agency(&self.current_token.clone());
                    self.reconstruct_formatting();
                }

                let node_id = self.insert_html_element(&self.current_token.clone());
                self.active_formatting_elements_push(node_id);
            }
            Token::EndTagToken { name, .. }
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
                self.run_adoption_agency(&self.current_token.clone());
            }
            Token::StartTagToken { name, .. }
                if name == "applet" || name == "marquee" || name == "object" =>
            {
                self.reconstruct_formatting();

                self.insert_html_element(&self.current_token.clone());

                self.active_formatting_elements_push_marker();
                self.frameset_ok = false;
            }
            Token::EndTagToken { name, .. } if name == "applet" || name == "marquee" || name == "object" => {
                if ! self.is_in_scope(name, Scope::Regular) {
                    self.parse_error("end tag not in scope");
                    // ignore token
                    return;
                }

                self.generate_all_implied_end_tags(None, false);

                if current_node!(self).name != *name {
                    self.parse_error("end tag not at top of stack");
                }

                pop_until!(self, *name);
                self.active_formatting_elements_clear_until_marker();
            }
            Token::StartTagToken { name, .. } if name == "table" => {
                if self.document.quirks_mode != QuirksMode::Quirks && self.is_in_scope("p", Scope::Button) {
                    self.close_p_element();
                }

                self.insert_html_element(&self.current_token.clone());

                self.frameset_ok = false;
                self.insertion_mode = InsertionMode::InTable;
            }
            Token::EndTagToken {
                name,
                is_self_closing,
                ..
            } if name == "br" => {
                self.parse_error("br end tag not allowed");
                self.reconstruct_formatting();

                // Remove attributes if any
                let mut br = self.current_token.clone();
                if let Token::StartTagToken { attributes, .. } = &mut br {
                    attributes.clear();
                }

                let node = self.create_node(&br, HTML_NAMESPACE);
                self.document.add_node(node, current_node!(self).id);

                self.open_elements.pop();
                acknowledge_closing_tag!(self, *is_self_closing);
                self.frameset_ok = false;
            }
            Token::StartTagToken {
                name,
                is_self_closing,
                ..
            } if name == "area"
                || name == "br"
                || name == "embed"
                || name == "img"
                || name == "keygen"
                || name == "wbr" =>
            {
                self.reconstruct_formatting();

                self.insert_html_element(&self.current_token.clone());
                self.open_elements.pop();

                acknowledge_closing_tag!(self, *is_self_closing);
                self.frameset_ok = false;
            }
            Token::StartTagToken {
                name,
                is_self_closing,
                attributes,
            } if name == "input" => {
                self.reconstruct_formatting();

                let node = self.create_node(&self.current_token, HTML_NAMESPACE);
                self.document.add_node(node, current_node!(self).id);
                self.open_elements.pop();

                acknowledge_closing_tag!(self, *is_self_closing);

                if !attributes.contains_key("type")
                    || attributes.get("type") != Some(&String::from("hidden"))
                {
                    self.frameset_ok = false;
                }
            }
            Token::StartTagToken {
                name,
                is_self_closing,
                ..
            } if name == "param" || name == "source" || name == "track" => {
                let node = self.create_node(&self.current_token, HTML_NAMESPACE);
                self.document.add_node(node, current_node!(self).id);
                self.open_elements.pop();

                acknowledge_closing_tag!(self, *is_self_closing);
            }
            Token::StartTagToken { name, is_self_closing, .. } if name == "hr" => {
                if self.is_in_scope("p", Scope::Button) {
                    self.close_p_element();
                }

                let node = self.create_node(&self.current_token, HTML_NAMESPACE);
                self.document.add_node(node, current_node!(self).id);
                self.open_elements.pop();

                acknowledge_closing_tag!(self, *is_self_closing);
                self.frameset_ok = false;
            }
            Token::StartTagToken {
                name,
                is_self_closing,
                attributes,
            } if name == "image" => {
                self.parse_error("image tag not allowed");
                self.current_token = Token::StartTagToken {
                    name: "img".to_string(),
                    attributes: attributes.clone(),
                    is_self_closing: *is_self_closing,
                };
                self.reprocess_token = true;
            }
            Token::StartTagToken { name, .. } if name == "textarea" => {
                let node = self.create_node(&self.current_token, HTML_NAMESPACE);
                self.document.add_node(node, current_node!(self).id);
                self.open_elements.pop();

                // @TODO: if next token == LF, ignore and move on to the next one

                self.tokenizer.state = State::RcDataState;
                self.frameset_ok = false;
                self.insertion_mode = InsertionMode::Text;
            }
            Token::StartTagToken { name, .. } if name == "xmp" => {
                if self.is_in_scope("p", Scope::Button) {
                    self.close_p_element();
                }

                self.reconstruct_formatting();

                self.frameset_ok = false;
                self.parse_raw_data();
            }
            Token::StartTagToken { name, .. } if name == "iframe" => {
                self.frameset_ok = false;
                self.parse_raw_data();
            }
            Token::StartTagToken { name, .. } if name == "noembed" => {
                self.parse_raw_data();
            }
            Token::StartTagToken { name, .. } if name == "noscript" && self.scripting_enabled => {
                self.parse_raw_data();
            }
            Token::StartTagToken { name, .. } if name == "select" => {
                self.reconstruct_formatting();

                let node = self.create_node(&self.current_token, HTML_NAMESPACE);
                self.document.add_node(node, current_node!(self).id);
                self.open_elements.pop();

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
            Token::StartTagToken { name, .. } if name == "optgroup" || name == "option" => {
                if current_node!(self).name == "option" {
                    self.open_elements.pop();
                }

                self.reconstruct_formatting();

                let node = self.create_node(&self.current_token, HTML_NAMESPACE);
                self.document.add_node(node, current_node!(self).id);
            }
            Token::StartTagToken { name, .. } if name == "rb" || name == "rtc" => {
                if self.is_in_scope("ruby", Scope::Regular) {
                    self.generate_all_implied_end_tags(None, false);
                }

                if current_node!(self).name != "ruby" {
                    self.parse_error("rb or rtc not in scope");
                }

                let node = self.create_node(&self.current_token, HTML_NAMESPACE);
                self.document.add_node(node, current_node!(self).id);
            }
            Token::StartTagToken { name, .. } if name == "rp" || name == "rt" => {
                if self.is_in_scope("ruby", Scope::Regular) {
                    self.generate_all_implied_end_tags(Some("rtc"), false);
                }

                if current_node!(self).name != "rtc" && current_node!(self).name != "ruby" {
                    self.parse_error("rp or rt not in scope");
                }

                let node = self.create_node(&self.current_token, HTML_NAMESPACE);
                self.document.add_node(node, current_node!(self).id);
            }
            Token::StartTagToken {
                name,
                is_self_closing,
                attributes,
            } if name == "math" => {
                self.reconstruct_formatting();

                let mut token = Token::StartTagToken {
                    name: name.clone(),
                    attributes: attributes.clone(),
                    is_self_closing: *is_self_closing,
                };
                self.adjust_mathml_attributes(&mut token);
                self.adjust_foreign_attributes(&mut token);

                self.insert_foreign_element(&token, MATHML_NAMESPACE.into());

                if *is_self_closing {
                    self.open_elements.pop();
                    acknowledge_closing_tag!(self, *is_self_closing);
                }
            }
            Token::StartTagToken {
                name,
                is_self_closing,
                attributes,
            } if name == "svg" => {
                self.reconstruct_formatting();

                let mut token = Token::StartTagToken {
                    name: name.clone(),
                    attributes: attributes.clone(),
                    is_self_closing: *is_self_closing,
                };

                self.adjust_svg_attributes(&mut token);
                self.adjust_foreign_attributes(&mut token);
                self.insert_foreign_element(&token, SVG_NAMESPACE.into());

                if *is_self_closing {
                    self.open_elements.pop();
                    acknowledge_closing_tag!(self, *is_self_closing);
                }
            }
            Token::StartTagToken { name, .. }
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
            Token::StartTagToken { .. } => {
                self.reconstruct_formatting();
                self.insert_html_element(&self.current_token.clone());
            }
            _ => any_other_end_tag = true,
        }

        if any_other_end_tag {
            // @TODO: do stuff
        }
    }

    // Handle insertion mode "in_head"
    fn handle_in_head(&mut self) {
        let mut anything_else = false;

        match &self.current_token {
            Token::TextToken { .. } if self.current_token.is_empty_or_white() => {
                let node = self.create_node(&self.current_token, HTML_NAMESPACE);
                self.document.add_node(node, current_node!(self).id);
            }
            Token::CommentToken { .. } => {
                let node = self.create_node(&self.current_token, HTML_NAMESPACE);
                self.document.add_node(node, current_node!(self).id);
            }
            Token::DocTypeToken { .. } => {
                self.parse_error("doctype not allowed in before head insertion mode");
                // ignore token
            }
            Token::StartTagToken {
                name,
                is_self_closing,
                ..
            } if name == "base" || name == "basefont" || name == "bgsound" || name == "link" => {
                acknowledge_closing_tag!(self, *is_self_closing);

                self.insert_html_element(&self.current_token.clone());
                self.open_elements.pop();
            }
            Token::StartTagToken {
                name,
                is_self_closing,
                ..
            } if name == "meta" => {
                acknowledge_closing_tag!(self, *is_self_closing);

                self.insert_html_element(&self.current_token.clone());
                self.open_elements.pop();

                // @TODO: if active speculative html parser is null then...
                // we probably want to change the encoding if the element has a charset attribute and the current encoding is "tentative"
            }
            Token::StartTagToken { name, .. } if name == "title" => {
                self.parse_rcdata();
            }
            Token::StartTagToken { name, .. } if name == "noscript" && self.scripting_enabled => {
                self.parse_raw_data();
            }
            Token::StartTagToken { name, .. } if name == "noframes" || name == "style" => {
                self.parse_raw_data();
            }
            Token::StartTagToken { name, .. } if name == "noscript" && !self.scripting_enabled => {
                self.insert_html_element(&self.current_token.clone());
                self.insertion_mode = InsertionMode::InHeadNoscript;
            }
            Token::StartTagToken { name, .. } if name == "script" => {
                // @TODO: lots of work
            }
            Token::EndTagToken { name, .. } if name == "head" => {
                pop_check!(self, "head");
                self.insertion_mode = InsertionMode::AfterHead;
            }
            Token::EndTagToken { name, .. } if name == "body" || name == "html" || name == "br" => {
                anything_else = true;
            }
            Token::StartTagToken { name, .. } if name == "template" => {
                self.insert_html_element(&self.current_token.clone());
                self.active_formatting_elements_push_marker();
                self.frameset_ok = false;
                self.insertion_mode = InsertionMode::InTemplate;
                self.template_insertion_mode.push(InsertionMode::InTemplate);
            }
            Token::EndTagToken { name, .. } if name == "template" => {
                if !open_elements_has!(self, "template") {
                    self.parse_error("could not find template tag in open element stack");
                    // ignore token
                    return;
                }

                self.generate_all_implied_end_tags(None, true);

                if current_node!(self).name != "template" {
                    self.parse_error("template end tag not at top of stack");
                }

                pop_until!(self, "template");
                self.active_formatting_elements_clear_until_marker();
                self.template_insertion_mode.pop();

                self.reset_insertion_mode();
            }
            Token::StartTagToken { name, .. } if name == "head" => {
                self.parse_error("head tag not allowed in in head insertion mode");
                // ignore token
                return;
            }
            Token::EndTagToken { .. } => {
                self.parse_error("end tag not allowed in in head insertion mode");
                // ignore token
                return;
            }
            _ => {
                anything_else = true;
            }
        }
        if anything_else {
            pop_check!(self, "head");
            self.insertion_mode = InsertionMode::AfterHead;
            self.reprocess_token = true;
        }
    }

    // Handle insertion mode "in_template"
    fn handle_in_template(&mut self) {
        todo!()
    }

    // Handle insertion mode "in_table"
    fn handle_in_table(&mut self) {
        let mut anything_else = false;

        match &self.current_token {
            Token::TextToken { .. }
                if ["table", "tbody", "template", "tfoot", "tr"]
                    .iter()
                    .any(|&node| node == current_node!(self).name) =>
            {
                self.pending_table_character_tokens = Vec::new();
                self.original_insertion_mode = self.insertion_mode;
                self.insertion_mode = InsertionMode::InTableText;
                self.reprocess_token = true;
            }
            Token::CommentToken { .. } => {
                let node = self.create_node(&self.current_token, HTML_NAMESPACE);
                self.document.add_node(node, current_node!(self).id);
            }
            Token::DocTypeToken { .. } => {
                self.parse_error("doctype not allowed in in table insertion mode");
                // ignore token
            }
            Token::StartTagToken { name, .. } if name == "caption" => {
                self.clear_stack_back_to_table_context();
                self.active_formatting_elements_push_marker();
                self.insert_html_element(&self.current_token.clone());
                self.insertion_mode = InsertionMode::InCaption;
            }
            Token::StartTagToken { name, .. } if name == "colgroup" => {
                self.clear_stack_back_to_table_context();
                self.insert_html_element(&self.current_token.clone());
                self.insertion_mode = InsertionMode::InColumnGroup;
            }
            Token::StartTagToken { name, .. } if name == "col" => {
                self.clear_stack_back_to_table_context();

                let token = Token::StartTagToken {
                    name: "colgroup".to_string(),
                    is_self_closing: false,
                    attributes: HashMap::new(),
                };
                self.insert_html_element(&token);

                self.insertion_mode = InsertionMode::InColumnGroup;
                self.reprocess_token = true;
            }
            Token::StartTagToken { name, .. }
                if name == "tbody" || name == "tfoot" || name == "thead" =>
            {
                self.clear_stack_back_to_table_context();

                self.insert_html_element(&self.current_token.clone());

                self.insertion_mode = InsertionMode::InTableBody;
            }
            Token::StartTagToken { name, .. } if name == "td" || name == "th" || name == "tr" => {
                self.clear_stack_back_to_table_context();

                let token = Token::StartTagToken {
                    name: "tbody".to_string(),
                    is_self_closing: false,
                    attributes: HashMap::new(),
                };
                self.insert_html_element(&token);

                self.insertion_mode = InsertionMode::InTableBody;
                self.reprocess_token = true;
            }
            Token::StartTagToken { name, .. } if name == "table" => {
                self.parse_error("table tag not allowed in in table insertion mode");

                if !open_elements_has!(self, "table") {
                    // ignore token
                    return;
                }

                pop_until!(self, "table");
                self.reset_insertion_mode();
                self.reprocess_token = true;
            }
            Token::EndTagToken { name, .. } if name == "table" => {
                if !open_elements_has!(self, "table") {
                    self.parse_error("table end tag not allowed in in table insertion mode");
                    // ignore token
                    return;
                }

                pop_until!(self, "table");
                self.reset_insertion_mode();
            }
            Token::EndTagToken { name, .. }
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
            Token::StartTagToken { name, .. }
                if name == "style" || name == "script" || name == "template" =>
            {
                self.handle_in_head();
            }
            Token::EndTagToken { name, .. } if name == "template" => {
                self.handle_in_head();
            }
            Token::StartTagToken {
                name,
                is_self_closing,
                attributes,
            } if name == "input" => {
                if !attributes.contains_key("type")
                    || attributes.get("type") == Some(&String::from("hidden"))
                {
                    anything_else = true;
                } else {
                    self.parse_error("input tag not allowed in in table insertion mode");

                    acknowledge_closing_tag!(self, *is_self_closing);

                    self.insert_html_element(&self.current_token.clone());
                    pop_check!(self, "input");
                }
            }
            Token::StartTagToken { name, .. } if name == "form" => {
                self.parse_error("form tag not allowed in in table insertion mode");

                if open_elements_has!(self, "template") || self.form_element.is_some() {
                    // ignore token
                    return;
                }

                let node_id = self.insert_html_element(&self.current_token.clone());
                self.form_element = Some(node_id);

                pop_check!(self, "form");
            }
            Token::EofToken => {
                self.handle_in_body();
            }
            _ => anything_else = true,
        }

        if anything_else {
            self.parse_error("anything else not allowed in in table insertion mode");

            self.foster_parenting = true;
            self.handle_in_body();
            self.foster_parenting = false;
        }
    }

    // Handle insertion mode "in_select"
    fn handle_in_select(&mut self) {
        todo!()
    }

    // Returns true if the given tag if found in the active formatting elements list (until the first marker)
    fn active_formatting_elements_has_until_marker(&self, tag: &str) -> Option<usize> {
        if self.active_formatting_elements.is_empty() {
            return None;
        }

        let mut idx = self.active_formatting_elements.len() - 1;
        loop {
            match self.active_formatting_elements[idx] {
                ActiveElement::Marker => return None,
                ActiveElement::NodeId(node_id) => {
                    if self.document.get_node_by_id(node_id).expect("node_id").name == tag {
                        return Some(node_id);
                    }
                }
            }

            if idx == 0 {
                // Reached the beginning of the list
                return None;
            }

            idx -= 1;
        }
    }

    // Adds a marker to the active formatting stack
    fn active_formatting_elements_push_marker(&mut self) {
        self.active_formatting_elements.push(ActiveElement::Marker);
    }

    // Clear the active formatting stack until we reach the first marker
    fn active_formatting_elements_clear_until_marker(&mut self) {
        while let Some(active_elem) = self.active_formatting_elements.pop() {
            if let ActiveElement::Marker = active_elem {
                // Found the marker
                return;
            }
        }
    }

    // Remove the given node_id from the active formatting elements list
    fn active_formatting_elements_remove(&mut self, target_node_id: usize) {
        self.active_formatting_elements.retain(|node_id| {
            match node_id {
                ActiveElement::NodeId(node_id) => {
                    *node_id != target_node_id
                },
                _ => true,
            }
        });
    }

    // Push a node onto the active formatting stack, make sure only max 3 of them can be added (between markers)
    fn active_formatting_elements_push(&mut self, node_id: usize) {
        let mut idx = self.active_formatting_elements.len();
        if idx == 0 {
            self.active_formatting_elements.push(ActiveElement::NodeId(node_id));
            return;
        }

        // Fetch the node we want to push, so we can compare
        let element_node = self.document.get_node_by_id(node_id).expect("node id not found");

        let mut found = 0;
        loop {
            let active_elem = *self.active_formatting_elements.get(idx - 1).expect("index out of bounds");
            if let ActiveElement::Marker = active_elem {
                // Don't continue after the last marker
                break;
            }

            // Fetch the node we want to compare with
            let match_node = match active_elem {
                ActiveElement::NodeId(node_id) => self.document.get_node_by_id(node_id).expect("node id not found"),
                ActiveElement::Marker => unreachable!(),
            };
            if match_node.matches_tag_and_attrs(element_node) {
                // Noah's Ark clause: we only allow 3 (instead of 2) of each tag (between markers)
                found += 1;
                if found == 3 {
                    // Remove the element from the list
                    self.active_formatting_elements.remove(idx - 1);
                    break;
                }
            }

            if idx == 0 {
                break;
            }
            idx -= 1;
        }

        self.active_formatting_elements.push(ActiveElement::NodeId(node_id));
    }

    fn reconstruct_formatting(&mut self) {
        if self.active_formatting_elements.is_empty() {
            return; // Nothing to reconstruct.
        }

        let mut entry_index: usize = self.active_formatting_elements.len() - 1;
        let entry = self.active_formatting_elements[entry_index];

        // If it's a marker or in the stack of open elements, nothing to reconstruct.
        if let ActiveElement::Marker = entry {
            return;
        }

        if self.open_elements.contains(&entry.node_id().expect("node id not found")) {
            return;
        }

        loop {
            let entry = self.active_formatting_elements[entry_index];

            // If it's a marker or in the stack of open elements, nothing to reconstruct.
            if let ActiveElement::Marker = entry {
                break;
            }

            if self.open_elements.contains(&entry.node_id().expect("node id not found")) {
                break;
            }

            if entry_index == 0 {
                break;
            }

            entry_index -= 1;
        }

        loop {
            let entry = self.active_formatting_elements[entry_index];
            let node_id = entry.node_id().expect("node id not found");

            let entry_node = self.document.get_node_by_id(node_id).expect("node not found").clone();
            let new_node_id = self.clone_node(entry_node);

            self.active_formatting_elements[entry_index] = ActiveElement::NodeId(new_node_id);

            if entry_index == self.active_formatting_elements.len() - 1 {
                break;
            }

            entry_index += 1;
        }
    }

    fn clone_node(&mut self, org_node: Node) -> usize {
        let new_node = org_node.clone();

        let new_node_id = self.document.add_node(new_node, current_node!(self).id);
        if let NodeData::Element { .. } = org_node.data {
            self.open_elements.push(new_node_id);
        }

        new_node_id
    }

    // fn create_new_formatting_element(&mut self, idx: usize, node: &Node) {
    //     match &node.data {
    //         NodeData::Element { name, attributes } => {
    //             let token = Token::StartTagToken { name: name.clone(), is_self_closing: false, attributes: attributes.clone() };
    //             let node_id = self.insert_html_element(&token);
    //
    //             self.active_formatting_elements[idx] = ActiveElement::NodeId(node_id);
    //         }
    //         _ => {}
    //     }
    // }

    fn stop_parsing(&self) {
        todo!()
    }

    // Close the p element that may or may not be on the open elements stack
    fn close_p_element(&mut self) {
        self.generate_all_implied_end_tags(Some("p"), false);

        if current_node!(self).name != "p" {
            self.parse_error("p element not at top of stack");
        }

        pop_until!(self, "p");
    }

    // Adjusts attributes names in the given token for SVG
    fn adjust_svg_attributes(&self, token: &mut Token) {
        if let Token::StartTagToken { attributes, .. } = token {
            let mut new_attributes = HashMap::new();
            for (name, value) in attributes.iter() {
                if SVG_ADJUSTMENTS.contains_key(name) {
                    let new_name = SVG_ADJUSTMENTS.get(name).unwrap();
                    new_attributes.insert(new_name.to_string(), value.clone());
                } else {
                    new_attributes.insert(name.clone(), value.clone());
                }
            }
            *attributes = new_attributes;
        }
    }

    // Adjust attribute names in the given token for MathML
    fn adjust_mathml_attributes(&self, token: &mut Token) {
        if let Token::StartTagToken { attributes, .. } = token {
            let mut new_attributes = HashMap::new();
            for (name, value) in attributes.iter() {
                if MATHML_ADJUSTMENTS.contains_key(name) {
                    let new_name = SVG_ADJUSTMENTS.get(name).unwrap();
                    new_attributes.insert(new_name.to_string(), value.clone());
                } else {
                    new_attributes.insert(name.clone(), value.clone());
                }
            }
            *attributes = new_attributes;
        }
    }

    fn adjust_foreign_attributes(&self, token: &mut Token) {
        if let Token::StartTagToken { attributes, .. } = token {
            let mut new_attributes = HashMap::new();
            for (name, value) in attributes.iter() {
                if XML_ADJUSTMENTS.contains_key(name) {
                    let new_name = SVG_ADJUSTMENTS.get(name).unwrap();
                    new_attributes.insert(new_name.to_string(), value.clone());
                } else {
                    new_attributes.insert(name.clone(), value.clone());
                }
            }
            *attributes = new_attributes;
        }
    }

    fn insert_html_element(&mut self, token: &Token) -> usize {
        self.insert_foreign_element(token, Some(HTML_NAMESPACE))
    }

    fn insert_foreign_element(&mut self, token: &Token, namespace: Option<&str>) -> usize {
        // adjusted insert location
        let adjusted_insert_location = self.adjusted_insert_location(None);
        //        let parent_id = current_node!(self).id;

        let node = self.create_node(token, namespace.unwrap_or(HTML_NAMESPACE));

        // if parent_id is possible to insert element  (for instance: document already has child element etc)
        //    if parser not created  as part of html fragmentparsing algorithm
        //      push new element queue onto relevant agent custom element reactions stack (???)

        //   insert element into adjusted_insert_location
        let node_id = self.document.add_node(node, adjusted_insert_location);

        //     if parser not created as part of html fragment parsing algorithm
        //       pop the top element queue from the relevant agent custom element reactions stack (???)

        // push element onto the stack of open elements so that is the new current node
        self.open_elements.push(node_id);

        // return element
        node_id
    }

    // Switch the parser and tokenizer to the RAWTEXT state
    fn parse_raw_data(&mut self) {
        self.insert_html_element(&self.current_token.clone());

        self.tokenizer.state = State::RawTextState;

        self.original_insertion_mode = self.insertion_mode;
        self.insertion_mode = InsertionMode::Text;
    }

    // Switch the parser and tokenizer to the RCDATA state
    fn parse_rcdata(&mut self) {
        self.insert_html_element(&self.current_token.clone());

        self.tokenizer.state = State::RcDataState;

        self.original_insertion_mode = self.insertion_mode;
        self.insertion_mode = InsertionMode::Text;
    }

    fn adjusted_insert_location(&self, override_node: Option<&Node>) -> usize {
        let target = match override_node {
            Some(node) => node,
            None => current_node!(self),
        };

        let mut adjusted_insertion_location = target.id;

        if self.foster_parenting
            && ["table", "tbody", "thead", "tfoot", "tr"].contains(&target.name.as_str())
        {
            /*
            @todo!()

            Run these substeps:

                Let last template be the last template element in the stack of open elements, if any.

                Let last table be the last table element in the stack of open elements, if any.

                If there is a last template and either there is no last table, or there is one, but last template is lower (more recently added) than last table in the stack of open elements, then: let adjusted insertion location be inside last template's template contents, after its last child (if any), and abort these steps.

                If there is no last table, then let adjusted insertion location be inside the first element in the stack of open elements (the html element), after its last child (if any), and abort these steps. (fragment case)

                If last table has a parent node, then let adjusted insertion location be inside last table's parent node, immediately before last table, and abort these steps.

                Let previous element be the element immediately above last table in the stack of open elements.

                Let adjusted insertion location be inside previous element, after its last child (if any).
             */

            adjusted_insertion_location = target.id
        }

        if target.name == "template" {
            // @todo!()
            // be the content
        }

        adjusted_insertion_location
    }

    #[cfg(debug_assertions)]
    fn display_debug_info(&self) {
        println!("** Frame **************************");
        println!("current token  : {}", self.current_token);
        println!("insertion mode : {:?}", self.insertion_mode);

        println!("-- open elements  -----------");
        for node_id in &self.open_elements {
            let node = self.document.get_node_by_id(*node_id).unwrap();
            println!("({}) {}", node.id, node.name);
        }
        println!("-----------------------------");

        println!("-- active formatting elems --");
        for elem in &self.active_formatting_elements {
            match elem {
                ActiveElement::NodeId(node_id) => {
                    let node = self.document.get_node_by_id(*node_id).unwrap();
                    println!("({}) {}", node.id, node.name);
                }
                ActiveElement::Marker => {
                    println!("marker");
                }
            }
        }
        println!("-----------------------------");
        println!("-- TREE ---------------------");
        println!("{}", self.document);
        println!("-----------------------------");
    }
}