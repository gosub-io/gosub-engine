mod attr_replacements;
pub mod document;
mod quirks;
pub mod tree_builder;

// ------------------------------------------------------------

use self::document::DocumentHandle;
use super::node::NodeId;
use crate::html5::error_logger::{ErrorLogger, ParserError};
use crate::html5::input_stream::InputStream;
use crate::html5::node::{Node, NodeData, HTML_NAMESPACE, MATHML_NAMESPACE, SVG_NAMESPACE};
use crate::html5::parser::attr_replacements::{
    MATHML_ADJUSTMENTS, SVG_ADJUSTMENTS_ATTRIBUTES, SVG_ADJUSTMENTS_TAGS, XML_ADJUSTMENTS,
};
use crate::html5::parser::document::{Document, DocumentBuilder, DocumentFragment, DocumentType};
use crate::html5::parser::quirks::QuirksMode;
use crate::html5::tokenizer::state::State;
use crate::html5::tokenizer::token::Token;
use crate::html5::tokenizer::{Tokenizer, CHAR_NUL, CHAR_REPLACEMENT};
use crate::types::{ParseError, Result};
use alloc::rc::Rc;
use core::cell::RefCell;
use core::option::Option::Some;
use std::collections::HashMap;
#[cfg(feature = "debug_parser")]
use std::io::Write;

/// Insertion modes as defined in 13.2.4.1
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

/// Additional extensions to the Vec type so we can do some stack operations
trait VecExtensions<T> {
    fn pop_until<F>(&mut self, f: F)
    where
        F: FnMut(&T) -> bool;
    fn pop_check<F>(&mut self, f: F) -> bool
    where
        F: FnMut(&T) -> bool;
}

impl VecExtensions<NodeId> for Vec<NodeId> {
    fn pop_until<F>(&mut self, mut f: F)
    where
        F: FnMut(&NodeId) -> bool,
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
        F: FnMut(&NodeId) -> bool,
    {
        match self.pop() {
            Some(popped_value) => f(&popped_value),
            None => false,
        }
    }
}

macro_rules! get_node_by_id {
    ($doc_handle:expr, $id:expr) => {
        $doc_handle
            .get()
            .get_node_by_id($id)
            .expect("Node not found")
            .clone()
    };
}

macro_rules! current_node {
    ($self:expr) => {{
        let current_node_idx = $self.open_elements.last().unwrap_or_default();
        $self
            .document
            .get()
            .get_node_by_id(*current_node_idx)
            .expect("Current node not found")
            .clone()
    }};
}

macro_rules! open_elements_get {
    ($self:expr, $idx:expr) => {{
        $self
            .document
            .get()
            .get_node_by_id($self.open_elements[$idx])
            .expect("Current node not found")
            .clone()
    }};
}

#[macro_use]
mod helper;

/// Active formatting elements, which could be a regular node(id), or a marker
#[derive(PartialEq, Clone, Copy)]
enum ActiveElement {
    Node(NodeId),
    Marker,
}

impl ActiveElement {
    fn node_id(&self) -> Option<NodeId> {
        match self {
            ActiveElement::Node(id) => Some(*id),
            _ => None,
        }
    }
}

pub struct Html5ParserOptions {
    pub scripting_enabled: bool,
}

impl Default for Html5ParserOptions {
    fn default() -> Self {
        Html5ParserOptions {
            scripting_enabled: true,
        }
    }
}

/// The main parser object
pub struct Html5Parser<'stream> {
    /// tokenizer object
    tokenizer: Tokenizer<'stream>,
    /// current insertion mode
    insertion_mode: InsertionMode,
    /// original insertion mode (used for text mode)
    original_insertion_mode: InsertionMode,
    /// template insertion mode stack
    template_insertion_mode: Vec<InsertionMode>,
    /// ??
    parser_cannot_change_mode: bool,
    /// Current token from the tokenizer
    current_token: Token,
    /// If true, the current token should be processed again
    reprocess_token: bool,
    /// Stack of open elements
    open_elements: Vec<NodeId>,
    /// Current head element
    head_element: Option<NodeId>,
    /// Current form element
    form_element: Option<NodeId>,
    /// If true, scripting is enabled
    scripting_enabled: bool,
    /// if true, we can insert a frameset
    frameset_ok: bool,
    /// Foster parenting flag
    foster_parenting: bool,
    /// If true, the script engine has already started
    script_already_started: bool,
    /// Pending table character tokens
    pending_table_character_tokens: String,
    /// Acknowledge self closing tags
    ack_self_closing: bool,
    /// List of active formatting elements or markers
    active_formatting_elements: Vec<ActiveElement>,
    /// Is the current parsing a fragment case. If so, the context_node_id and context_doc should be set as well.
    is_fragment_case: bool,
    /// A reference to the document we are parsing
    document: DocumentHandle,
    /// Error logger, which is shared with the tokenizer
    error_logger: Rc<RefCell<ErrorLogger>>,
    /// Levels of scripting we currently are in
    script_nesting_level: u32,
    /// If true, the parser is paused
    parser_pause_flag: bool,
    /// Keeps the position of where any document.write() should be inserted when running a script
    insertion_point: Option<usize>,
    /// Ignore when next token is LF
    ignore_lf: bool,
    /// Sometimes tokens needs to be split up (and it seems the tokenizer cannot do this?)
    token_queue: Vec<Token>,
    /// When true, the parser is finished and should not consume more tokens (there aren't any)
    parser_finished: bool,
    /// Context node id for fragment parsing
    context_node_id: Option<NodeId>,
    /// Context node document for fragment parsing (we don't want to keep Option<Node> as this clones a whole node
    context_doc: Option<DocumentHandle>,
}

/// Defines the scopes for in_scope()
enum Scope {
    Regular,
    ListItem,
    Button,
    Table,
    Select,
}

/// Defines the mode we should dispatch
enum DispatcherMode {
    Foreign,
    Html,
}

impl<'stream> Html5Parser<'stream> {
    // Initializes the parser for whole document parsing
    fn init(
        tokenizer: Tokenizer<'stream>,
        document: DocumentHandle,
        error_logger: Rc<RefCell<ErrorLogger>>,
        options: Option<Html5ParserOptions>,
    ) -> Self {
        Html5Parser {
            tokenizer,
            insertion_mode: InsertionMode::Initial,
            original_insertion_mode: InsertionMode::Initial,
            template_insertion_mode: vec![],
            parser_cannot_change_mode: false,
            current_token: Token::Eof,
            reprocess_token: false,
            open_elements: Vec::new(),
            head_element: None,
            form_element: None,
            scripting_enabled: options.unwrap_or_default().scripting_enabled,
            frameset_ok: true,
            foster_parenting: false,
            script_already_started: false,
            pending_table_character_tokens: String::new(),
            ack_self_closing: false,
            active_formatting_elements: vec![],
            is_fragment_case: false,
            document,
            error_logger,
            script_nesting_level: 0,
            parser_pause_flag: false,
            insertion_point: None,
            ignore_lf: false,
            token_queue: vec![],
            parser_finished: false,
            context_node_id: None,
            context_doc: None,
        }
    }

    /// Creates a new parser with a dummy document and dummy tokenizer. This is ONLY used for testing purposes.
    /// Regular users should use the parse_document() and parse_fragment() functions instead.
    pub fn new_parser(stream: &'stream mut InputStream) -> Self {
        let doc = DocumentBuilder::new_document();
        let error_logger = Rc::new(RefCell::new(ErrorLogger::new()));
        let tokenizer = Tokenizer::new(stream, None, error_logger.clone());

        Html5Parser {
            tokenizer,
            insertion_mode: InsertionMode::Initial,
            original_insertion_mode: InsertionMode::Initial,
            template_insertion_mode: vec![],
            parser_cannot_change_mode: false,
            current_token: Token::Eof,
            reprocess_token: false,
            open_elements: Vec::new(),
            head_element: None,
            form_element: None,
            scripting_enabled: true,
            frameset_ok: true,
            foster_parenting: false,
            script_already_started: false,
            pending_table_character_tokens: String::new(),
            ack_self_closing: false,
            active_formatting_elements: vec![],
            is_fragment_case: false,
            document: Document::clone(&doc),
            error_logger,
            script_nesting_level: 0,
            parser_pause_flag: false,
            insertion_point: None,
            ignore_lf: false,
            token_queue: vec![],
            parser_finished: false,
            context_node_id: None,
            context_doc: None,
        }
    }

    /// Parses a fragment of HTML instead of a whole document. It will run the parser in a slightly different mode.
    /// This is used for parsing innerHTML and document fragments.
    pub fn parse_fragment(
        stream: &'stream mut InputStream,
        mut document: DocumentHandle,
        context_node: &Node,
        options: Option<Html5ParserOptions>,
    ) -> Result<Vec<ParseError>> {
        // https://html.spec.whatwg.org/multipage/parsing.html#parsing-html-fragments

        // 1.
        document.get_mut().doctype = DocumentType::HTML;

        // 2.
        document.get_mut().quirks_mode = context_node.document.get().quirks_mode;

        // 3.
        let error_logger = Rc::new(RefCell::new(ErrorLogger::new()));

        let tokenizer = Tokenizer::new(stream, None, error_logger.clone());
        let mut parser =
            Html5Parser::init(tokenizer, Document::clone(&document), error_logger, options);

        // 4. / 12.
        parser.initialize_fragment_case(context_node);

        // 5. / 6.
        // Not needed, as the document should have been created with DocumentBuilder::document_fragment(), and already got a HTML root node.

        // 7.
        parser.open_elements.push(NodeId::root());

        // 8.
        if context_node.name == "template" {
            parser
                .template_insertion_mode
                .push(InsertionMode::InTemplate);
        }

        // 9.
        // @Todo: this does not do anything yet
        let _token = match &context_node.data {
            NodeData::Element(element) => {
                let node_attributes = element.attributes.clone();

                Token::StartTag {
                    name: context_node.name.clone(),
                    is_self_closing: false,
                    attributes: node_attributes,
                }
            }
            _ => panic!("not an element"),
        };

        // 10.
        parser.reset_insertion_mode();

        // 11. Set the parser's form element pointer to the nearest node to the context element that is a form element (going straight up the ancestor chain, and including the element itself, if it is a form element), if any. (If there is no such form element, the form element pointer keeps its initial value, null.)
        let mut node = context_node.clone();
        loop {
            if node.name == "form" {
                parser.form_element = Some(node.id);
                break;
            }

            if node.parent.is_none() {
                break;
            }

            node = get_node_by_id!(document, node.parent.unwrap());
        }

        // 13. / 14.
        parser.do_parse()
    }

    /// Parses the input stream into a full document (including html, body, head, etc.). Note that
    /// the document returned is not a full document, but a document fragment and has a "html" root
    /// node that should not be used. The children of the root-node should be used on the context
    /// node where this document fragment needs to be inserted into.
    pub fn parse_document(
        stream: &'stream mut InputStream,
        document: DocumentHandle,
        options: Option<Html5ParserOptions>,
    ) -> Result<Vec<ParseError>> {
        // Create a new error logger that will be used in both the tokenizer and the parser
        let error_logger = Rc::new(RefCell::new(ErrorLogger::new()));

        let tokenizer = Tokenizer::new(stream, None, error_logger.clone());
        let mut parser = Html5Parser::init(tokenizer, document, error_logger, options);

        parser.do_parse()
    }

    /// Internal parser function that does the actual parsing
    pub fn do_parse(&mut self) -> Result<Vec<ParseError>> {
        let mut dispatcher_mode = DispatcherMode::Html;

        loop {
            // When the parser is signalled to finish, we break our main parser loop
            if self.parser_finished {
                break;
            }

            // If reprocess_token is true, we should process the same token again
            if !self.reprocess_token {
                self.current_token = self.fetch_next_token();

                // If we reprocess a given token, the dispatcher mode should stay the same and
                // should not be re-evaluated
                dispatcher_mode = self.select_dispatch_mode();
            }

            self.reprocess_token = false;

            // Check how we should dispatch the token, and dispatch to the correct function
            match dispatcher_mode {
                DispatcherMode::Foreign => {
                    self.process_foreign_content();
                }
                DispatcherMode::Html => {
                    self.process_html_content();
                }
            }

            #[cfg(feature = "debug_parser")]
            self.display_debug_info();
        }

        let result = Ok(self.error_logger.borrow().get_errors().clone());
        result
    }

    // Process token in foreign content (svg, mathml)
    fn process_foreign_content(&mut self) {
        let mut handle_as_script_endtag = false;

        match &self.current_token.clone() {
            Token::Text(..) if self.current_token.is_null() => {
                self.parse_error("null character not allowed in foreign content");
                self.insert_text_element(&Token::Text(CHAR_REPLACEMENT.to_string()));
            }
            Token::Text(..) if self.current_token.is_empty_or_white() => {
                self.insert_text_element(&self.current_token.clone());
            }
            Token::Text(..) => {
                self.insert_text_element(&self.current_token.clone());

                self.frameset_ok = false;
            }
            Token::Comment(..) => {
                self.insert_comment_element(&self.current_token.clone(), None);
            }
            Token::DocType { .. } => {
                self.parse_error("doctype not allowed in foreign content");
                // ignore token
            }
            Token::StartTag { name, .. }
                if name == "b"
                    || name == "big"
                    || name == "blockquote"
                    || name == "body"
                    || name == "br"
                    || name == "center"
                    || name == "code"
                    || name == "dd"
                    || name == "div"
                    || name == "dl"
                    || name == "dt"
                    || name == "em"
                    || name == "embed"
                    || name == "h1"
                    || name == "h2"
                    || name == "h3"
                    || name == "h4"
                    || name == "h5"
                    || name == "h6"
                    || name == "head"
                    || name == "hr"
                    || name == "i"
                    || name == "img"
                    || name == "li"
                    || name == "listing"
                    || name == "menu"
                    || name == "meta"
                    || name == "nobr"
                    || name == "ol"
                    || name == "p"
                    || name == "pre"
                    || name == "ruby"
                    || name == "s"
                    || name == "small"
                    || name == "span"
                    || name == "strong"
                    || name == "strike"
                    || name == "sub"
                    || name == "sup"
                    || name == "table"
                    || name == "tt"
                    || name == "u"
                    || name == "ul"
                    || name == "var" =>
            {
                self.process_unexpected_html_tag();
            }
            Token::StartTag {
                name, attributes, ..
            } if name == "font" => {
                if attributes.contains_key("color")
                    || attributes.contains_key("face")
                    || attributes.contains_key("size")
                {
                    self.process_unexpected_html_tag();
                }
            }
            Token::EndTag { name, .. } if name == "br" || name == "p" => {
                self.process_unexpected_html_tag();
            }
            Token::StartTag {
                name,
                is_self_closing,
                ..
            } => {
                let mut current_token = self.current_token.clone();

                let acn = self.get_adjusted_current_node();
                if acn.is_namespace(MATHML_NAMESPACE) {
                    self.adjust_mathml_attributes(&mut current_token);
                }

                if acn.is_namespace(SVG_NAMESPACE) {
                    self.adjust_svg_tag_names(&mut current_token);
                    self.adjust_svg_attributes(&mut current_token);
                }

                self.adjust_foreign_attributes(&mut current_token);
                self.insert_foreign_element(
                    &current_token,
                    acn.namespace.expect("namespace").as_str(),
                );

                if *is_self_closing {
                    if name == "script"
                        && current_node!(self).namespace.unwrap_or_default() == SVG_NAMESPACE
                    {
                        self.ack_self_closing = true;
                        handle_as_script_endtag = true;
                    } else {
                        self.open_elements.pop();
                        // self.current_token = self.fetch_next_token();
                        self.ack_self_closing = true;
                    }
                }
            }
            Token::EndTag { name, .. } if name == "script" => {
                handle_as_script_endtag = true;
            }
            Token::EndTag { name, .. } => {
                if self.open_elements.is_empty() {
                    return;
                }

                let mut node_idx = self.open_elements.len() - 1;
                let mut node = get_node_by_id!(self.document, self.open_elements[node_idx]);

                if node.name.to_lowercase() != *name {
                    self.parse_error("end tag does not match current node");
                }

                loop {
                    // Fragment case is when the first element in the stack is this node
                    match self.open_elements.get(0) {
                        // fragment case
                        Some(node_id) if *node_id == node.id => return,
                        _ => {}
                    }

                    if node.name.to_lowercase() == *name {
                        while let Some(node_id) = self.open_elements.pop() {
                            if node_id == node.id {
                                break;
                            }
                        }
                        return;
                    }

                    node_idx -= 1;
                    node = get_node_by_id!(self.document, self.open_elements[node_idx]);

                    if !node.is_namespace(HTML_NAMESPACE) {
                        continue;
                    }

                    self.process_html_content();
                    break;
                }
            }
            Token::Eof => {
                panic!("eof is not expected here");
            }
        }

        if handle_as_script_endtag {
            self.open_elements.pop();

            let old_insertion_point = self.insertion_point;
            self.insertion_point = Some(self.tokenizer.get_position().offset);

            self.script_nesting_level += 1;

            // @todo: do script stuff

            self.script_nesting_level -= 1;
            if self.script_nesting_level == 0 {
                self.parser_pause_flag = false;
            }

            self.insertion_point = old_insertion_point;
        }
    }

    /// Process a token in HTML content
    fn process_html_content(&mut self) {
        if self.ignore_lf {
            if let Token::Text(value) = &self.current_token {
                if value.eq(&"\n".to_string()) {
                    self.current_token = self.fetch_next_token();
                }
            }
            self.ignore_lf = false;
        }

        // // Break when we reach the end of the token stream
        // if self.current_token.is_eof() {
        //     self.parser_finished = true;
        // }

        match self.insertion_mode {
            InsertionMode::Initial => {
                let mut anything_else = false;

                match &self.current_token.clone() {
                    Token::Text(..) if self.current_token.is_empty_or_white() => {
                        // ignore token
                    }
                    Token::Comment(..) => {
                        self.insert_comment_element(
                            &self.current_token.clone(),
                            Some(NodeId::root()),
                        );
                    }
                    Token::DocType {
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

                        self.insert_doctype_element(&self.current_token.clone());

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
                    Token::Text(..) => {
                        if !self.is_iframesrcdoc() {
                            self.parse_error(ParserError::ExpectedDocTypeButGotChars.as_str());
                        }
                        anything_else = true;
                    }
                    _ => anything_else = true,
                }

                if anything_else {
                    if !self.parser_cannot_change_mode {
                        self.set_quirks_mode(QuirksMode::Quirks);
                    }

                    self.insertion_mode = InsertionMode::BeforeHtml;
                    self.reprocess_token = true;
                }
            }
            InsertionMode::BeforeHtml => {
                let mut anything_else = false;

                match &self.current_token {
                    Token::DocType { .. } => {
                        self.parse_error("doctype not allowed in before html insertion mode");
                    }
                    Token::Comment(..) => {
                        self.insert_comment_element(
                            &self.current_token.clone(),
                            Some(NodeId::root()),
                        );
                    }
                    Token::Text(..) if self.current_token.is_empty_or_white() => {
                        // ignore token
                    }
                    Token::StartTag { name, .. } if name == "html" => {
                        self.insert_document_element(&self.current_token.clone());

                        self.insertion_mode = InsertionMode::BeforeHead;
                    }
                    Token::EndTag { name, .. }
                        if name == "head" || name == "body" || name == "html" || name == "br" =>
                    {
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
                    let token = Token::StartTag {
                        name: "html".to_string(),
                        is_self_closing: false,
                        attributes: HashMap::new(),
                    };
                    self.insert_document_element(&token);

                    self.insertion_mode = InsertionMode::BeforeHead;
                    self.reprocess_token = true;
                }
            }
            InsertionMode::BeforeHead => {
                let mut anything_else = false;

                match &self.current_token {
                    Token::Text(..) if self.current_token.is_empty_or_white() => {
                        // ignore token
                    }
                    Token::Comment(..) => {
                        self.insert_comment_element(&self.current_token.clone(), None);
                    }
                    Token::DocType { .. } => {
                        self.parse_error("doctype not allowed in before head insertion mode");
                        // ignore token
                    }
                    Token::StartTag { name, .. } if name == "html" => {
                        self.handle_in_body();
                    }
                    Token::StartTag { name, .. } if name == "head" => {
                        let node_id = self.insert_html_element(&self.current_token.clone());
                        self.head_element = Some(node_id);
                        self.insertion_mode = InsertionMode::InHead;
                    }
                    Token::EndTag { name, .. }
                        if name == "head" || name == "body" || name == "html" || name == "br" =>
                    {
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
                    let token = Token::StartTag {
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
            InsertionMode::InHead => self.handle_in_head(),
            InsertionMode::InHeadNoscript => {
                let mut anything_else = false;

                match &self.current_token {
                    Token::DocType { .. } => {
                        self.parse_error("doctype not allowed in 'head no script' insertion mode");
                        // ignore token
                        return;
                    }
                    Token::StartTag { name, .. } if name == "html" => {
                        self.handle_in_body();
                    }
                    Token::EndTag { name, .. } if name == "noscript" => {
                        self.pop_check("noscript");
                        self.check_last_element("head");
                        self.insertion_mode = InsertionMode::InHead;
                    }
                    Token::Text(..) if self.current_token.is_empty_or_white() => {
                        self.handle_in_head();
                    }
                    Token::Comment(..) => {
                        self.handle_in_head();
                    }
                    Token::StartTag { name, .. }
                        if name == "basefont"
                            || name == "bgsound"
                            || name == "link"
                            || name == "meta"
                            || name == "noframes"
                            || name == "style" =>
                    {
                        self.handle_in_head();
                    }
                    Token::EndTag { name, .. } if name == "br" => {
                        anything_else = true;
                    }
                    Token::StartTag { name, .. } if name == "head" || name == "noscript" => {
                        self.parse_error(
                            "head or noscript tag not allowed in after head insertion mode",
                        );
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
                    self.check_last_element("head");

                    self.insertion_mode = InsertionMode::InHead;
                    self.reprocess_token = true;
                }
            }
            InsertionMode::AfterHead => {
                let mut anything_else = false;

                match &self.current_token {
                    Token::Text(..) if self.current_token.is_empty_or_white() => {
                        self.insert_text_element(&self.current_token.clone());
                    }
                    Token::Comment(..) => {
                        self.insert_comment_element(&self.current_token.clone(), None);
                    }
                    Token::DocType { .. } => {
                        self.parse_error("doctype not allowed in after head insertion mode");
                        // ignore token
                    }
                    Token::StartTag { name, .. } if name == "html" => {
                        self.handle_in_body();
                    }
                    Token::StartTag { name, .. } if name == "body" => {
                        self.insert_html_element(&self.current_token.clone());

                        self.frameset_ok = false;
                        self.insertion_mode = InsertionMode::InBody;
                    }
                    Token::StartTag { name, .. } if name == "frameset" => {
                        self.insert_html_element(&self.current_token.clone());

                        self.insertion_mode = InsertionMode::InFrameset;
                    }
                    Token::StartTag { name, .. }
                        if [
                            "base", "basefont", "bgsound", "link", "meta", "noframes", "script",
                            "style", "template", "title",
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
                            self.open_elements_remove(node_id);
                        }
                    }
                    Token::EndTag { name, .. } if name == "template" => {
                        self.handle_in_head();
                    }
                    Token::EndTag { name, .. }
                        if name == "body" || name == "html" || name == "br" =>
                    {
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
                    let token = Token::StartTag {
                        name: "body".to_string(),
                        is_self_closing: false,
                        attributes: HashMap::new(),
                    };
                    self.insert_html_element(&token);

                    self.insertion_mode = InsertionMode::InBody;
                    self.reprocess_token = true;
                }
            }
            InsertionMode::InBody => self.handle_in_body(),
            InsertionMode::Text => {
                match &self.current_token {
                    Token::Text(..) => {
                        self.insert_text_element(&self.current_token.clone());
                    }
                    Token::Eof => {
                        self.parse_error("eof not allowed in text insertion mode");

                        if current_node!(self).name == "script" {
                            self.script_already_started = true;
                        }
                        self.open_elements.pop();
                        self.insertion_mode = self.original_insertion_mode;
                        self.reprocess_token = true;
                    }
                    Token::EndTag { name, .. } if name == "script" => {
                        // @todo: If the active speculative HTML parser is null and the JavaScript execution context stack is empty, then perform a microtask checkpoint.

                        let _script = current_node!(self);

                        self.open_elements.pop();
                        self.insertion_mode = self.original_insertion_mode;

                        let old_insertion_point = self.insertion_point;
                        self.insertion_point = Some(self.tokenizer.get_position().offset);

                        self.script_nesting_level += 1;

                        // do script stuff

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
            InsertionMode::InTable => self.handle_in_table(),
            InsertionMode::InTableText => {
                match &self.current_token {
                    Token::Text(..) if self.current_token.is_null() => {
                        self.parse_error(
                            "null character not allowed in in table text insertion mode",
                        );
                        // ignore token
                    }
                    Token::Text(value) => {
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
                        let tokens = self.pending_table_character_tokens.clone();

                        let mut process_as_intable_anything_else = false;

                        for c in self.pending_table_character_tokens.chars() {
                            if !c.is_ascii_whitespace() {
                                self.parse_error(
                                    "non whitespace character in pending table character tokens",
                                );
                                process_as_intable_anything_else = true;
                                break;
                            }
                        }

                        if process_as_intable_anything_else {
                            let tmp = self.current_token.clone();
                            self.current_token = Token::Text(tokens);

                            self.foster_parenting = true;
                            self.handle_in_body();
                            self.foster_parenting = false;

                            self.current_token = tmp;
                        } else {
                            self.insert_text_element(&Token::Text(tokens));
                        }

                        self.pending_table_character_tokens.clear();

                        self.insertion_mode = self.original_insertion_mode;
                        self.reprocess_token = true;
                    }
                }
            }
            InsertionMode::InCaption => {
                let mut process_incaption_body = false;

                match &self.current_token {
                    Token::EndTag { name, .. } if name == "caption" => {
                        process_incaption_body = true;
                    }
                    Token::StartTag { name, .. }
                        if [
                            "caption", "col", "colgroup", "tbody", "td", "tfoot", "th", "thead",
                            "tr",
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
                    _ => self.handle_in_body(),
                }

                if process_incaption_body {
                    if !self.open_elements_has("caption") {
                        // fragment case
                        self.parse_error(
                            "caption end tag not allowed in in caption insertion mode",
                        );
                        // ignore token
                        self.reprocess_token = false;
                        return;
                    }

                    self.generate_implied_end_tags(None, false);

                    if current_node!(self).name != "caption" {
                        self.parse_error("caption end tag not at top of stack");
                    }

                    self.pop_until_named("caption");
                    self.active_formatting_elements_clear_until_marker();

                    self.insertion_mode = InsertionMode::InTable;
                }
            }
            InsertionMode::InColumnGroup => {
                match &self.current_token {
                    Token::Text(..) if self.current_token.is_empty_or_white() => {
                        self.insert_text_element(&self.current_token.clone());
                    }
                    Token::Comment(..) => {
                        self.insert_comment_element(&self.current_token.clone(), None);
                    }
                    Token::DocType { .. } => {
                        self.parse_error("doctype not allowed in column group insertion mode");
                        // ignore token
                    }
                    Token::StartTag { name, .. } if name == "html" => {
                        self.handle_in_body();
                    }
                    Token::StartTag {
                        name,
                        is_self_closing,
                        ..
                    } if name == "col" => {
                        self.acknowledge_closing_tag(*is_self_closing);

                        self.insert_html_element(&self.current_token.clone());
                        self.open_elements.pop();
                    }
                    Token::StartTag { name, .. } if name == "template" => {
                        self.handle_in_head();
                    }
                    Token::EndTag { name, .. } if name == "template" => {
                        self.handle_in_head();
                    }
                    Token::Eof => {
                        self.handle_in_body();
                    }
                    Token::EndTag { name, .. } if name == "colgroup" => {
                        if current_node!(self).name != "colgroup" {
                            self.parse_error("colgroup end tag not at top of stack");
                            // ignore token
                            return;
                        }

                        self.open_elements.pop();
                        self.insertion_mode = InsertionMode::InTable;
                        self.reprocess_token = true;
                    }
                    Token::EndTag { name, .. } if name == "col" => {
                        self.parse_error("col end tag not allowed in column group insertion mode");
                        // ignore token
                    }
                    _ => {
                        if current_node!(self).name != "colgroup" {
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
            InsertionMode::InTableBody => {
                match &self.current_token {
                    Token::StartTag { name, .. } if name == "tr" => {
                        self.clear_stack_back_to_table_body_context();

                        self.insert_html_element(&self.current_token.clone());

                        self.insertion_mode = InsertionMode::InRow;
                    }
                    Token::StartTag { name, .. } if name == "th" || name == "td" => {
                        self.parse_error(
                            "th or td tag not allowed in in table body insertion mode",
                        );

                        self.clear_stack_back_to_table_body_context();

                        let token = Token::StartTag {
                            name: "tr".to_string(),
                            is_self_closing: false,
                            attributes: HashMap::new(),
                        };
                        self.insert_html_element(&token);

                        self.insertion_mode = InsertionMode::InRow;
                        self.reprocess_token = true;
                    }
                    Token::EndTag { name, .. }
                        if name == "tbody" || name == "tfoot" || name == "thead" =>
                    {
                        if !self.is_in_scope(name, Scope::Table) {
                            self.parse_error("tbody, tfoot or thead tag not allowed in in table body insertion mode");
                            // ignore token
                            return;
                        }

                        self.clear_stack_back_to_table_body_context();
                        self.open_elements.pop();

                        self.insertion_mode = InsertionMode::InTable;
                    }
                    Token::StartTag { name, .. }
                        if ["caption", "col", "colgroup", "tbody", "tfoot", "thead"]
                            .contains(&name.as_str()) =>
                    {
                        if !self.is_in_scope("tbody", Scope::Table)
                            && !self.is_in_scope("tfoot", Scope::Table)
                            && !self.is_in_scope("thead", Scope::Table)
                        {
                            self.parse_error("caption, col, colgroup, tbody, tfoot or thead tag not allowed in in table body insertion mode");
                            // ignore token
                            return;
                        }

                        self.clear_stack_back_to_table_body_context();
                        self.open_elements.pop();

                        self.insertion_mode = InsertionMode::InTable;
                        self.reprocess_token = true;
                    }
                    Token::EndTag { name, .. } if name == "table" => {
                        if !self.is_in_scope("tbody", Scope::Table)
                            && !self.is_in_scope("tfoot", Scope::Table)
                            && !self.is_in_scope("thead", Scope::Table)
                        {
                            self.parse_error(
                                "table end tag not allowed in in table body insertion mode",
                            );
                            return;
                        }

                        self.clear_stack_back_to_table_body_context();
                        self.open_elements.pop();

                        self.insertion_mode = InsertionMode::InTable;
                        self.reprocess_token = true;
                    }
                    Token::EndTag { name, .. }
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
            InsertionMode::InRow => {
                match &self.current_token {
                    Token::StartTag { name, .. } if name == "th" || name == "td" => {
                        self.clear_stack_back_to_table_row_context();

                        self.insert_html_element(&self.current_token.clone());

                        self.insertion_mode = InsertionMode::InCell;
                        self.active_formatting_elements_push_marker();
                    }
                    Token::EndTag { name, .. } if name == "tr" => {
                        if !self.is_in_scope("tr", Scope::Table) {
                            self.parse_error("tr tag not allowed in in row insertion mode");
                            // ignore token
                            return;
                        }

                        self.clear_stack_back_to_table_row_context();
                        self.pop_check("tr");

                        self.insertion_mode = InsertionMode::InTableBody;
                    }
                    Token::StartTag { name, .. }
                        if [
                            "caption", "col", "colgroup", "tbody", "tfoot", "thead", "tr",
                        ]
                        .contains(&name.as_str()) =>
                    {
                        if !self.is_in_scope("tr", Scope::Table) {
                            self.parse_error("caption, col, colgroup, tbody, tfoot or thead tag not allowed in in row insertion mode");
                            // ignore token
                            return;
                        }

                        self.clear_stack_back_to_table_row_context();
                        self.pop_check("tr");

                        self.insertion_mode = InsertionMode::InTableBody;
                        self.reprocess_token = true;
                    }
                    Token::EndTag { name, .. } if name == "table" => {
                        if !self.is_in_scope("tr", Scope::Table) {
                            self.parse_error("table tag not allowed in in row insertion mode");
                            // ignore token
                            return;
                        }

                        self.clear_stack_back_to_table_row_context();
                        self.pop_check("tr");

                        self.insertion_mode = InsertionMode::InTableBody;
                        self.reprocess_token = true;
                    }
                    Token::EndTag { name, .. }
                        if name == "tbody" || name == "tfoot" || name == "thead" =>
                    {
                        if !self.is_in_scope(name, Scope::Table) {
                            self.parse_error("tbody, tfoot or thead tag not allowed in in table body insertion mode");
                            // ignore token
                            return;
                        }

                        if !self.is_in_scope("tr", Scope::Table) {
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
                    _ => self.handle_in_table(),
                }
            }
            InsertionMode::InCell => {
                match &self.current_token {
                    Token::EndTag { name, .. } if name == "th" || name == "td" => {
                        let token_name = name.clone();

                        if !self.is_in_scope(name.as_str(), Scope::Table) {
                            self.parse_error("th or td tag not allowed in in cell insertion mode");
                            // ignore token
                            return;
                        }
                        self.generate_implied_end_tags(None, false);

                        if current_node!(self).name != token_name {
                            self.parse_error("current node should be th or td");
                        }

                        self.pop_until_named(&token_name);

                        self.active_formatting_elements_clear_until_marker();

                        self.insertion_mode = InsertionMode::InRow;
                    }
                    Token::StartTag { name, .. }
                        if [
                            "caption", "col", "colgroup", "tbody", "td", "tfoot", "th", "thead",
                            "tr",
                        ]
                        .contains(&name.as_str()) =>
                    {
                        if !self.is_in_scope("td", Scope::Table)
                            && !self.is_in_scope("th", Scope::Table)
                        {
                            // fragment case
                            self.parse_error("caption, col, colgroup, tbody, tfoot or thead tag not allowed in in cell insertion mode");
                            // ignore token
                            return;
                        }

                        self.close_cell();
                        self.reprocess_token = true;
                    }
                    Token::EndTag { name, .. }
                        if name == "body"
                            || name == "caption"
                            || name == "col"
                            || name == "colgroup"
                            || name == "html" =>
                    {
                        self.parse_error("end tag not allowed in in cell insertion mode");
                        // ignore token
                    }
                    Token::EndTag { name, .. }
                        if name == "table"
                            || name == "tbody"
                            || name == "tfoot"
                            || name == "thead"
                            || name == "tr" =>
                    {
                        if !self.is_in_scope(name.as_str(), Scope::Table) {
                            self.parse_error("tbody, tfoot or thead tag not allowed in in table body insertion mode");
                            // ignore token
                            return;
                        }

                        self.close_cell();
                        self.reprocess_token = true;
                    }
                    _ => self.handle_in_body(),
                }
            }
            InsertionMode::InSelect => self.handle_in_select(),
            InsertionMode::InSelectInTable => {
                match &self.current_token {
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

                        if !self.is_in_scope(name, Scope::Table) {
                            // ignore token
                            return;
                        }

                        self.pop_until_named("select");
                        self.reset_insertion_mode();
                        self.reprocess_token = true;
                    }
                    _ => self.handle_in_select(),
                }
            }
            InsertionMode::InTemplate => self.handle_in_template(),
            InsertionMode::AfterBody => {
                match &self.current_token {
                    Token::Text(..) if self.current_token.is_empty_or_white() => {
                        self.handle_in_body();
                    }
                    Token::Comment(..) => {
                        let html_node_id = self.open_elements.first().unwrap_or_default();
                        self.insert_comment_element(
                            &self.current_token.clone(),
                            Some(*html_node_id),
                        );
                    }
                    Token::DocType { .. } => {
                        self.parse_error("doctype not allowed in after body insertion mode");
                        // ignore token
                    }
                    Token::StartTag { name, .. } if name == "html" => {
                        self.handle_in_body();
                    }
                    Token::EndTag { name, .. } if name == "html" => {
                        if self.is_fragment_case {
                            // fragment case
                            self.parse_error(
                                "html end tag not allowed in after body insertion mode",
                            );
                            // ignore token
                            return;
                        }
                        self.insertion_mode = InsertionMode::AfterAfterBody;
                    }
                    Token::Eof => {
                        self.stop_parsing();
                    }
                    _ => {
                        self.parse_error("anything else not allowed in after body insertion mode");
                        self.insertion_mode = InsertionMode::InBody;
                        self.reprocess_token = true;
                    }
                }
            }
            InsertionMode::InFrameset => {
                match &self.current_token {
                    Token::Text(..) if self.current_token.is_empty_or_white() => {
                        self.insert_text_element(&self.current_token.clone());
                    }
                    Token::Comment(..) => {
                        self.insert_comment_element(&self.current_token.clone(), None);
                    }
                    Token::DocType { .. } => {
                        self.parse_error("doctype not allowed in frameset insertion mode");
                        // ignore token
                    }
                    Token::StartTag { name, .. } if name == "html" => {
                        self.handle_in_body();
                    }
                    Token::StartTag { name, .. } if name == "frameset" => {
                        self.insert_html_element(&self.current_token.clone());
                    }
                    Token::EndTag { name, .. } if name == "frameset" => {
                        if current_node!(self).name == "html" {
                            // fragment case
                            self.parse_error("frameset tag not allowed in frameset insertion mode");
                            // ignore token
                            return;
                        }

                        self.open_elements.pop();

                        if !self.is_fragment_case && current_node!(self).name != "frameset" {
                            // fragment case
                            self.insertion_mode = InsertionMode::AfterFrameset;
                        }
                    }
                    Token::StartTag {
                        name,
                        is_self_closing,
                        ..
                    } if name == "frame" => {
                        self.acknowledge_closing_tag(*is_self_closing);

                        self.insert_html_element(&self.current_token.clone());
                        self.open_elements.pop();
                    }
                    Token::StartTag { name, .. } if name == "noframes" => {
                        self.handle_in_head();
                    }
                    Token::Eof => {
                        if current_node!(self).name != "html" {
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
            InsertionMode::AfterFrameset => {
                match &self.current_token {
                    Token::Text(..) if self.current_token.is_empty_or_white() => {
                        self.insert_text_element(&self.current_token.clone());
                    }
                    Token::Comment(..) => {
                        self.insert_comment_element(&self.current_token.clone(), None);
                    }
                    Token::DocType { .. } => {
                        self.parse_error("doctype not allowed in frameset insertion mode");
                        // ignore token
                    }
                    Token::StartTag { name, .. } if name == "html" => {
                        self.handle_in_body();
                    }
                    Token::EndTag { name, .. } if name == "html" => {
                        self.insertion_mode = InsertionMode::AfterAfterFrameset;
                    }
                    Token::StartTag { name, .. } if name == "noframes" => {
                        self.handle_in_head();
                    }
                    Token::Eof => {
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
            InsertionMode::AfterAfterBody => match &self.current_token {
                Token::Comment(..) => {
                    self.insert_comment_element(&self.current_token.clone(), Some(NodeId::root()));
                }
                Token::DocType { .. } => {
                    self.handle_in_body();
                }
                Token::Text(..) if self.current_token.is_empty_or_white() => {
                    self.handle_in_body();
                }
                Token::StartTag { name, .. } if name == "html" => {
                    self.handle_in_body();
                }
                Token::Eof => {
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
            InsertionMode::AfterAfterFrameset => {
                match &self.current_token {
                    Token::Comment(..) => {
                        self.insert_comment_element(
                            &self.current_token.clone(),
                            Some(NodeId::root()),
                        );
                    }
                    Token::DocType { .. } => {
                        self.handle_in_body();
                    }
                    Token::Text(..) if self.current_token.is_empty_or_white() => {
                        self.handle_in_body();
                    }
                    Token::StartTag { name, .. } if name == "html" => {
                        self.handle_in_body();
                    }
                    Token::Eof => {
                        self.stop_parsing();
                    }
                    Token::StartTag { name, .. } if name == "noframes" => {
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

    fn set_quirks_mode(&mut self, mode: QuirksMode) {
        self.document.get_mut().quirks_mode = mode;
    }

    fn is_iframesrcdoc(&self) -> bool {
        self.document.get().doctype == DocumentType::IframeSrcDoc
    }

    /// Enables or disables scripting
    pub fn enabled_scripting(&mut self, enabled: bool) {
        self.scripting_enabled = enabled;
    }

    fn acknowledge_closing_tag(&mut self, is_self_closing: bool) {
        if is_self_closing {
            self.ack_self_closing = true;
        }
    }

    /// Pops the last element from the open elements until we reach $name
    fn pop_until_named(&mut self, name: &str) {
        loop {
            if self.open_elements.is_empty() {
                break;
            }

            if current_node!(self).name == name && current_node!(self).is_namespace(HTML_NAMESPACE)
            {
                self.open_elements.pop();
                break;
            }

            self.open_elements.pop();
        }
    }

    /// Pops the last element from the open elements until we reach $name
    fn pop_until(&mut self, name: &str) {
        loop {
            if self.open_elements.is_empty() {
                break;
            }

            if current_node!(self).name == name {
                self.open_elements.pop();
                break;
            }

            self.open_elements.pop();
        }
    }

    /// Pops the last element from the open elements until we reach any of the elements in $arr
    fn pop_until_any(&mut self, arr: &[&str]) {
        while !self.open_elements.is_empty() {
            let node_id = self.open_elements.pop();
            if node_id.is_none() {
                break;
            }

            let tag = get_node_by_id!(self.document, node_id.expect("node_id not found")).name;
            if arr.contains(&tag.as_str()) {
                break;
            }
        }
    }

    /// Remove the given node_id from the open elements stack. Will do nothing when the node_id is not found
    fn open_elements_remove(&mut self, target_node_id: NodeId) {
        self.open_elements
            .retain(|&node_id| node_id != target_node_id);
    }

    /// Pops the last element from the open elements, and panics if it is not $name
    fn pop_check(&mut self, name: &str) {
        let node_id = self.open_elements.pop().expect("Open elements is empty");
        if get_node_by_id!(self.document, node_id).name != name {
            panic!("{} tag should be popped from open elements", name);
        }
    }

    /// Checks if the last element on the open elements is $name, and panics if not
    fn check_last_element(&self, name: &str) {
        let node_id = self.open_elements.last().unwrap_or_default();
        if get_node_by_id!(self.document, *node_id).name != name {
            panic!("{name} tag should be last element in open elements");
        }
    }

    /// Returns true when the open elements has $name
    fn open_elements_has(&self, name: &str) -> bool {
        self.open_elements.iter().rev().any(|node_id| {
            self.document
                .get()
                .get_node_by_id(*node_id)
                .expect("node not found")
                .name
                == name
        })
    }

    /// Retrieves a list of all errors generated by the parser/tokenizer
    pub fn get_parse_errors(&self) -> Vec<ParseError> {
        self.error_logger.borrow().get_errors().clone()
    }

    /// Send a parse error to the error logger
    fn parse_error(&self, message: &str) {
        self.error_logger
            .borrow_mut()
            .add_error(self.tokenizer.get_position(), message);
    }

    /// Create a new node that is not connected or attached to the document arena
    fn create_node(&self, token: &Token, namespace: &str) -> Node {
        match token {
            Token::DocType {
                name,
                pub_identifier,
                sys_identifier,
                ..
            } => Node::new_doctype(
                &self.document,
                &name.clone().unwrap_or_default(),
                &pub_identifier.clone().unwrap_or_default(),
                &sys_identifier.clone().unwrap_or_default(),
            ),
            Token::StartTag {
                name, attributes, ..
            } => Node::new_element(&self.document, name, attributes.clone(), namespace),
            Token::EndTag { name, .. } => {
                Node::new_element(&self.document, name, HashMap::new(), namespace)
            }
            Token::Comment(value) => Node::new_comment(&self.document, value),
            Token::Text(value) => Node::new_text(&self.document, value.to_string().as_str()),
            Token::Eof => {
                panic!("EOF token not allowed");
            }
        }
    }

    fn flush_pending_table_character_tokens(&mut self) {
        todo!()
    }

    /// This function will pop elements off the stack until it reaches the first element that matches
    /// our condition (which can be changed with the except and thoroughly parameters)
    fn generate_implied_end_tags(&mut self, except: Option<&str>, thoroughly: bool) {
        loop {
            if self.open_elements.is_empty() {
                return;
            }

            let tag = current_node!(self).name.clone();
            let is_html = current_node!(self).is_namespace(HTML_NAMESPACE);
            if let Some(except) = except {
                if except == tag && is_html {
                    return;
                }
            }
            if thoroughly {
                if !([
                    "tbody", "td", "tfoot", "th", "thead", "tr", "dd", "dt", "li", "option",
                    "optgroup", "p", "rb", "rp", "rt", "rtc",
                ]
                .contains(&tag.as_str())
                    && is_html)
                {
                    return;
                }
            } else if !([
                "dd", "dt", "li", "option", "optgroup", "p", "rb", "rp", "rt", "rtc",
            ]
            .contains(&tag.as_str())
                && is_html)
            {
                return;
            }

            self.open_elements.pop();
        }
    }

    /// Reset insertion mode based on all kind of rules
    fn reset_insertion_mode(&mut self) {
        let mut last = false;
        let mut idx = self.open_elements.len() - 1;

        loop {
            let mut node = open_elements_get!(self, idx);
            if idx == 0 {
                last = true;

                // fragment case
                if self.is_fragment_case {
                    node = get_node_by_id!(
                        self.context_doc.clone().expect("context_doc not found"),
                        self.context_node_id.expect("context_node_id not found")
                    )
                    .clone();
                }
            }
            match node.name.as_str() {
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
                        let ancestor = open_elements_get!(self, ancestor_idx);
                        match ancestor.name.as_str() {
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
                    self.insertion_mode = *self.template_insertion_mode.last().unwrap();
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

    /// Pop all elements back to a table context
    fn clear_stack_back_to_table_context(&mut self) {
        while !self.open_elements.is_empty() {
            if ["table", "template", "html"].contains(&current_node!(self).name.as_str()) {
                return;
            }
            self.open_elements.pop();
        }
    }

    /// Pop all elements back to a table context
    fn clear_stack_back_to_table_body_context(&mut self) {
        while !self.open_elements.is_empty() {
            if ["tbody", "tfoot", "thead", "template", "html"]
                .contains(&current_node!(self).name.as_str())
            {
                return;
            }
            self.open_elements.pop();
        }
    }

    /// Pop all elements back to a table row context
    fn clear_stack_back_to_table_row_context(&mut self) {
        while !self.open_elements.is_empty() {
            let val = current_node!(self).name.clone();
            if ["tr", "template", "html"].contains(&val.as_str()) {
                return;
            }
            self.open_elements.pop();
        }
    }

    /// Checks if the given element is in given scope
    fn is_in_scope(&self, tag: &str, scope: Scope) -> bool {
        for &node_id in self.open_elements.iter().rev() {
            let node = get_node_by_id!(self.document, node_id).clone();
            if node.name == tag {
                return true;
            }
            let default_html_scope = [
                "applet", "caption", "html", "table", "td", "th", "marquee", "object", "template",
            ];
            let default_mathml_scope = ["mo", "mi", "ms", "mn", "mtext", "annotation-xml"];
            let default_svg_scope = ["foreignObject", "desc", "title"];
            match scope {
                Scope::Regular => {
                    if (node.is_namespace(HTML_NAMESPACE)
                        && default_html_scope.contains(&node.name.as_str()))
                        || (node.is_namespace(MATHML_NAMESPACE)
                            && default_mathml_scope.contains(&node.name.as_str()))
                        || (node.is_namespace(SVG_NAMESPACE)
                            && default_svg_scope.contains(&node.name.as_str()))
                    {
                        return false;
                    }
                }
                Scope::ListItem => {
                    if (node.is_namespace(HTML_NAMESPACE)
                        && (default_html_scope.contains(&node.name.as_str())
                            || ["ol", "ul"].contains(&node.name.as_str())))
                        || (node.is_namespace(MATHML_NAMESPACE)
                            && default_mathml_scope.contains(&node.name.as_str()))
                        || (node.is_namespace(SVG_NAMESPACE)
                            && default_svg_scope.contains(&node.name.as_str()))
                    {
                        return false;
                    }
                }
                Scope::Button => {
                    if (node.is_namespace(HTML_NAMESPACE)
                        && (default_html_scope.contains(&node.name.as_str())
                            || node.name == "button"))
                        || (node.is_namespace(MATHML_NAMESPACE)
                            && default_mathml_scope.contains(&node.name.as_str()))
                        || (node.is_namespace(SVG_NAMESPACE)
                            && default_svg_scope.contains(&node.name.as_str()))
                    {
                        return false;
                    }
                }
                Scope::Table => {
                    if node.is_namespace(HTML_NAMESPACE)
                        && ["html", "template", "table"].contains(&node.name.as_str())
                    {
                        return false;
                    }
                }
                Scope::Select => {
                    if !(node.is_namespace(HTML_NAMESPACE)
                        && ["optgroup", "option"].contains(&node.name.as_str()))
                    {
                        return false;
                    }
                }
            }
        }

        false
    }

    /// Closes a table cell and switches the insertion mode to InRow
    fn close_cell(&mut self) {
        self.generate_implied_end_tags(None, false);

        let tag = current_node!(self).name;
        if tag != "td" && tag != "th" {
            self.parse_error("current node should be td or th");
        }

        self.pop_until_any(&["td", "th"]);

        self.active_formatting_elements_clear_until_marker();
        self.insertion_mode = InsertionMode::InRow;
    }

    /// Handle insertion mode "in_body"
    fn handle_in_body(&mut self) {
        match &self.current_token.clone() {
            Token::Text(..) if self.current_token.is_null() => {
                self.parse_error("null character not allowed in in body insertion mode");
                // ignore token
            }
            Token::Text(..) if self.current_token.is_empty_or_white() => {
                self.reconstruct_formatting();

                self.insert_text_element(&self.current_token.clone());
            }
            Token::Text(..) => {
                self.reconstruct_formatting();

                self.insert_text_element(&self.current_token.clone());

                self.frameset_ok = false;
            }
            Token::Comment(..) => {
                self.insert_comment_element(&self.current_token.clone(), None);
            }
            Token::DocType { .. } => {
                self.parse_error("doctype not allowed in in body insertion mode");
                // ignore token
            }
            Token::StartTag {
                name, attributes, ..
            } if name == "html" => {
                self.parse_error("html tag not allowed in in body insertion mode");

                if self.open_elements_has("template") {
                    // ignore token
                    return;
                }

                // Add attributes to html element
                let first_node_id = *self.open_elements.first().unwrap();
                let mut doc = self.document.get_mut();
                let first_node = doc
                    .get_node_by_id_mut(first_node_id)
                    .expect("node not found");
                if let NodeData::Element(element) = &mut first_node.data {
                    for (key, value) in attributes {
                        if !element.attributes.contains_key(key) {
                            element.attributes.insert(key.to_owned(), value.to_owned());
                        }
                    }
                };
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
                self.handle_in_head();
            }
            Token::EndTag { name, .. } if name == "template" => {
                self.handle_in_head();
            }
            Token::StartTag { name, .. } if name == "body" => {
                self.parse_error("body tag not allowed in in body insertion mode");

                if self.open_elements.len() == 1
                    || open_elements_get!(self, 1).name != "body"
                    || self.open_elements_has("template")
                {
                    // fragment case
                    // ignore token
                    return;
                }

                self.frameset_ok = false;

                // Add attributes to body element
                // @TODO add body attributes
            }
            Token::StartTag { name, .. } if name == "frameset" => {
                self.parse_error("frameset tag not allowed in in body insertion mode");

                if self.open_elements.len() == 1 || open_elements_get!(self, 1).name != "body" {
                    // ignore token
                    return;
                }

                if !self.frameset_ok {
                    // ignore token
                    return;
                }

                if self.open_elements.len() > 1 {
                    let second_node_id = self.open_elements[1];
                    let second_node = get_node_by_id!(self.document, second_node_id);
                    if second_node.parent.is_some() {
                        self.document
                            .get_mut()
                            .detach_node_from_parent(second_node_id);
                    }
                }

                while current_node!(self).name != "html" {
                    self.open_elements.pop();
                }

                self.insert_html_element(&self.current_token.clone());

                self.insertion_mode = InsertionMode::InFrameset;
            }
            Token::Eof => {
                if !self.template_insertion_mode.is_empty() {
                    self.handle_in_template();
                } else {
                    // @TODO: do stuff
                    self.stop_parsing();
                }
            }
            Token::EndTag { name, .. } if name == "body" => {
                if !self.is_in_scope("body", Scope::Regular) {
                    self.parse_error("body end tag not in scope");
                    // ignore token
                    return;
                }

                // @TODO: Other stuff

                self.insertion_mode = InsertionMode::AfterBody;
            }
            Token::EndTag { name, .. } if name == "html" => {
                if !self.is_in_scope("body", Scope::Regular) {
                    self.parse_error("body end tag not in scope");
                    // ignore token
                    return;
                }

                // @TODO: Other stuff

                self.insertion_mode = InsertionMode::AfterBody;
                self.reprocess_token = true;
            }
            Token::StartTag { name, .. }
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
                if self.is_in_scope("p", Scope::Button) {
                    self.close_p_element();
                }

                self.insert_html_element(&self.current_token.clone());
            }
            Token::StartTag { name, .. }
                if name == "h1"
                    || name == "h2"
                    || name == "h3"
                    || name == "h4"
                    || name == "h5"
                    || name == "h6" =>
            {
                if self.is_in_scope("p", Scope::Button) {
                    self.close_p_element();
                }

                if ["h1", "h2", "h3", "h4", "h5", "h6"].contains(&current_node!(self).name.as_str())
                {
                    self.parse_error("h1-h6 not allowed in in body insertion mode");
                    self.open_elements.pop();
                }

                self.insert_html_element(&self.current_token.clone());
            }
            Token::StartTag { name, .. } if name == "pre" || name == "listing" => {
                if self.is_in_scope("p", Scope::Button) {
                    self.close_p_element();
                }

                self.insert_html_element(&self.current_token.clone());

                self.ignore_lf = true;

                self.frameset_ok = false;
            }
            Token::StartTag { name, .. } if name == "form" => {
                if self.form_element.is_some() && !self.open_elements_has("template") {
                    self.parse_error("error with template, form shzzl");
                    // ignore token
                    return;
                }

                if self.is_in_scope("p", Scope::Button) {
                    self.close_p_element();
                }

                let node_id = self.insert_html_element(&self.current_token.clone());
                if !self.open_elements_has("template") {
                    self.form_element = Some(node_id);
                }
            }
            Token::StartTag { name, .. } if name == "li" => {
                self.frameset_ok = false;

                let mut idx = self.open_elements.len() - 1;
                loop {
                    let node = open_elements_get!(self, idx);
                    let tag = node.name.clone();

                    if tag == "li" {
                        self.generate_implied_end_tags(Some("li"), false);

                        if current_node!(self).name != "li" {
                            self.parse_error("li tag not at top of stack");
                        }

                        self.pop_until_named("li");
                        break;
                    }

                    if !["address", "div", "p"].contains(&tag.as_str()) && node.is_special() {
                        break;
                    }

                    idx -= 1;
                }

                if self.is_in_scope("p", Scope::Button) {
                    self.close_p_element();
                }

                self.insert_html_element(&self.current_token.clone());
            }
            Token::StartTag { name, .. } if name == "dd" || name == "dt" => {
                self.frameset_ok = false;

                let mut idx = self.open_elements.len() - 1;
                loop {
                    let node = open_elements_get!(self, idx);
                    let tag = node.name.clone();

                    if ["dd", "dt"].contains(&tag.as_str()) {
                        self.generate_implied_end_tags(Some(tag.as_str()), false);

                        if current_node!(self).name != tag {
                            self.parse_error("{tag} tag not at top of stack");
                        }

                        self.pop_until_named(tag.as_str());
                        break;
                    }

                    if !["address", "div", "p"].contains(&tag.as_str()) && node.is_special() {
                        break;
                    }

                    idx -= 1;
                }

                if self.is_in_scope("p", Scope::Button) {
                    self.close_p_element();
                }

                self.insert_html_element(&self.current_token.clone());
            }
            Token::StartTag { name, .. } if name == "plaintext" => {
                if self.is_in_scope("p", Scope::Button) {
                    self.close_p_element();
                }

                self.insert_html_element(&self.current_token.clone());

                self.tokenizer.state = State::PLAINTEXT;
            }
            Token::StartTag { name, .. } if name == "button" => {
                if self.is_in_scope("button", Scope::Regular) {
                    self.parse_error("button tag not allowed in in body insertion mode");
                    self.generate_implied_end_tags(None, false);
                    self.pop_until_named("button");
                }

                self.reconstruct_formatting();
                self.insert_html_element(&self.current_token.clone());
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
                if !self.is_in_scope(name, Scope::Regular) {
                    self.parse_error("end tag not in scope");
                    // ignore token
                    return;
                }

                self.generate_implied_end_tags(None, false);

                let cn = current_node!(self);
                if cn.name != *name {
                    self.parse_error("end tag not at top of stack");
                }

                self.pop_until_named(name);
            }
            Token::EndTag { name, .. } if name == "form" => {
                if !self.open_elements_has("template") {
                    let node_id = self.form_element;
                    self.form_element = None;

                    if node_id.is_none() || !self.is_in_scope(name, Scope::Regular) {
                        self.parse_error("end tag not in scope");
                        // ignore token
                        return;
                    }
                    let node_id = node_id.expect("node_id");

                    self.generate_implied_end_tags(None, false);

                    let cn = current_node!(self);
                    if cn.name != *name {
                        self.parse_error("end tag not at top of stack");
                    }

                    if node_id != cn.id {
                        self.parse_error("end tag not at top of stack");
                    }
                    self.open_elements_remove(node_id);
                } else {
                    if !self.is_in_scope(name, Scope::Regular) {
                        self.parse_error("end tag not in scope");
                        // ignore token
                        return;
                    }

                    self.generate_implied_end_tags(None, false);

                    let cn = current_node!(self);
                    if cn.name != *name {
                        self.parse_error("end tag not at top of stack");
                    }

                    self.pop_until_named(name);
                }
            }
            Token::EndTag { name, .. } if name == "p" => {
                if !self.is_in_scope(name, Scope::Button) {
                    self.parse_error("end tag not in scope");

                    let token = Token::StartTag {
                        name: "p".to_string(),
                        is_self_closing: false,
                        attributes: HashMap::new(),
                    };
                    self.insert_html_element(&token);
                }

                self.close_p_element();
            }
            Token::EndTag { name, .. } if name == "li" => {
                if !self.is_in_scope(name, Scope::ListItem) {
                    self.parse_error("end tag not in scope");
                    // ignore token
                    return;
                }

                self.generate_implied_end_tags(Some("li"), false);

                if current_node!(self).name != *name {
                    self.parse_error("end tag not at top of stack");
                }

                self.pop_until_named(name);
            }
            Token::EndTag { name, .. } if name == "dd" || name == "dt" => {
                if !self.is_in_scope(name, Scope::Regular) {
                    self.parse_error("end tag not in scope");
                    // ignore token
                    return;
                }

                self.generate_implied_end_tags(Some(name), false);

                if current_node!(self).name != *name {
                    self.parse_error("end tag not at top of stack");
                }

                self.pop_until_named(name);
            }
            Token::EndTag { name, .. }
                if name == "h1"
                    || name == "h2"
                    || name == "h3"
                    || name == "h4"
                    || name == "h5"
                    || name == "h6" =>
            {
                if ["h1", "h2", "h3", "h4", "h5", "h6"]
                    .iter()
                    .map(|tag| self.is_in_scope(tag, Scope::Regular))
                    .any(|res| res)
                {
                    self.generate_implied_end_tags(Some(name), false);

                    if current_node!(self).name != *name {
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
                    self.adoption_agency_algorithm(&self.current_token.clone());

                    // Remove from lists if not done already by the adoption agency
                    self.open_elements_remove(node_id);
                    self.active_formatting_elements_remove(node_id);
                }

                self.reconstruct_formatting();

                let node_id = self.insert_html_element(&self.current_token.clone());
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

                let node_id = self.insert_html_element(&self.current_token.clone());
                self.active_formatting_elements_push(node_id);
            }
            Token::StartTag { name, .. } if name == "nobr" => {
                self.reconstruct_formatting();

                if self.is_in_scope("nobr", Scope::Regular) {
                    self.parse_error("nobr tag in scope");
                    self.adoption_agency_algorithm(&self.current_token.clone());
                    self.reconstruct_formatting();
                }

                let node_id = self.insert_html_element(&self.current_token.clone());
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
                self.adoption_agency_algorithm(&self.current_token.clone());

                #[cfg(feature = "debug_parser")]
                self.display_debug_info();
            }
            Token::StartTag { name, .. }
                if name == "applet" || name == "marquee" || name == "object" =>
            {
                self.reconstruct_formatting();

                self.insert_html_element(&self.current_token.clone());

                self.active_formatting_elements_push_marker();
                self.frameset_ok = false;
            }
            Token::EndTag { name, .. }
                if name == "applet" || name == "marquee" || name == "object" =>
            {
                if !self.is_in_scope(name, Scope::Regular) {
                    self.parse_error("end tag not in scope");
                    // ignore token
                    return;
                }

                self.generate_implied_end_tags(None, false);

                if current_node!(self).name != *name {
                    self.parse_error("end tag not at top of stack");
                }

                self.pop_until_named(name);
                self.active_formatting_elements_clear_until_marker();
            }
            Token::StartTag { name, .. } if name == "table" => {
                if self.document.get_mut().quirks_mode != QuirksMode::Quirks
                    && self.is_in_scope("p", Scope::Button)
                {
                    self.close_p_element();
                }

                self.insert_html_element(&self.current_token.clone());

                self.frameset_ok = false;
                self.insertion_mode = InsertionMode::InTable;
            }
            Token::EndTag {
                name,
                is_self_closing,
                ..
            } if name == "br" => {
                self.parse_error("br end tag not allowed");
                self.reconstruct_formatting();

                // Remove attributes if any
                let mut br = self.current_token.clone();
                if let Token::StartTag { attributes, .. } = &mut br {
                    attributes.clear();
                }

                self.insert_html_element(&br);

                self.open_elements.pop();
                self.acknowledge_closing_tag(*is_self_closing);
                self.frameset_ok = false;
            }
            Token::StartTag {
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

                self.acknowledge_closing_tag(*is_self_closing);
                self.frameset_ok = false;
            }
            Token::StartTag {
                name,
                is_self_closing,
                attributes,
            } if name == "input" => {
                self.reconstruct_formatting();

                self.insert_html_element(&self.current_token.clone());
                self.open_elements.pop();

                self.acknowledge_closing_tag(*is_self_closing);

                if !attributes.contains_key("type")
                    || attributes.get("type").unwrap().to_lowercase() != *"hidden"
                {
                    self.frameset_ok = false;
                }
            }
            Token::StartTag {
                name,
                is_self_closing,
                ..
            } if name == "param" || name == "source" || name == "track" => {
                self.insert_html_element(&self.current_token.clone());
                self.open_elements.pop();

                self.acknowledge_closing_tag(*is_self_closing);
            }
            Token::StartTag {
                name,
                is_self_closing,
                ..
            } if name == "hr" => {
                if self.is_in_scope("p", Scope::Button) {
                    self.close_p_element();
                }

                self.insert_html_element(&self.current_token.clone());
                self.open_elements.pop();

                self.acknowledge_closing_tag(*is_self_closing);
                self.frameset_ok = false;
            }
            Token::StartTag {
                name,
                is_self_closing,
                attributes,
            } if name == "image" => {
                self.parse_error("image tag not allowed");
                self.current_token = Token::StartTag {
                    name: "img".to_string(),
                    attributes: attributes.clone(),
                    is_self_closing: *is_self_closing,
                };
                self.reprocess_token = true;
            }
            Token::StartTag { name, .. } if name == "textarea" => {
                self.insert_html_element(&self.current_token.clone());

                self.ignore_lf = true;

                self.tokenizer.state = State::RCDATA;
                self.original_insertion_mode = self.insertion_mode;
                self.frameset_ok = false;
                self.insertion_mode = InsertionMode::Text;
            }
            Token::StartTag { name, .. } if name == "xmp" => {
                if self.is_in_scope("p", Scope::Button) {
                    self.close_p_element();
                }

                self.reconstruct_formatting();

                self.frameset_ok = false;
                self.parse_raw_data();
            }
            Token::StartTag { name, .. } if name == "iframe" => {
                self.frameset_ok = false;
                self.parse_raw_data();
            }
            Token::StartTag { name, .. } if name == "noembed" => {
                self.parse_raw_data();
            }
            Token::StartTag { name, .. } if name == "noscript" && self.scripting_enabled => {
                self.parse_raw_data();
            }
            Token::StartTag { name, .. } if name == "select" => {
                self.reconstruct_formatting();

                self.insert_html_element(&self.current_token.clone());
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
                if current_node!(self).name == "option" {
                    self.open_elements.pop();
                }

                self.reconstruct_formatting();

                self.insert_html_element(&self.current_token.clone());
            }
            Token::StartTag { name, .. } if name == "rb" || name == "rtc" => {
                if self.is_in_scope("ruby", Scope::Regular) {
                    self.generate_implied_end_tags(None, false);
                }

                if current_node!(self).name != "ruby" {
                    self.parse_error("rb or rtc not in scope");
                }

                self.insert_html_element(&self.current_token.clone());
            }
            Token::StartTag { name, .. } if name == "rp" || name == "rt" => {
                if self.is_in_scope("ruby", Scope::Regular) {
                    self.generate_implied_end_tags(Some("rtc"), false);
                }

                if current_node!(self).name != "rtc" && current_node!(self).name != "ruby" {
                    self.parse_error("rp or rt not in scope");
                }

                self.insert_html_element(&self.current_token.clone());
            }
            Token::StartTag {
                name,
                is_self_closing,
                attributes,
            } if name == "math" => {
                self.reconstruct_formatting();

                let mut token = Token::StartTag {
                    name: name.clone(),
                    attributes: attributes.clone(),
                    is_self_closing: *is_self_closing,
                };
                self.adjust_mathml_attributes(&mut token);
                self.adjust_foreign_attributes(&mut token);

                self.insert_foreign_element(&token, MATHML_NAMESPACE);

                if *is_self_closing {
                    self.open_elements.pop();
                    self.acknowledge_closing_tag(*is_self_closing);
                }
            }
            Token::StartTag {
                name,
                is_self_closing,
                attributes,
            } if name == "svg" => {
                self.reconstruct_formatting();

                let mut token = Token::StartTag {
                    name: name.clone(),
                    attributes: attributes.clone(),
                    is_self_closing: *is_self_closing,
                };

                self.adjust_svg_attributes(&mut token);
                self.adjust_foreign_attributes(&mut token);
                self.insert_foreign_element(&token, SVG_NAMESPACE);

                if *is_self_closing {
                    self.open_elements.pop();
                    self.acknowledge_closing_tag(*is_self_closing);
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
                self.insert_html_element(&self.current_token.clone());
            }
            Token::EndTag { name, .. } => {
                self.handle_in_body_any_other_end_tag(name);
            }
        }
    }

    /// Handle insertion mode "in_head"
    fn handle_in_head(&mut self) {
        let mut anything_else = false;

        match &self.current_token {
            Token::Text(..) if self.current_token.is_empty_or_white() => {
                self.insert_text_element(&self.current_token.clone());
            }
            Token::Comment(..) => {
                self.insert_comment_element(&self.current_token.clone(), None);
            }
            Token::DocType { .. } => {
                self.parse_error("doctype not allowed in before head insertion mode");
                // ignore token
            }
            Token::StartTag { name, .. } if name == "html" => {
                self.handle_in_body();
            }
            Token::StartTag {
                name,
                is_self_closing,
                ..
            } if name == "base" || name == "basefont" || name == "bgsound" || name == "link" => {
                self.acknowledge_closing_tag(*is_self_closing);

                self.insert_html_element(&self.current_token.clone());
                self.open_elements.pop();
            }
            Token::StartTag {
                name,
                is_self_closing,
                ..
            } if name == "meta" => {
                self.acknowledge_closing_tag(*is_self_closing);

                self.insert_html_element(&self.current_token.clone());
                self.open_elements.pop();

                // @TODO: if active speculative html parser is null then...
                // we probably want to change the encoding if the element has a charset attribute and the current encoding is "tentative"
            }
            Token::StartTag { name, .. } if name == "title" => {
                self.parse_rcdata();
            }
            Token::StartTag { name, .. } if name == "noscript" && self.scripting_enabled => {
                self.parse_raw_data();
            }
            Token::StartTag { name, .. } if name == "noframes" || name == "style" => {
                self.parse_raw_data();
            }
            Token::StartTag { name, .. } if name == "noscript" && !self.scripting_enabled => {
                self.insert_html_element(&self.current_token.clone());
                self.insertion_mode = InsertionMode::InHeadNoscript;
            }
            Token::StartTag { name, .. } if name == "script" => {
                let insert_position = self.appropriate_place_insert(None);
                let node = self.create_node(&self.current_token.clone(), HTML_NAMESPACE);
                let node_id = self.document.get_mut().add_new_node(node);
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
            Token::StartTag { name, .. } if name == "template" => {
                let node_id = self.insert_html_element(&self.current_token.clone());

                {
                    let current_node_id = current_node!(self).id;

                    let doc = Document::clone(&self.document);
                    let mut doc_mut = self.document.get_mut();
                    let node = doc_mut.get_node_by_id_mut(node_id).expect("node not found");
                    if let NodeData::Element(ref mut data) = node.data {
                        data.template_contents = Some(DocumentFragment::new(doc, current_node_id));
                    }
                }

                self.active_formatting_elements_push_marker();
                self.frameset_ok = false;
                self.insertion_mode = InsertionMode::InTemplate;
                self.template_insertion_mode.push(InsertionMode::InTemplate);
            }
            Token::EndTag { name, .. } if name == "template" => {
                if !self.open_elements_has("template") {
                    self.parse_error("could not find template tag in open element stack");
                    // ignore token
                    return;
                }

                self.generate_implied_end_tags(None, true);

                if current_node!(self).name != "template" {
                    self.parse_error("template end tag not at top of stack");
                }

                self.pop_until_named("template");
                self.active_formatting_elements_clear_until_marker();
                self.reset_insertion_mode();
                self.template_insertion_mode.pop();
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

    /// Handle insertion mode "in_template"
    fn handle_in_template(&mut self) {
        match &self.current_token {
            Token::Text(..) => {
                self.handle_in_body();
            }
            Token::Comment(..) => {
                self.handle_in_body();
            }
            Token::DocType { .. } => {
                self.handle_in_body();
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
                self.handle_in_head();
            }
            Token::EndTag { name, .. } if name == "template" => {
                self.handle_in_head();
            }
            Token::StartTag { name, .. }
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
            Token::StartTag { name, .. } if name == "col" => {
                self.template_insertion_mode.pop();
                self.template_insertion_mode
                    .push(InsertionMode::InColumnGroup);
                self.insertion_mode = InsertionMode::InColumnGroup;
                self.reprocess_token = true;
            }
            Token::StartTag { name, .. } if name == "tr" => {
                self.template_insertion_mode.pop();
                self.template_insertion_mode
                    .push(InsertionMode::InTableBody);
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
            Token::Eof => {
                if !self.open_elements_has("template") {
                    // fragment case
                    self.stop_parsing();
                    return;
                }

                self.parse_error("eof not allowed in in template insertion mode");

                self.pop_until_named("template");
                self.active_formatting_elements_clear_until_marker();
                self.reset_insertion_mode();
                self.template_insertion_mode.pop();
                self.reprocess_token = true;
            }
        }
    }

    /// Handle insertion mode "in_table"
    fn handle_in_table(&mut self) {
        let mut anything_else = false;

        match &self.current_token {
            Token::Text(..)
                if ["table", "tbody", "template", "tfoot", "tr"]
                    .iter()
                    .any(|&node| node == current_node!(self).name) =>
            {
                self.pending_table_character_tokens = String::new();
                self.original_insertion_mode = self.insertion_mode;
                self.insertion_mode = InsertionMode::InTableText;
                self.reprocess_token = true;
            }
            Token::Comment(..) => {
                self.insert_comment_element(&self.current_token.clone(), None);
            }
            Token::DocType { .. } => {
                self.parse_error("doctype not allowed in in table insertion mode");
                // ignore token
            }
            Token::StartTag { name, .. } if name == "caption" => {
                self.clear_stack_back_to_table_context();
                self.active_formatting_elements_push_marker();
                self.insert_html_element(&self.current_token.clone());
                self.insertion_mode = InsertionMode::InCaption;
            }
            Token::StartTag { name, .. } if name == "colgroup" => {
                self.clear_stack_back_to_table_context();
                self.insert_html_element(&self.current_token.clone());
                self.insertion_mode = InsertionMode::InColumnGroup;
            }
            Token::StartTag { name, .. } if name == "col" => {
                self.clear_stack_back_to_table_context();

                let token = Token::StartTag {
                    name: "colgroup".to_string(),
                    is_self_closing: false,
                    attributes: HashMap::new(),
                };
                self.insert_html_element(&token);

                self.insertion_mode = InsertionMode::InColumnGroup;
                self.reprocess_token = true;
            }
            Token::StartTag { name, .. }
                if name == "tbody" || name == "tfoot" || name == "thead" =>
            {
                self.clear_stack_back_to_table_context();

                self.insert_html_element(&self.current_token.clone());

                self.insertion_mode = InsertionMode::InTableBody;
            }
            Token::StartTag { name, .. } if name == "td" || name == "th" || name == "tr" => {
                self.clear_stack_back_to_table_context();

                let token = Token::StartTag {
                    name: "tbody".to_string(),
                    is_self_closing: false,
                    attributes: HashMap::new(),
                };
                self.insert_html_element(&token);

                self.insertion_mode = InsertionMode::InTableBody;
                self.reprocess_token = true;
            }
            Token::StartTag { name, .. } if name == "table" => {
                self.parse_error("table tag not allowed in in table insertion mode");

                if !self.open_elements_has("table") {
                    // ignore token
                    return;
                }

                self.pop_until_named("table");
                self.reset_insertion_mode();
                self.reprocess_token = true;
            }
            Token::EndTag { name, .. } if name == "table" => {
                if !self.open_elements_has("table") {
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
            Token::StartTag { name, .. }
                if name == "style" || name == "script" || name == "template" =>
            {
                self.handle_in_head();
            }
            Token::EndTag { name, .. } if name == "template" => {
                self.handle_in_head();
            }
            Token::StartTag {
                name,
                is_self_closing,
                attributes,
            } if name == "input" => {
                if !attributes.contains_key("type")
                    || attributes.get("type").unwrap().to_lowercase() != *"hidden"
                {
                    anything_else = true;
                } else {
                    self.parse_error("input tag not allowed in in table insertion mode");

                    self.acknowledge_closing_tag(*is_self_closing);

                    self.insert_html_element(&self.current_token.clone());
                    self.pop_check("input");
                }
            }
            Token::StartTag { name, .. } if name == "form" => {
                self.parse_error("form tag not allowed in in table insertion mode");

                if self.open_elements_has("template") || self.form_element.is_some() {
                    // ignore token
                    return;
                }

                let node_id = self.insert_html_element(&self.current_token.clone());
                self.form_element = Some(node_id);

                self.pop_check("form");
            }
            Token::Eof => {
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

    /// Handle insertion mode "in_select"
    fn handle_in_select(&mut self) {
        match &self.current_token {
            Token::Text(..) if self.current_token.is_null() => {
                self.parse_error("null character not allowed in in select insertion mode");
                // ignore token
            }
            Token::Text(..) => {
                self.insert_text_element(&self.current_token.clone());
            }
            Token::Comment(..) => {
                self.insert_comment_element(&self.current_token.clone(), None);
            }
            Token::DocType { .. } => {
                self.parse_error("doctype not allowed in in select insertion mode");
                // ignore token
            }
            Token::StartTag { name, .. } if name == "html" => {
                self.handle_in_body();
            }
            Token::StartTag { name, .. } if name == "option" => {
                if current_node!(self).name == "option" {
                    self.open_elements.pop();
                }

                self.insert_html_element(&self.current_token.clone());
            }
            Token::StartTag { name, .. } if name == "optgroup" => {
                if current_node!(self).name == "option" {
                    self.open_elements.pop();
                }

                if current_node!(self).name == "optgroup" {
                    self.open_elements.pop();
                }

                self.insert_html_element(&self.current_token.clone());
            }
            Token::StartTag {
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

                self.acknowledge_closing_tag(*is_self_closing);

                self.insert_html_element(&self.current_token.clone());
                self.open_elements.pop();
            }
            Token::EndTag { name, .. } if name == "optgroup" => {
                if current_node!(self).name == "option"
                    && self.open_elements.len() > 1
                    && open_elements_get!(self, self.open_elements.len() - 2).name == "optgroup"
                {
                    self.open_elements.pop();
                }

                if current_node!(self).name == "optgroup" {
                    self.open_elements.pop();
                } else {
                    self.parse_error("optgroup end tag not allowed in in select insertion mode");
                    // ignore token
                }
            }
            Token::EndTag { name, .. } if name == "option" => {
                if current_node!(self).name == "option" {
                    self.open_elements.pop();
                } else {
                    self.parse_error("option end tag not allowed in in select insertion mode");
                    // ignore token
                }
            }
            Token::EndTag { name, .. } if name == "select" => {
                if !self.is_in_scope("select", Scope::Select) {
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

                if !self.is_in_scope("select", Scope::Select) {
                    // fragment case
                    // ignore token
                    return;
                }

                self.pop_until_named("select");
                self.reset_insertion_mode();
            }
            Token::StartTag { name, .. }
                if name == "input" || name == "keygen" || name == "textarea" =>
            {
                self.parse_error(
                    "input, keygen or textarea tag not allowed in in select insertion mode",
                );

                if !self.is_in_scope("select", Scope::Select) {
                    // fragment case
                    // ignore token
                    return;
                }

                self.pop_until_named("select");
                self.reset_insertion_mode();
                self.reprocess_token = true;
            }

            Token::StartTag { name, .. } if name == "script" || name == "template" => {
                self.handle_in_head();
            }
            Token::EndTag { name, .. } if name == "template" => {
                self.handle_in_head();
            }
            Token::Eof => {
                self.handle_in_body();
            }
            _ => {
                self.parse_error("anything else not allowed in in select insertion mode");
                // ignore token
            }
        }
    }

    /// Returns true if the given tag if found in the active formatting elements list (until the first marker)
    fn active_formatting_elements_has_until_marker(&self, tag: &str) -> Option<NodeId> {
        if self.active_formatting_elements.is_empty() {
            return None;
        }

        let mut idx = self.active_formatting_elements.len() - 1;
        loop {
            match self.active_formatting_elements[idx] {
                ActiveElement::Marker => return None,
                ActiveElement::Node(node_id) => {
                    if get_node_by_id!(self.document, node_id).name == tag {
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

    /// Adds a marker to the active formatting stack
    fn active_formatting_elements_push_marker(&mut self) {
        self.active_formatting_elements.push(ActiveElement::Marker);
    }

    /// Clear the active formatting stack until we reach the first marker
    fn active_formatting_elements_clear_until_marker(&mut self) {
        while let Some(active_elem) = self.active_formatting_elements.pop() {
            if let ActiveElement::Marker = active_elem {
                // Found the marker
                return;
            }
        }
    }

    /// Remove the given node_id from the active formatting elements list. Will do nothing when the node is not found
    fn active_formatting_elements_remove(&mut self, target_node_id: NodeId) {
        self.active_formatting_elements
            .retain(|node_id| match node_id {
                ActiveElement::Node(node_id) => *node_id != target_node_id,
                _ => true,
            });
    }

    /// Push a node onto the active formatting stack, make sure only max 3 of them can be added (between markers)
    fn active_formatting_elements_push(&mut self, node_id: NodeId) {
        let mut matched = 0;
        let mut first_matched = None;
        let node = get_node_by_id!(self.document, node_id);
        for entry in self.active_formatting_elements.iter().rev() {
            match entry {
                ActiveElement::Marker => break,
                &ActiveElement::Node(id) => {
                    let current_node = get_node_by_id!(self.document, id);
                    if current_node.matches_tag_and_attrs_without_order(&node) {
                        if matched >= 2 {
                            first_matched = Some(id);
                            break;
                        }
                        matched += 1;
                    }
                }
            }
        }
        if let Some(first_matched) = first_matched {
            self.active_formatting_elements
                .retain(|n| n != &ActiveElement::Node(first_matched));
        }

        self.active_formatting_elements
            .push(ActiveElement::Node(node_id));
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

        if self
            .open_elements
            .contains(&entry.node_id().expect("node id not found"))
        {
            return;
        }

        loop {
            // If it's a marker or in the stack of open elements, nothing to reconstruct.
            let entry = self.active_formatting_elements[entry_index];
            if let ActiveElement::Marker = entry {
                entry_index += 1;
                break;
            }

            if self
                .open_elements
                .contains(&entry.node_id().expect("node id not found"))
            {
                entry_index += 1;
                break;
            }

            if entry_index == 0 {
                break;
            }

            entry_index -= 1;
        }

        loop {
            let entry = self.active_formatting_elements[entry_index];
            if let ActiveElement::Marker = entry {
                // Marker found. This should not happen!
                break;
            }
            let node_id = entry.node_id().expect("node id not found");

            let entry_node = get_node_by_id!(self.document, node_id).clone();
            let new_node_id = self.insert_element_from_node(entry_node, None);

            self.active_formatting_elements[entry_index] = ActiveElement::Node(new_node_id);

            if entry_index == self.active_formatting_elements.len() - 1 {
                break;
            }

            entry_index += 1;
        }
    }

    fn stop_parsing(&mut self) {
        self.parser_finished = true;
    }

    /// Close the p element that may or may not be on the open elements stack
    fn close_p_element(&mut self) {
        self.generate_implied_end_tags(Some("p"), false);

        if current_node!(self).name != "p" {
            self.parse_error("p element not at top of stack");
        }

        self.pop_until_named("p");
    }

    /// Adjusts attributes names in the given token for SVG
    fn adjust_svg_attributes(&self, token: &mut Token) {
        if let Token::StartTag { attributes, .. } = token {
            let mut new_attributes = HashMap::new();
            for (name, value) in attributes.iter() {
                if SVG_ADJUSTMENTS_ATTRIBUTES.contains_key(name) {
                    let new_name = SVG_ADJUSTMENTS_ATTRIBUTES
                        .get(name)
                        .expect("svg adjustments");
                    new_attributes.insert(new_name.to_string(), value.clone());
                } else {
                    new_attributes.insert(name.clone(), value.clone());
                }
            }
            *attributes = new_attributes;
        }
    }

    /// Adjusts tag name in the given token for SVG
    fn adjust_svg_tag_names(&self, token: &mut Token) {
        if let Token::StartTag { name, .. } = token {
            if SVG_ADJUSTMENTS_TAGS.contains_key(name) {
                *name = SVG_ADJUSTMENTS_TAGS
                    .get(name)
                    .expect("svg tagname")
                    .to_string();
            }
        }
    }

    // Adjust attribute names in the given token for MathML
    fn adjust_mathml_attributes(&self, token: &mut Token) {
        if let Token::StartTag { attributes, .. } = token {
            let mut new_attributes = HashMap::new();
            for (name, value) in attributes.iter() {
                if MATHML_ADJUSTMENTS.contains_key(name) {
                    let new_name = MATHML_ADJUSTMENTS.get(name).expect("svg adjustments");
                    new_attributes.insert(new_name.to_string(), value.clone());
                } else {
                    new_attributes.insert(name.clone(), value.clone());
                }
            }
            *attributes = new_attributes;
        }
    }

    fn adjust_foreign_attributes(&self, token: &mut Token) {
        if let Token::StartTag { attributes, .. } = token {
            let mut new_attributes = HashMap::new();
            for (name, value) in attributes.iter() {
                if XML_ADJUSTMENTS.contains_key(name) {
                    let (prefix, local_name, _namespace) =
                        XML_ADJUSTMENTS.get(name).expect("cml adjustments");
                    new_attributes.insert(format!("{} {}", prefix, local_name), value.clone());
                } else {
                    new_attributes.insert(name.clone(), value.clone());
                }
            }
            *attributes = new_attributes;
        }
    }

    /// Switch the parser and tokenizer to the RAWTEXT state
    fn parse_raw_data(&mut self) {
        self.insert_html_element(&self.current_token.clone());

        self.tokenizer.state = State::RAWTEXT;

        self.original_insertion_mode = self.insertion_mode;
        self.insertion_mode = InsertionMode::Text;
    }

    /// Switch the parser and tokenizer to the RCDATA state
    fn parse_rcdata(&mut self) {
        self.insert_html_element(&self.current_token.clone());

        self.tokenizer.state = State::RCDATA;

        self.original_insertion_mode = self.insertion_mode;
        self.insertion_mode = InsertionMode::Text;
    }

    #[cfg(feature = "debug_parser")]
    fn display_debug_info(&self) {
        println!("-----------------------------------------\n");
        self.document.get().print_nodes();
        println!("-----------------------------------------\n");
        println!("current token   : '{}'", self.current_token);
        println!("insertion mode  : {:?}", self.insertion_mode);
        print!("Open elements   : [ ");
        for node_id in &self.open_elements {
            let node = get_node_by_id!(self.document, *node_id);
            print!("({}) {}, ", node_id, node.name);
        }
        println!("]");

        print!("Active elements : [");
        for elem in &self.active_formatting_elements {
            match elem {
                ActiveElement::Node(node_id) => {
                    let node = get_node_by_id!(self.document, *node_id);
                    print!("({}) {}, ", node_id, node.name);
                }
                ActiveElement::Marker => {
                    print!("marker, ");
                }
            }
        }
        println!("]");

        println!("Output:");
        println!("{}", self.document);

        std::io::stdout().flush().ok();
    }

    /// Handles and other end tag as found during the in-body insertion mode. This needs to be a
    /// separate function as this is also called during the adoption agency algorithm
    fn handle_in_body_any_other_end_tag(&mut self, tag_name: &str) {
        if self.open_elements.is_empty() {
            self.parse_error("no open elements");
            // ignore token
            return;
        }

        for idx in (0..self.open_elements.len()).rev() {
            let node_id = self.open_elements[idx];
            let node = get_node_by_id!(self.document, node_id).clone();

            if node.name == tag_name {
                self.generate_implied_end_tags(Some(node.name.as_str()), false);

                // It might be possible that the last item is not our node_id. Emit parse error if so
                if current_node!(self).id != node.id {
                    self.parse_error("end tag not at top of stack");
                }

                // Pop until we reach the node.id
                while current_node!(self).id != node.id {
                    self.open_elements.pop();
                }
                // Pop node_id as well
                self.open_elements.pop();

                break;
            }

            if node.is_special() {
                self.parse_error("special node");
                // ignore token
                return;
            }
        }
    }

    /// Fetches the next token from the tokenizer. However, if the token is a text token AND
    /// it starts with one or more whitespaces, the token is split into 2 tokens: the whitespace part
    /// and the remainder.
    fn fetch_next_token(&mut self) -> Token {
        // If there are no tokens to fetch, fetch the next token from the tokenizer
        if self.token_queue.is_empty() {
            let token = self.tokenizer.next_token().expect("tokenizer error");

            if let Token::Text(value) = token {
                for c in value.chars() {
                    self.token_queue.push(Token::Text(c.to_string()));
                }
            } else {
                // Simply return the token
                return token;
            }
        }

        let token = self.token_queue.get(0).cloned();
        self.token_queue.remove(0);

        token.expect("no token found")
    }

    fn get_adjusted_current_node(&self) -> Node {
        if self.is_fragment_case && self.open_elements.len() == 1 {
            // fragment case
            return get_node_by_id!(
                self.context_doc.clone().expect("context doc not found"),
                self.context_node_id.expect("context node not found")
            )
            .clone();
        }

        current_node!(self)
    }

    /// Checks the current token, node and parser context to see if the parser needs to switch to
    /// the foreign content or html content mode.
    fn select_dispatch_mode(&self) -> DispatcherMode {
        let acn = self.get_adjusted_current_node();

        if self.open_elements.is_empty() {
            return DispatcherMode::Html;
        }

        if acn.is_namespace(HTML_NAMESPACE) {
            return DispatcherMode::Html;
        }

        if acn.is_mathml_integration_point()
            && (!self.current_token.is_start_tag("mglyph")
                && !self.current_token.is_start_tag("malignmark"))
        {
            return DispatcherMode::Html;
        }

        if acn.is_mathml_integration_point() && self.current_token.is_text_token() {
            return DispatcherMode::Html;
        }

        if acn.is_namespace(MATHML_NAMESPACE)
            && acn.name == "annotation-xml"
            && self.current_token.is_start_tag("svg")
        {
            return DispatcherMode::Html;
        }

        if acn.is_html_integration_point() && self.current_token.is_any_start_tag() {
            return DispatcherMode::Html;
        }

        if acn.is_html_integration_point() && self.current_token.is_text_token() {
            return DispatcherMode::Html;
        }

        if self.current_token.is_eof() {
            return DispatcherMode::Html;
        }

        DispatcherMode::Foreign
    }

    /// Finds the node where to place an unexpected html tag. This can only be done on a mathml
    /// insertion point, a svg_html insertion point, or at a regular html namespaced node.
    fn process_unexpected_html_tag(&mut self) {
        self.parse_error("process_unexpected_html_tag");

        let mut current_node = current_node!(self);

        while !current_node.is_mathml_integration_point()
            && !current_node.is_html_integration_point()
            && !current_node.is_namespace(HTML_NAMESPACE)
        {
            self.open_elements.pop();
            if self.open_elements.is_empty() {
                return;
            }

            current_node = current_node!(self);
        }

        // Process as HTML content
        self.process_html_content();
    }

    /// Find the correct tokenizer state when we are about to parse a fragment case
    fn find_initial_state_for_context(&self, context_node: &Node) -> State {
        match context_node.name.as_str() {
            "title" | "textarea" => State::RCDATA,
            "style" | "xmp" | "iframe" | "noembed" | "noframes" => State::RAWTEXT,
            "script" => State::ScriptData,
            "noscript" => {
                if self.scripting_enabled {
                    State::RAWTEXT
                } else {
                    State::Data
                }
            }
            "plaintext" => State::PLAINTEXT,
            _ => State::Data,
        }
    }

    // Initialize all parser settings for parsing a fragment case
    fn initialize_fragment_case(&mut self, context_node: &Node) {
        self.is_fragment_case = true;
        self.context_doc = Some(Document::clone(&context_node.document));
        self.context_node_id = Some(context_node.id);
        self.tokenizer
            .set_state(self.find_initial_state_for_context(context_node));
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::html5::input_stream::Encoding;
    use crate::html5::parser::document::DocumentBuilder;

    macro_rules! node_create {
        ($self:expr, $name:expr) => {{
            let node = Node::new_element(&$self.document, $name, HashMap::new(), HTML_NAMESPACE);
            let node_id = $self
                .document
                .get_mut()
                .add_node(node, NodeId::root(), None);
            $self.open_elements.push(node_id);
        }};
    }

    #[test]
    fn is_in_scope() {
        let stream = &mut InputStream::new();
        let mut parser = Html5Parser::new_parser(stream);

        node_create!(parser, "html");
        node_create!(parser, "div");
        node_create!(parser, "p");
        node_create!(parser, "button");
        assert!(parser.is_in_scope("p", Scope::Regular));
        assert!(!parser.is_in_scope("p", Scope::Button));
        assert!(parser.is_in_scope("p", Scope::ListItem));
        assert!(!parser.is_in_scope("p", Scope::Select));
    }

    #[test]
    fn is_in_scope_empty_stack() {
        let stream = &mut InputStream::new();
        let mut parser = Html5Parser::new_parser(stream);

        parser.open_elements.clear();
        assert!(!parser.is_in_scope("p", Scope::Regular));
        assert!(!parser.is_in_scope("p", Scope::Button));
        assert!(!parser.is_in_scope("p", Scope::ListItem));
        assert!(!parser.is_in_scope("p", Scope::Select));
    }

    #[test]
    fn is_in_scope_non_existing_node() {
        let stream = &mut InputStream::new();
        let mut parser = Html5Parser::new_parser(stream);

        node_create!(parser, "html");
        node_create!(parser, "div");
        node_create!(parser, "p");
        node_create!(parser, "button");

        assert!(!parser.is_in_scope("foo", Scope::Regular));
        assert!(!parser.is_in_scope("foo", Scope::Button));
        assert!(!parser.is_in_scope("foo", Scope::ListItem));
        assert!(!parser.is_in_scope("foo", Scope::Select));
    }

    #[test]
    fn is_in_scope_1() {
        let stream = &mut InputStream::new();
        let mut parser = Html5Parser::new_parser(stream);

        node_create!(parser, "html");
        node_create!(parser, "div");
        node_create!(parser, "table");
        node_create!(parser, "tr");
        node_create!(parser, "td");
        node_create!(parser, "p");
        node_create!(parser, "span");

        assert!(parser.is_in_scope("p", Scope::Regular));
        assert!(parser.is_in_scope("p", Scope::ListItem));
        assert!(parser.is_in_scope("p", Scope::Button));
        assert!(parser.is_in_scope("p", Scope::Table));
        assert!(!parser.is_in_scope("p", Scope::Select));

        assert!(!parser.is_in_scope("div", Scope::Regular));
        assert!(!parser.is_in_scope("div", Scope::ListItem));
        assert!(!parser.is_in_scope("div", Scope::Button));
        assert!(!parser.is_in_scope("div", Scope::Table));
        assert!(!parser.is_in_scope("div", Scope::Select));

        assert!(!parser.is_in_scope("tr", Scope::Regular));
        assert!(!parser.is_in_scope("tr", Scope::ListItem));
        assert!(!parser.is_in_scope("tr", Scope::Button));
        assert!(parser.is_in_scope("tr", Scope::Table));
        assert!(!parser.is_in_scope("tr", Scope::Select));

        assert!(!parser.is_in_scope("xmp", Scope::Regular));
        assert!(!parser.is_in_scope("xmp", Scope::ListItem));
        assert!(!parser.is_in_scope("xmp", Scope::Button));
        assert!(!parser.is_in_scope("xmp", Scope::Table));
        assert!(!parser.is_in_scope("xmp", Scope::Select));
    }

    #[test]
    fn is_in_scope_2() {
        let stream = &mut InputStream::new();
        let mut parser = Html5Parser::new_parser(stream);

        node_create!(parser, "html");
        node_create!(parser, "body");
        node_create!(parser, "ul");
        node_create!(parser, "li");
        node_create!(parser, "div");
        node_create!(parser, "button");

        assert!(parser.is_in_scope("li", Scope::Regular));
        assert!(parser.is_in_scope("li", Scope::ListItem));
        assert!(!parser.is_in_scope("li", Scope::Button));
        assert!(parser.is_in_scope("li", Scope::Table));
        assert!(!parser.is_in_scope("li", Scope::Select));
    }

    #[test]
    fn is_in_scope_3() {
        let stream = &mut InputStream::new();
        let mut parser = Html5Parser::new_parser(stream);

        node_create!(parser, "html");
        node_create!(parser, "body");
        node_create!(parser, "div");
        node_create!(parser, "ul");
        node_create!(parser, "li");
        node_create!(parser, "p");

        assert!(parser.is_in_scope("li", Scope::Regular));
        assert!(parser.is_in_scope("li", Scope::ListItem));
        assert!(parser.is_in_scope("li", Scope::Button));
        assert!(parser.is_in_scope("li", Scope::Table));
        assert!(!parser.is_in_scope("li", Scope::Select));
    }

    #[test]
    fn is_in_scope_4() {
        let stream = &mut InputStream::new();
        let mut parser = Html5Parser::new_parser(stream);

        node_create!(parser, "html");
        node_create!(parser, "body");
        node_create!(parser, "table");
        node_create!(parser, "tbody");
        node_create!(parser, "tr");
        node_create!(parser, "td");
        node_create!(parser, "button");
        node_create!(parser, "span");

        assert!(parser.is_in_scope("td", Scope::Regular));
        assert!(parser.is_in_scope("td", Scope::ListItem));
        assert!(!parser.is_in_scope("td", Scope::Button));
        assert!(parser.is_in_scope("td", Scope::Table));
        assert!(!parser.is_in_scope("td", Scope::Select));
    }

    #[test]
    fn is_in_scope_5() {
        let stream = &mut InputStream::new();
        let mut parser = Html5Parser::new_parser(stream);

        node_create!(parser, "html");
        node_create!(parser, "body");
        node_create!(parser, "div");
        node_create!(parser, "object");
        node_create!(parser, "p");
        node_create!(parser, "a");
        node_create!(parser, "span");

        assert!(!parser.is_in_scope("div", Scope::Regular));
        assert!(!parser.is_in_scope("div", Scope::ListItem));
        assert!(!parser.is_in_scope("div", Scope::Button));
        assert!(parser.is_in_scope("div", Scope::Table));
        assert!(!parser.is_in_scope("div", Scope::Select));
    }

    #[test]
    fn is_in_scope_6() {
        let stream = &mut InputStream::new();
        let mut parser = Html5Parser::new_parser(stream);

        node_create!(parser, "html");
        node_create!(parser, "body");
        node_create!(parser, "div");
        node_create!(parser, "ul");
        node_create!(parser, "li");
        node_create!(parser, "marquee");
        node_create!(parser, "p");

        assert!(!parser.is_in_scope("ul", Scope::Regular));
        assert!(!parser.is_in_scope("ul", Scope::ListItem));
        assert!(!parser.is_in_scope("ul", Scope::Button));
        assert!(parser.is_in_scope("ul", Scope::Table));
        assert!(!parser.is_in_scope("ul", Scope::Select));
    }

    #[test]
    fn is_in_scope_7() {
        let stream = &mut InputStream::new();
        let mut parser = Html5Parser::new_parser(stream);

        node_create!(parser, "html");
        node_create!(parser, "body");
        node_create!(parser, "div");
        node_create!(parser, "table");
        node_create!(parser, "caption");
        node_create!(parser, "p");

        assert!(!parser.is_in_scope("table", Scope::Regular));
        assert!(!parser.is_in_scope("table", Scope::ListItem));
        assert!(!parser.is_in_scope("table", Scope::Button));
        assert!(parser.is_in_scope("table", Scope::Table));
        assert!(!parser.is_in_scope("table", Scope::Select));
    }

    #[test]
    fn is_in_scope_8() {
        let stream = &mut InputStream::new();
        let mut parser = Html5Parser::new_parser(stream);

        node_create!(parser, "html");
        node_create!(parser, "body");
        node_create!(parser, "select");
        node_create!(parser, "optgroup");
        node_create!(parser, "option");

        assert!(parser.is_in_scope("select", Scope::Regular));
        assert!(parser.is_in_scope("select", Scope::ListItem));
        assert!(parser.is_in_scope("select", Scope::Button));
        assert!(parser.is_in_scope("select", Scope::Table));
        assert!(parser.is_in_scope("select", Scope::Select));
    }

    #[test]
    fn reconstruct_formatting() {
        let mut stream = InputStream::new();
        stream.read_from_str(
            "<p><b>bold<i>bold and italic</b>italic</i></p>",
            Some(Encoding::UTF8),
        );

        let document = DocumentBuilder::new_document();
        let _ = Html5Parser::parse_document(&mut stream, Document::clone(&document), None);

        println!("{}", document);
    }

    #[test]
    fn element_with_classes() {
        let mut stream = InputStream::new();
        stream.read_from_str("<div class=\"one two three\"></div>", Some(Encoding::UTF8));

        let document = DocumentBuilder::new_document();
        let _ = Html5Parser::parse_document(&mut stream, Document::clone(&document), None);

        let binding = document.get();

        // document -> html -> head -> body -> div
        let div = binding.get_node_by_id(4.into()).unwrap();

        let NodeData::Element(element) = &div.data else {
            panic!()
        };

        assert_eq!(element.classes.len(), 3);

        assert!(element.classes.contains("one"));
        assert!(element.classes.contains("two"));
        assert!(element.classes.contains("three"));

        assert!(element.classes.is_active("one"));
        assert!(element.classes.is_active("two"));
        assert!(element.classes.is_active("three"));
    }

    #[test]
    fn element_with_classes_extra_whitespace() {
        let mut stream = InputStream::new();
        stream.read_from_str(
            "<div class=\" one    two     three   \"></div>",
            Some(Encoding::UTF8),
        );

        let document = DocumentBuilder::new_document();
        let _ = Html5Parser::parse_document(&mut stream, Document::clone(&document), None);

        let binding = document.get();

        // document -> html -> head -> body -> div
        let div = binding.get_node_by_id(4.into()).unwrap();

        let NodeData::Element(element) = &div.data else {
            panic!()
        };

        assert_eq!(element.classes.len(), 3);

        assert!(element.classes.contains("one"));
        assert!(element.classes.contains("two"));
        assert!(element.classes.contains("three"));

        assert!(element.classes.is_active("one"));
        assert!(element.classes.is_active("two"));
        assert!(element.classes.is_active("three"));
    }

    #[test]
    fn element_with_invalid_named_id() {
        let mut stream = InputStream::new();
        stream.read_from_str(
            "<div id=\"my id\"></div> \
             <div id=\"123\"></div> \
             <div id=\"\"></div>",
            Some(Encoding::UTF8),
        );

        let document = DocumentBuilder::new_document();
        let _ = Html5Parser::parse_document(&mut stream, Document::clone(&document), None);

        assert!(document.get().get_node_by_named_id("my id").is_none());
        assert!(document.get().get_node_by_named_id("123").is_none());
        assert!(document.get().get_node_by_named_id("").is_none());
    }

    #[test]
    fn element_with_named_id() {
        let mut stream = InputStream::new();
        stream.read_from_str(
            "<div id=\"myid\"></div> \
             <p id=\"myid\"></p>",
            Some(Encoding::UTF8),
        );

        let document = DocumentBuilder::new_document();
        let _ = Html5Parser::parse_document(&mut stream, Document::clone(&document), None);

        // we are expecting the div (ID: 4) and p would be ignored
        let doc_read = document.get();
        let div = doc_read.get_node_by_named_id("myid").unwrap();
        assert_eq!(div.id, NodeId::from(4));
        assert_eq!(div.name, "div");
    }
}
