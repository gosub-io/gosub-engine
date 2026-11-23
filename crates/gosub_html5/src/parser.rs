use core::cell::{Cell, RefCell};
use core::option::Option::Some;
#[cfg(all(feature = "debug_parser", test))]
use gosub_interface::node::NodeType;
use gosub_shared::attributes::Attributes;
use std::borrow::Cow;
#[cfg(all(feature = "debug_parser", test))]
use std::io::Write;
use std::rc::Rc;
use std::sync::Arc;

use crate::node::{HTML_NAMESPACE, MATHML_NAMESPACE, SVG_NAMESPACE};
use crate::parser::errors::ErrorLogger;
use crate::parser::helper::{is_html_integration_point, is_mathml_text_integration_point};
use crate::parser::open_elements::OpenElements;
use crate::tokenizer::state::State;
use crate::tokenizer::token::Token;
use crate::tokenizer::{ParserData, Tokenizer};
use cow_utils::CowUtils;
use gosub_interface::config::HasDocument;
use gosub_interface::document::{Document, DocumentType};
use gosub_interface::node::{ShadowRootInit, ShadowRootMode, SlotAssignmentMode};

use gosub_interface::html5::ParserOptions;
use gosub_interface::node::QuirksMode;
use gosub_shared::byte_stream::{ByteStream, Encoding, Location};
use gosub_shared::node::NodeId;
use gosub_shared::timing::{self, Timer};
use gosub_shared::types::{ParseError, Result};

mod attr_replacements;
pub mod errors;
mod open_elements;
mod quirks;
#[cfg(test)]
mod tree_construction_tests;

// ------------------------------------------------------------

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

/// Returns the NodeId of the current node (top of open_elements stack).
macro_rules! current_node_id {
    ($self:expr) => {{
        *$self.open_elements.last().unwrap_or(&NodeId::root())
    }};
}

/// Returns the tag name of a given NodeId in self.document (empty string if not an element).
macro_rules! node_tag {
    ($self:expr, $id:expr) => {{
        $self.document.tag_name($id).unwrap_or_default()
    }};
}

/// Returns the namespace of a given NodeId in self.document (None if not an element).
macro_rules! node_ns {
    ($self:expr, $id:expr) => {{
        $self.document.namespace($id)
    }};
}

#[macro_use]
mod helper;
mod foreign;
mod formatting;
mod fragment;
mod modes;
mod scope;
mod stylesheets;

use formatting::ActiveElement;
use scope::Scope;
pub use stylesheets::StylesheetSource;

pub struct Html5ParserOptions {
    pub scripting_enabled: bool,
    /// Where the bytes of a linked stylesheet come from when a script has to wait for one.
    ///
    /// `None` means nothing waits: the sheets are resolved after the parse instead, which
    /// is right for a document with no scripts in it and for every test in this crate.
    pub stylesheets: Option<Arc<dyn StylesheetSource>>,
}

impl ParserOptions for Html5ParserOptions {
    fn new(scripting: bool) -> Self {
        Self {
            scripting_enabled: scripting,
            stylesheets: None,
        }
    }
}

impl Default for Html5ParserOptions {
    fn default() -> Self {
        Self {
            scripting_enabled: true,
            stylesheets: None,
        }
    }
}

/// The main parser object
pub struct Html5Parser<'tokens, C: HasDocument> {
    /// tokenizer object
    tokenizer: Tokenizer<'tokens>,
    /// current insertion mode
    insertion_mode: InsertionMode,
    /// original insertion mode (used for text mode)
    original_insertion_mode: InsertionMode,
    /// template insertion mode stack
    template_insertion_mode: Vec<InsertionMode>,
    /// The spec's "parser cannot change the mode" flag: when set, a DOCTYPE leaves the
    /// quirks mode alone.
    parser_cannot_change_mode: bool,
    /// Current token from the tokenizer
    current_token: Token,
    /// If true, the current token should be processed again
    reprocess_token: bool,
    /// True when an insertion-mode handler replaced `current_token` in place (e.g. the
    /// `<image>` -> `<img>` rewrite). Signals that the replacement, not the token moved
    /// out for matching, must survive into a reprocess. See `process_html_content`.
    current_token_rewritten: bool,
    /// Stack of open elements
    open_elements: OpenElements,
    /// Current head element
    head_element: Option<NodeId>,
    /// Current form element
    form_element: Option<NodeId>,
    /// If true, scripting is enabled
    scripting_enabled: bool,
    /// Where a blocking script's stylesheets come from; see [`StylesheetSource`].
    stylesheet_source: Option<Arc<dyn StylesheetSource>>,
    /// if true, we can insert a frameset
    frameset_ok: bool,
    /// Foster parenting flag
    foster_parenting: bool,
    /// If true, the script engine has already started. This and `script_nesting_level`,
    /// `parser_pause_flag` and `insertion_point` are kept for script execution and
    /// `document.write`; they are set but not read yet.
    script_already_started: bool,
    /// Microseconds this parse spent waiting for stylesheets, at a blocking script or at an
    /// `@import`. Subtracted from `decode.html`, which measures bytes to DOM only. A `Cell`
    /// because an import is resolved from `&self`.
    blocked_on_css_us: Cell<u64>,
    /// Whether the `<script>` currently being read blocks: an inline one always does, and
    /// one with a `src` does unless it is marked `async` or `defer`. Read at the start tag,
    /// where the attributes are, and acted on at the end tag, where the script would run.
    current_script_blocks: bool,
    /// Pending table character tokens
    pending_table_character_tokens: String,
    /// Location of the first pending table character token
    pending_table_character_location: Location,
    /// Acknowledge self-closing tags
    ack_self_closing: bool,
    /// List of active formatting elements or markers
    active_formatting_elements: Vec<ActiveElement>,
    /// Is the current parsing a fragment case. If so, the `context_node_id` and `context_doc` should be set as well.
    is_fragment_case: bool,
    /// A reference to the document we are parsing
    document: &'tokens mut C::Document,
    /// Error logger, which is shared with the tokenizer
    error_logger: Rc<RefCell<ErrorLogger>>,
    /// Levels of scripting we currently are in
    script_nesting_level: u32,
    /// If true, the parser is paused
    parser_pause_flag: bool,
    /// Keeps the position of where any `document.write()` should be inserted when running a script
    insertion_point: Option<usize>,
    /// Ignore when next token is LF
    ignore_lf: bool,
    /// When true, the parser is finished and should not consume more tokens (there aren't any)
    parser_finished: bool,
    /// Context node id for fragment parsing
    context_node_id: Option<NodeId>,
    /// The document's "allow declarative shadow roots" flag. True when parsing a document, so
    /// `<template shadowrootmode>` attaches a shadow root; false for fragment parsing, which is
    /// what `innerHTML` uses - there, the same markup must stay an inert template.
    /// (`setHTMLUnsafe()`/`parseHTMLUnsafe()` would set it true, but neither exists yet.)
    allow_declarative_shadow_roots: bool,
}

impl<C: HasDocument> gosub_interface::html5::Html5Parser<C> for Html5Parser<'_, C> {
    type Options = Html5ParserOptions;

    fn parse(stream: &mut ByteStream, doc: &mut C::Document, opts: Option<Self::Options>) -> Result<Vec<ParseError>> {
        Self::parse_document(stream, doc, opts)
    }

    fn parse_fragment(
        stream: &mut ByteStream,
        doc: &mut C::Document,
        context_node: NodeId,
        options: Option<Self::Options>,
        start_location: Location,
    ) -> Result<Vec<ParseError>> {
        Self::parse_fragment(stream, doc, context_node, options, start_location)
    }
}

/// Defines the mode we should dispatch
enum DispatcherMode {
    Foreign,
    Html,
}

/// Where foreign content hands the current token to HTML processing
enum Delegate {
    None,
    UnexpectedHtmlTag,
    HtmlContent,
}

/// Returns the location just past `text`, when `text` starts at `location`. Token text is
/// decoded, so after a character reference or a CRLF in `text` the result no longer matches
/// the source exactly.
fn location_after(location: Location, text: &str) -> Location {
    let mut location = location;
    for ch in text.chars() {
        if ch == '\n' {
            location.line += 1;
            location.column = 1;
        } else {
            location.column += 1;
        }
        location.offset += ch.len_utf8();
    }
    location
}

/// Extracts the encoding declared by a `<meta charset="...">` or
/// `<meta http-equiv="Content-Type" content="...; charset=...">` token.
/// Returns `None` when no valid/supported encoding is found.
fn meta_charset_encoding(token: &Token) -> Option<Encoding> {
    let Token::StartTag { attributes, .. } = token else {
        return None;
    };

    if let Some(charset) = attributes.get("charset") {
        return charset_label_to_encoding(charset.trim());
    }

    // http-equiv="Content-Type" with a content attribute
    if let Some(http_equiv) = attributes.get("http-equiv") {
        if http_equiv.eq_ignore_ascii_case("content-type") {
            if let Some(content) = attributes.get("content") {
                let charset = content.split(';').find_map(|part| {
                    let part = part.trim();
                    let (k, v) = part.split_once('=')?;
                    if !k.trim().eq_ignore_ascii_case("charset") {
                        return None;
                    }
                    Some(v.trim())
                })?;
                return charset_label_to_encoding(charset);
            }
        }
    }

    None
}

/// Maps a charset label string to our `Encoding` enum.
/// Per WHATWG spec section 13.2.3.5, a UTF-16 label from a meta element must be
/// treated as UTF-8 (a truly UTF-16 document would have a BOM detected earlier).
fn charset_label_to_encoding(label: &str) -> Option<Encoding> {
    match label.cow_to_ascii_lowercase().as_ref() {
        "utf-8" | "utf8" | "unicode-1-1-utf-8" => Some(Encoding::UTF8),
        "us-ascii" | "ascii" | "ansi_x3.4-1968" | "iso-ir-6" | "iso646-us" | "latin1" | "iso-8859-1" => {
            Some(Encoding::Latin1)
        }
        // UTF-16 from meta -> treat as UTF-8 per spec
        "utf-16" | "utf-16le" | "utf-16be" => Some(Encoding::UTF8),
        _ => None,
    }
}

impl<'a, C: HasDocument> Html5Parser<'a, C> {
    /// Initialize the parser and retain its selected scripting mode on the document for serialization.
    fn init(
        tokenizer: Tokenizer<'a>,
        document: &'a mut C::Document,
        error_logger: Rc<RefCell<ErrorLogger>>,
        options: Option<Html5ParserOptions>,
    ) -> Self {
        let scripting_enabled = options.as_ref().is_none_or(|o| o.scripting_enabled);
        document.set_scripting_enabled(scripting_enabled);
        Self {
            tokenizer,
            insertion_mode: InsertionMode::Initial,
            original_insertion_mode: InsertionMode::Initial,
            template_insertion_mode: vec![],
            parser_cannot_change_mode: false,
            current_token: Token::Eof {
                location: Location::default(),
            },
            reprocess_token: false,
            current_token_rewritten: false,
            open_elements: OpenElements::default(),
            head_element: None,
            form_element: None,
            scripting_enabled,
            stylesheet_source: options.and_then(|o| o.stylesheets),
            frameset_ok: true,
            foster_parenting: false,
            script_already_started: false,
            current_script_blocks: false,
            blocked_on_css_us: Cell::new(0),
            pending_table_character_tokens: String::new(),
            pending_table_character_location: Location::default(),
            ack_self_closing: false,
            active_formatting_elements: vec![],
            is_fragment_case: false,
            document,
            error_logger,
            script_nesting_level: 0,
            parser_pause_flag: false,
            insertion_point: None,
            ignore_lf: false,
            parser_finished: false,
            context_node_id: None,
            allow_declarative_shadow_roots: true,
        }
    }

    /// Create a parser for tests with scripting enabled on both the parser and the supplied document.
    /// Regular users should use the `parse_document()` and `parse_fragment()` functions instead.
    pub fn new_parser(stream: &'a mut ByteStream, document: &'a mut C::Document) -> Self {
        let error_logger = Rc::new(RefCell::new(ErrorLogger::new()));
        let tokenizer = Tokenizer::new(stream, None, error_logger.clone());
        document.set_scripting_enabled(true);

        Self {
            tokenizer,
            insertion_mode: InsertionMode::Initial,
            original_insertion_mode: InsertionMode::Initial,
            template_insertion_mode: vec![],
            parser_cannot_change_mode: false,
            current_token: Token::Eof {
                location: Location::default(),
            },
            reprocess_token: false,
            current_token_rewritten: false,
            open_elements: OpenElements::default(),
            head_element: None,
            form_element: None,
            scripting_enabled: true,
            stylesheet_source: None,
            frameset_ok: true,
            foster_parenting: false,
            script_already_started: false,
            current_script_blocks: false,
            blocked_on_css_us: Cell::new(0),
            pending_table_character_tokens: String::new(),
            pending_table_character_location: Location::default(),
            ack_self_closing: false,
            active_formatting_elements: vec![],
            is_fragment_case: false,
            document,
            error_logger,
            script_nesting_level: 0,
            parser_pause_flag: false,
            insertion_point: None,
            ignore_lf: false,
            parser_finished: false,
            context_node_id: None,
            allow_declarative_shadow_roots: true,
        }
    }

    /// Parses the input into `document` as a full document (html, head, body, ...). Returns the
    /// parse errors.
    pub fn parse_document(
        stream: &mut ByteStream,
        document: &mut C::Document,
        options: Option<Html5ParserOptions>,
    ) -> Result<Vec<ParseError>> {
        // Create a new error logger that will be used in both the tokenizer and the parser
        let error_logger = Rc::new(RefCell::new(ErrorLogger::new()));

        let context = document.url().map_or_else(|| "unknown".to_string(), |u| u.to_string());

        let tokenizer = Tokenizer::new(stream, None, error_logger.clone());
        let mut parser = Html5Parser::<C>::init(tokenizer, document, error_logger, options);

        // Bytes to DOM only. The time spent waiting for stylesheets at a blocking script is
        // recorded under `script.blocked_on_css` and taken back out here.
        let mut wall = Timer::new(Some(context.clone()));
        let ret = parser.do_parse();
        wall.end();

        let parse_us = wall.duration().saturating_sub(parser.blocked_on_css_us.get());
        timing::record(gosub_shared::timing::Timing::DecodeHtml, parse_us, Some(context));

        ret
    }

    /// Internal parser function that does the actual parsing
    fn do_parse(&mut self) -> Result<Vec<ParseError>> {
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

            // The handlers swap the token out for a placeholder, so the trace keeps its own copy.
            #[cfg(all(feature = "debug_parser", test))]
            let traced = self.current_token.clone();

            // Check how we should dispatch the token, and dispatch to the correct function
            match dispatcher_mode {
                DispatcherMode::Foreign => {
                    self.process_foreign_content();
                }
                DispatcherMode::Html => {
                    self.process_html_content();
                }
            }

            #[cfg(all(feature = "debug_parser", test))]
            self.display_debug_info(&traced);
        }

        Ok(self.error_logger.borrow().get_errors())
    }

    fn set_quirks_mode(&mut self, quirks_mode: QuirksMode) {
        self.document.set_quirks_mode(quirks_mode);
    }

    fn is_iframesrcdoc(&self) -> bool {
        self.document.doctype() == DocumentType::IframeSrcDoc
    }

    /// Update the parser and document scripting settings together, including `noscript` serialization.
    pub fn enabled_scripting(&mut self, enabled: bool) {
        self.scripting_enabled = enabled;
        self.document.set_scripting_enabled(enabled);
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

            let cn_id = current_node_id!(self);
            if node_tag!(self, cn_id) == name && node_ns!(self, cn_id) == Some(HTML_NAMESPACE) {
                self.open_elements.pop();
                break;
            }

            self.open_elements.pop();
        }
    }

    /// Pops the last element from the open elements until we reach any of the elements in $arr
    fn pop_until_any(&mut self, arr: &[&str]) {
        while !self.open_elements.is_empty() {
            let Some(node_id) = self.open_elements.pop() else {
                break;
            };
            let tag = self.document.tag_name(node_id).unwrap_or_default();
            if arr.contains(&tag) {
                break;
            }
        }
    }

    /// Remove the given `node_id` from the open elements stack. Will do nothing when the `node_id` is not found
    fn open_elements_remove(&mut self, target_node_id: NodeId) {
        self.open_elements.retain(|&node_id| node_id != target_node_id);
    }

    fn pop_check(&mut self, name: &str) {
        let Some(node_id) = self.open_elements.pop() else {
            return;
        };
        if self.document.tag_name(node_id).unwrap_or_default() != name {
            self.parse_error(&format!("{name} tag should be popped from open elements"));
        }
    }

    /// Returns true when the open elements have an HTML element named $name. The spec's "a
    /// template element" is an HTML one: an SVG `template` does not count.
    fn open_elements_has(&self, name: &str) -> bool {
        self.open_elements.iter().rev().any(|&node_id| {
            self.document.tag_name(node_id).unwrap_or_default() == name
                && self.document.namespace(node_id) == Some(HTML_NAMESPACE)
        })
    }

    /// Retrieves a list of all errors generated by the parser/tokenizer
    pub fn get_parse_errors(&self) -> Vec<ParseError> {
        self.error_logger.borrow().get_errors()
    }

    /// Emits a parse error if any open element (in the HTML namespace) is not in the set of
    /// elements that are allowed to remain open at body/EOF close time (spec section 13.2.6.4.7).
    fn check_open_elements_at_body_end(&self) {
        const ALLOWED: &[&str] = &[
            "dd", "dt", "li", "optgroup", "option", "p", "rb", "rp", "rt", "rtc", "tbody", "td", "tfoot", "th",
            "thead", "tr", "body", "html",
        ];
        for &node_id in &self.open_elements {
            let tag = self.document.tag_name(node_id).unwrap_or_default();
            let ns = self.document.namespace(node_id);
            if ns == Some(HTML_NAMESPACE) && !ALLOWED.contains(&tag) {
                self.parse_error("open element not allowed at end of body");
                return;
            }
        }
    }

    /// Send a parse error to the error logger
    fn parse_error(&self, message: &str) {
        self.error_logger
            .borrow_mut()
            .add_error(self.current_token.get_location(), message);
    }

    /// One parse error per NUL in `text`, a run of NULs that starts at `location`. The spec sees
    /// each NUL as its own character token.
    fn null_parse_errors(&self, text: &str, location: Location, message: &str) {
        let mut location = location;
        for _ in text.chars() {
            self.error_logger.borrow_mut().add_error(location, message);
            location = location_after(location, "\0");
        }
    }

    /// Create a new node that is not connected or attached to the document arena
    /// Create and register a new node in the document arena, returning its NodeId.
    fn create_node(&mut self, token: &Token, namespace: &str) -> NodeId {
        match token {
            Token::DocType {
                name,
                pub_identifier,
                sys_identifier,
                location,
                ..
            } => self.document.create_doctype(
                name.as_deref().unwrap_or_default(),
                pub_identifier.as_deref(),
                sys_identifier.as_deref(),
                *location,
            ),
            Token::StartTag {
                name,
                attributes,
                location,
                ..
            } => self
                .document
                .create_element(name, Some(namespace), attributes.clone(), *location),
            Token::EndTag { name, location, .. } => {
                self.document
                    .create_element(name, Some(namespace), Attributes::new(), *location)
            }
            Token::Comment {
                comment: value,
                location,
                ..
            } => self.document.create_comment(value, *location),
            Token::Text {
                text: value, location, ..
            } => self.document.create_text(value.as_str(), *location),
            Token::Eof { location } => self.document.create_text("", *location),
        }
    }

    /// Like `create_node`, but a start tag's attributes are moved into the element instead
    /// of copied. The token keeps its name and flags; its attributes are empty afterwards.
    fn create_element_node(&mut self, token: &mut Token, namespace: &str) -> NodeId {
        if let Token::StartTag {
            name,
            attributes,
            location,
            ..
        } = token
        {
            let attributes = std::mem::take(attributes);
            return self
                .document
                .create_element(name, Some(namespace), attributes, *location);
        }
        self.create_node(token, namespace)
    }

    /// This function will pop elements off the stack until it reaches the first element that matches
    /// our condition (which can be changed with the except and thoroughly parameters)
    fn generate_implied_end_tags(&mut self, except: Option<&str>, thoroughly: bool) {
        loop {
            if self.open_elements.is_empty() {
                return;
            }

            let cn_id = current_node_id!(self);
            let tag = node_tag!(self, cn_id);
            let is_html = node_ns!(self, cn_id) == Some(HTML_NAMESPACE);
            if let Some(except) = except {
                if except == tag && is_html {
                    return;
                }
            }
            if thoroughly {
                if !([
                    "caption", "colgroup", "tbody", "td", "tfoot", "th", "thead", "tr", "dd", "dt", "li", "option",
                    "optgroup", "p", "rb", "rp", "rt", "rtc",
                ]
                .contains(&tag)
                    && is_html)
                {
                    return;
                }
            } else if !(["dd", "dt", "li", "option", "optgroup", "p", "rb", "rp", "rt", "rtc"].contains(&tag)
                && is_html)
            {
                return;
            }

            self.open_elements.pop();
        }
    }

    fn stop_parsing(&mut self) {
        self.parser_finished = true;
    }

    /// Switch the parser and tokenizer to the RAWTEXT state
    fn parse_raw_data(&mut self, token: &mut Token) {
        self.insert_html_element(token);

        self.tokenizer.state = State::RAWTEXT;

        self.original_insertion_mode = self.insertion_mode;
        self.insertion_mode = InsertionMode::Text;
    }

    /// Switch the parser and tokenizer to the RCDATA state
    fn parse_rcdata(&mut self, token: &mut Token) {
        self.insert_html_element(token);

        self.tokenizer.state = State::RCDATA;

        self.original_insertion_mode = self.insertion_mode;
        self.insertion_mode = InsertionMode::Text;
    }

    #[cfg(all(feature = "debug_parser", test))]
    fn display_debug_info(&self, token: &Token) {
        println!("-----------------------------------------\n");
        println!("current token   : '{token}'");
        println!("insertion mode  : {:?}", self.insertion_mode);
        print!("Open elements   : [ ");
        for node_id in &self.open_elements {
            if self.document.node_type(*node_id) == NodeType::ElementNode {
                print!(
                    "({}) {}, ",
                    node_id,
                    self.document.tag_name(*node_id).unwrap_or_default()
                );
            } else {
                print!("({}), ", node_id);
            }
        }
        println!("]");

        print!("Active elements : [");
        for elem in &self.active_formatting_elements {
            match elem {
                ActiveElement::Node(node_id) => {
                    if self.document.node_type(*node_id) == NodeType::ElementNode {
                        print!(
                            "({}) {}, ",
                            node_id,
                            self.document.tag_name(*node_id).unwrap_or_default()
                        );
                    } else {
                        print!("({}), ", node_id);
                    }
                }
                ActiveElement::Marker => {
                    print!("marker, ");
                }
            }
        }
        println!("]");

        std::io::stdout().flush().ok();
    }

    fn parser_data(&self) -> ParserData {
        if self.open_elements.is_empty() {
            return ParserData::default();
        }

        let acn_id = self.get_adjusted_current_node_id();
        let namespace = match self.document.namespace(acn_id).unwrap_or(HTML_NAMESPACE) {
            HTML_NAMESPACE => Cow::Borrowed(HTML_NAMESPACE),
            MATHML_NAMESPACE => Cow::Borrowed(MATHML_NAMESPACE),
            SVG_NAMESPACE => Cow::Borrowed(SVG_NAMESPACE),
            other => Cow::Owned(other.to_string()),
        };
        ParserData {
            adjusted_node_namespace: namespace,
        }
    }

    /// Fetches the next token from the tokenizer. A tokenizer error ends the input with an EOF
    /// token.
    fn fetch_next_token(&mut self) -> Token {
        match self.tokenizer.next_token(self.parser_data()) {
            Ok(token) => token,
            Err(_) => Token::Eof {
                location: Location::default(),
            },
        }
    }

    /// The declarative-shadow-root branch of the "in head" `<template>` rules (HTML 13.2.6.4.4).
    ///
    /// Returns the shadow root when the token declares one and its intended parent can host it -
    /// in which case the caller must *not* insert the template element into the tree. Every
    /// failed check falls through to `None`, i.e. to ordinary inert-template behavior; that
    /// includes the spec's "catch the `NotSupportedError`" recovery for a parent that is not a
    /// valid shadow host or already has a shadow root.
    fn attach_declarative_shadow_root(&mut self, attributes: &Attributes, location: Location) -> Option<NodeId> {
        if !self.allow_declarative_shadow_roots {
            return None;
        }
        let mode = ShadowRootMode::from_attribute(attributes.get("shadowrootmode")?)?;

        // The host is the adjusted current node, but never the topmost element on the stack of
        // open elements - that would hang a shadow root off <html>.
        let host = self.get_adjusted_current_node_id();
        if self.open_elements.first() == Some(&host) {
            return None;
        }

        let init = ShadowRootInit {
            mode,
            // The rest are boolean attributes: present at all means set.
            delegates_focus: attributes.contains_key("shadowrootdelegatesfocus"),
            clonable: attributes.contains_key("shadowrootclonable"),
            serializable: attributes.contains_key("shadowrootserializable"),
            // A declarative shadow root is always named; only `attachShadow()` can ask for
            // manual assignment, and that does not exist yet.
            slot_assignment: SlotAssignmentMode::Named,
        };

        self.document.attach_shadow_root(host, init, location)
    }

    /// Returns the NodeId of the adjusted current node.
    fn get_adjusted_current_node_id(&self) -> NodeId {
        // fragment case: return the context node (13.2.4), which fragment parsing always
        // sets. Falling through to the current node when it is somehow absent gives the same
        // answer everywhere it was set, and an answer rather than an abort where it was not.
        if self.is_fragment_case && self.open_elements.len() == 1 {
            if let Some(context_node_id) = self.context_node_id {
                return context_node_id;
            }
        }
        current_node_id!(self)
    }

    /// Checks the current token, node and parser context to see if the parser needs to switch to
    /// the foreign content or html content mode.
    fn select_dispatch_mode(&self) -> DispatcherMode {
        if self.open_elements.is_empty() {
            return DispatcherMode::Html;
        }

        let acn_id = self.get_adjusted_current_node_id();
        let acn_ns = self.document.namespace(acn_id).unwrap_or(HTML_NAMESPACE);

        if acn_ns == HTML_NAMESPACE {
            return DispatcherMode::Html;
        }

        if is_mathml_text_integration_point::<C>(self.document, acn_id)
            && self.current_token.is_any_start_tag()
            && (!self.current_token.is_start_tag("mglyph") && !self.current_token.is_start_tag("malignmark"))
        {
            return DispatcherMode::Html;
        }

        if is_mathml_text_integration_point::<C>(self.document, acn_id) && self.current_token.is_text_token() {
            return DispatcherMode::Html;
        }

        if acn_ns == MATHML_NAMESPACE
            && self.document.tag_name(acn_id).unwrap_or_default() == "annotation-xml"
            && self.current_token.is_start_tag("svg")
        {
            return DispatcherMode::Html;
        }

        if is_html_integration_point::<C>(self.document, acn_id) && self.current_token.is_any_start_tag() {
            return DispatcherMode::Html;
        }

        if is_html_integration_point::<C>(self.document, acn_id) && self.current_token.is_text_token() {
            return DispatcherMode::Html;
        }

        if self.current_token.is_eof() {
            return DispatcherMode::Html;
        }

        DispatcherMode::Foreign
    }

    /// Splits a regular text token with mixed characters into tokens of 3 groups:
    /// null-characters, (ascii) whitespaces, and regular (rest) characters.
    /// These tokens are then inserted into the token buffer queue, so they can get parsed
    /// correctly.
    ///
    /// example:
    ///
    ///   `Token::Text`("  foo bar\0  ")
    ///
    /// is split into 6 tokens:
    ///
    ///   `Token::Text`("  ")  // whitespace
    ///   `Token::Text("foo`") // regular
    ///   `Token::Text`(" ")   // whitespace
    ///   `Token::Text("bar`") // regular
    ///   `Token::Text("\0`")  // null
    ///   `Token::Text`("  ")  // whitespace
    ///
    /// This is needed because the tokenizer does not know about the context of the text it is,
    /// so it will always try to tokenize as greedy as possible. But sometimes we need this split
    /// to happen where a differentation between whitespaces, null and regular characters are needed.
    /// Only in those cases, this function is called, and the token will be split into multiple
    /// tokens.
    /// The idea is that large blobs of javascript for instance will not be split into separate
    /// tokens, but still be seen and parsed as a single `TextToken`.
    ///
    fn split_mixed_token(&self, text: &str, location: Location) -> Vec<Token> {
        self.split_token_by_group(text, location, |ch| {
            if ch == '\0' {
                '0'
            } else if ch.is_ascii_whitespace() {
                'w'
            } else {
                'r'
            }
        })
    }

    /// This will split tokens into \0 groups and non-\0 groups.
    fn split_mixed_token_null(&self, text: &str, location: Location) -> Vec<Token> {
        self.split_token_by_group(text, location, |ch| if ch == '\0' { '0' } else { 'r' })
    }

    /// Splits `text` into consecutive runs of characters that map to the same group, emitting
    /// one `Token::Text` per run. `group_of` classifies each character into a group key; a
    /// change in key starts a new token. This lets callers pick which distinctions matter
    /// (e.g. whitespace vs null vs regular) without re-splitting large blobs unnecessarily.
    /// `location` is where `text` starts; each token gets the location of its first character.
    fn split_token_by_group(&self, text: &str, location: Location, group_of: impl Fn(char) -> char) -> Vec<Token> {
        let mut tokens = vec![];
        let mut last_group = 'x';

        let mut found = String::new();
        let mut location = location;

        for ch in text.chars() {
            let group = group_of(ch);

            if last_group != group && !found.is_empty() {
                let next_location = location_after(location, &found);
                tokens.push(Token::Text {
                    text: std::mem::take(&mut found),
                    location,
                });
                location = next_location;
            }

            found.push(ch);
            last_group = group;
        }

        if !found.is_empty() {
            tokens.push(Token::Text { text: found, location });
        }

        tokens
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::document::builder::DocumentBuilderImpl;
    use crate::document::document_impl::DocumentImpl;
    use crate::node::node_impl::NodeDataTypeInternal;
    use gosub_css3::system::Css3System;
    use gosub_interface::config::ModuleConfiguration;
    use gosub_shared::byte_stream::{Confidence, Encoding};

    #[derive(Clone, Debug, PartialEq)]
    struct Config;

    impl ModuleConfiguration for Config {
        type CssSystem = Css3System;
        type Document = DocumentImpl<Self>;
        type HtmlParser = Html5Parser<'static, Self>;
    }

    type Parser<'a> = Html5Parser<'a, Config>;

    macro_rules! node_create {
        ($self:expr, $name:expr) => {{
            let node_id =
                $self
                    .document
                    .create_element($name, Some(HTML_NAMESPACE), Attributes::new(), Location::default());
            $self.document.attach(node_id, NodeId::root(), None).unwrap();
            $self.open_elements.push(node_id);
        }};
    }

    #[test]
    fn is_in_scope() {
        let mut stream = ByteStream::new(Encoding::UTF8, None);
        let mut document = DocumentBuilderImpl::new_document::<Config>(None);
        let mut parser = Parser::new_parser(&mut stream, &mut document);

        node_create!(parser, "html");
        node_create!(parser, "div");
        node_create!(parser, "p");
        node_create!(parser, "button");
        assert!(parser.is_in_scope("p", HTML_NAMESPACE, Scope::Regular));
        assert!(!parser.is_in_scope("p", HTML_NAMESPACE, Scope::Button));
        assert!(parser.is_in_scope("p", HTML_NAMESPACE, Scope::ListItem));
        assert!(!parser.is_in_scope("p", HTML_NAMESPACE, Scope::Select));
    }

    #[test]
    fn split_mixed_token_locations() {
        let mut stream = ByteStream::new(Encoding::UTF8, None);
        let mut document = DocumentBuilderImpl::new_document::<Config>(None);
        let parser = Parser::new_parser(&mut stream, &mut document);

        let locations = |tokens: Vec<Token>| {
            tokens
                .iter()
                .map(|t| {
                    let l = t.get_location();
                    (t.to_string(), l.line, l.column, l.offset)
                })
                .collect::<Vec<_>>()
        };
        let piece = |text: &str, line, column, offset| (text.to_string(), line, column, offset);

        assert_eq!(
            locations(parser.split_mixed_token(" \u{e9}\n\0x", Location::new(3, 5, 20))),
            vec![
                piece(" ", 3, 5, 20),
                piece("\u{e9}", 3, 6, 21),
                piece("\n", 3, 7, 23),
                piece("\0", 4, 1, 24),
                piece("x", 4, 2, 25),
            ]
        );
        assert_eq!(
            locations(parser.split_mixed_token_null("ab\0\0c", Location::new(1, 4, 3))),
            vec![piece("ab", 1, 4, 3), piece("\0\0", 1, 6, 5), piece("c", 1, 8, 7)]
        );
    }

    #[test]
    fn is_in_scope_empty_stack() {
        let mut stream = ByteStream::new(Encoding::UTF8, None);
        let mut document = DocumentBuilderImpl::new_document::<Config>(None);
        let mut parser = Parser::new_parser(&mut stream, &mut document);

        parser.open_elements.clear();
        assert!(!parser.is_in_scope("p", HTML_NAMESPACE, Scope::Regular));
        assert!(!parser.is_in_scope("p", HTML_NAMESPACE, Scope::Button));
        assert!(!parser.is_in_scope("p", HTML_NAMESPACE, Scope::ListItem));
        assert!(!parser.is_in_scope("p", HTML_NAMESPACE, Scope::Select));
    }

    #[test]
    fn is_in_scope_non_existing_node() {
        let mut stream = ByteStream::new(Encoding::UTF8, None);
        let mut document = DocumentBuilderImpl::new_document::<Config>(None);
        let mut parser = Parser::new_parser(&mut stream, &mut document);
        node_create!(parser, "html");
        node_create!(parser, "div");
        node_create!(parser, "p");
        node_create!(parser, "button");

        assert!(!parser.is_in_scope("foo", HTML_NAMESPACE, Scope::Regular));
        assert!(!parser.is_in_scope("foo", HTML_NAMESPACE, Scope::Button));
        assert!(!parser.is_in_scope("foo", HTML_NAMESPACE, Scope::ListItem));
        assert!(!parser.is_in_scope("foo", HTML_NAMESPACE, Scope::Select));
    }

    #[test]
    fn is_in_scope_1() {
        let mut stream = ByteStream::new(Encoding::UTF8, None);
        let mut document = DocumentBuilderImpl::new_document::<Config>(None);
        let mut parser = Parser::new_parser(&mut stream, &mut document);
        node_create!(parser, "html");
        node_create!(parser, "div");
        node_create!(parser, "table");
        node_create!(parser, "tr");
        node_create!(parser, "td");
        node_create!(parser, "p");
        node_create!(parser, "span");

        assert!(parser.is_in_scope("p", HTML_NAMESPACE, Scope::Regular));
        assert!(parser.is_in_scope("p", HTML_NAMESPACE, Scope::ListItem));
        assert!(parser.is_in_scope("p", HTML_NAMESPACE, Scope::Button));
        assert!(parser.is_in_scope("p", HTML_NAMESPACE, Scope::Table));
        assert!(!parser.is_in_scope("p", HTML_NAMESPACE, Scope::Select));

        assert!(!parser.is_in_scope("div", HTML_NAMESPACE, Scope::Regular));
        assert!(!parser.is_in_scope("div", HTML_NAMESPACE, Scope::ListItem));
        assert!(!parser.is_in_scope("div", HTML_NAMESPACE, Scope::Button));
        assert!(!parser.is_in_scope("div", HTML_NAMESPACE, Scope::Table));
        assert!(!parser.is_in_scope("div", HTML_NAMESPACE, Scope::Select));

        assert!(!parser.is_in_scope("tr", HTML_NAMESPACE, Scope::Regular));
        assert!(!parser.is_in_scope("tr", HTML_NAMESPACE, Scope::ListItem));
        assert!(!parser.is_in_scope("tr", HTML_NAMESPACE, Scope::Button));
        assert!(parser.is_in_scope("tr", HTML_NAMESPACE, Scope::Table));
        assert!(!parser.is_in_scope("tr", HTML_NAMESPACE, Scope::Select));

        assert!(!parser.is_in_scope("xmp", HTML_NAMESPACE, Scope::Regular));
        assert!(!parser.is_in_scope("xmp", HTML_NAMESPACE, Scope::ListItem));
        assert!(!parser.is_in_scope("xmp", HTML_NAMESPACE, Scope::Button));
        assert!(!parser.is_in_scope("xmp", HTML_NAMESPACE, Scope::Table));
        assert!(!parser.is_in_scope("xmp", HTML_NAMESPACE, Scope::Select));
    }

    #[test]
    fn is_in_scope_2() {
        let mut stream = ByteStream::new(Encoding::UTF8, None);
        let mut document = DocumentBuilderImpl::new_document::<Config>(None);
        let mut parser = Parser::new_parser(&mut stream, &mut document);
        node_create!(parser, "html");
        node_create!(parser, "body");
        node_create!(parser, "ul");
        node_create!(parser, "li");
        node_create!(parser, "div");
        node_create!(parser, "button");

        assert!(parser.is_in_scope("li", HTML_NAMESPACE, Scope::Regular));
        assert!(parser.is_in_scope("li", HTML_NAMESPACE, Scope::ListItem));
        assert!(!parser.is_in_scope("li", HTML_NAMESPACE, Scope::Button));
        assert!(parser.is_in_scope("li", HTML_NAMESPACE, Scope::Table));
        assert!(!parser.is_in_scope("li", HTML_NAMESPACE, Scope::Select));
    }

    #[test]
    fn is_in_scope_3() {
        let mut stream = ByteStream::new(Encoding::UTF8, None);
        let mut document = DocumentBuilderImpl::new_document::<Config>(None);
        let mut parser = Parser::new_parser(&mut stream, &mut document);
        node_create!(parser, "html");
        node_create!(parser, "body");
        node_create!(parser, "div");
        node_create!(parser, "ul");
        node_create!(parser, "li");
        node_create!(parser, "p");

        assert!(parser.is_in_scope("li", HTML_NAMESPACE, Scope::Regular));
        assert!(parser.is_in_scope("li", HTML_NAMESPACE, Scope::ListItem));
        assert!(parser.is_in_scope("li", HTML_NAMESPACE, Scope::Button));
        assert!(parser.is_in_scope("li", HTML_NAMESPACE, Scope::Table));
        assert!(!parser.is_in_scope("li", HTML_NAMESPACE, Scope::Select));
    }

    #[test]
    fn is_in_scope_4() {
        let mut stream = ByteStream::new(Encoding::UTF8, None);
        let mut document = DocumentBuilderImpl::new_document::<Config>(None);
        let mut parser = Parser::new_parser(&mut stream, &mut document);
        node_create!(parser, "html");
        node_create!(parser, "body");
        node_create!(parser, "table");
        node_create!(parser, "tbody");
        node_create!(parser, "tr");
        node_create!(parser, "td");
        node_create!(parser, "button");
        node_create!(parser, "span");

        assert!(parser.is_in_scope("td", HTML_NAMESPACE, Scope::Regular));
        assert!(parser.is_in_scope("td", HTML_NAMESPACE, Scope::ListItem));
        assert!(!parser.is_in_scope("td", HTML_NAMESPACE, Scope::Button));
        assert!(parser.is_in_scope("td", HTML_NAMESPACE, Scope::Table));
        assert!(!parser.is_in_scope("td", HTML_NAMESPACE, Scope::Select));
    }

    #[test]
    fn is_in_scope_5() {
        let mut stream = ByteStream::new(Encoding::UTF8, None);
        let mut document = DocumentBuilderImpl::new_document::<Config>(None);
        let mut parser = Parser::new_parser(&mut stream, &mut document);
        node_create!(parser, "html");
        node_create!(parser, "body");
        node_create!(parser, "div");
        node_create!(parser, "object");
        node_create!(parser, "p");
        node_create!(parser, "a");
        node_create!(parser, "span");

        assert!(!parser.is_in_scope("div", HTML_NAMESPACE, Scope::Regular));
        assert!(!parser.is_in_scope("div", HTML_NAMESPACE, Scope::ListItem));
        assert!(!parser.is_in_scope("div", HTML_NAMESPACE, Scope::Button));
        assert!(parser.is_in_scope("div", HTML_NAMESPACE, Scope::Table));
        assert!(!parser.is_in_scope("div", HTML_NAMESPACE, Scope::Select));
    }

    #[test]
    fn is_in_scope_6() {
        let mut stream = ByteStream::new(Encoding::UTF8, None);
        let mut document = DocumentBuilderImpl::new_document::<Config>(None);
        let mut parser = Parser::new_parser(&mut stream, &mut document);
        node_create!(parser, "html");
        node_create!(parser, "body");
        node_create!(parser, "div");
        node_create!(parser, "ul");
        node_create!(parser, "li");
        node_create!(parser, "marquee");
        node_create!(parser, "p");

        assert!(!parser.is_in_scope("ul", HTML_NAMESPACE, Scope::Regular));
        assert!(!parser.is_in_scope("ul", HTML_NAMESPACE, Scope::ListItem));
        assert!(!parser.is_in_scope("ul", HTML_NAMESPACE, Scope::Button));
        assert!(parser.is_in_scope("ul", HTML_NAMESPACE, Scope::Table));
        assert!(!parser.is_in_scope("ul", HTML_NAMESPACE, Scope::Select));
    }

    #[test]
    fn is_in_scope_7() {
        let mut stream = ByteStream::new(Encoding::UTF8, None);
        let mut document = DocumentBuilderImpl::new_document::<Config>(None);
        let mut parser = Parser::new_parser(&mut stream, &mut document);
        node_create!(parser, "html");
        node_create!(parser, "body");
        node_create!(parser, "div");
        node_create!(parser, "table");
        node_create!(parser, "caption");
        node_create!(parser, "p");

        assert!(!parser.is_in_scope("table", HTML_NAMESPACE, Scope::Regular));
        assert!(!parser.is_in_scope("table", HTML_NAMESPACE, Scope::ListItem));
        assert!(!parser.is_in_scope("table", HTML_NAMESPACE, Scope::Button));
        assert!(parser.is_in_scope("table", HTML_NAMESPACE, Scope::Table));
        assert!(!parser.is_in_scope("table", HTML_NAMESPACE, Scope::Select));
    }

    #[test]
    fn is_in_scope_8() {
        let mut stream = ByteStream::new(Encoding::UTF8, None);
        let mut document = DocumentBuilderImpl::new_document::<Config>(None);
        let mut parser = Parser::new_parser(&mut stream, &mut document);
        node_create!(parser, "html");
        node_create!(parser, "body");
        node_create!(parser, "select");
        node_create!(parser, "optgroup");
        node_create!(parser, "option");

        assert!(parser.is_in_scope("select", HTML_NAMESPACE, Scope::Regular));
        assert!(parser.is_in_scope("select", HTML_NAMESPACE, Scope::ListItem));
        assert!(parser.is_in_scope("select", HTML_NAMESPACE, Scope::Button));
        assert!(parser.is_in_scope("select", HTML_NAMESPACE, Scope::Table));
        assert!(parser.is_in_scope("select", HTML_NAMESPACE, Scope::Select));
    }

    #[test]
    fn reconstruct_formatting() {
        let mut stream = ByteStream::from_str("<p><b>bold<i>bold and italic</b>italic</i></p>", Encoding::UTF8);

        let mut doc_handle = DocumentBuilderImpl::new_document::<Config>(None);
        let _ = Parser::parse_document(&mut stream, &mut doc_handle, None);

        println!("{doc_handle}");
    }

    #[test]
    fn element_with_classes() {
        let mut stream = ByteStream::from_str("<div class=\"one two three\"></div>", Encoding::UTF8);

        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let _ = Parser::parse_document(&mut stream, &mut doc, None);

        // document -> html -> head -> body -> div
        let div = doc.node_by_id(4usize.into()).unwrap();

        let NodeDataTypeInternal::Element(element) = &div.data else {
            panic!()
        };

        assert_eq!(element.classlist().len(), 3);

        assert!(element.classlist().contains("one"));
        assert!(element.classlist().contains("two"));
        assert!(element.classlist().contains("three"));

        assert_eq!(element.classlist().as_vec(), ["one", "two", "three"]);
    }

    #[test]
    fn element_with_classes_extra_whitespace() {
        let mut stream = ByteStream::from_str("<div class=\" one    two     three   \"></div>", Encoding::UTF8);

        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let _ = Parser::parse_document(&mut stream, &mut doc, None);

        // document -> html -> head -> body -> div
        let div = doc.node_by_id(4usize.into()).unwrap();

        let NodeDataTypeInternal::Element(element) = &div.data else {
            panic!()
        };

        assert_eq!(element.classlist().len(), 3);

        assert!(element.classlist().contains("one"));
        assert!(element.classlist().contains("two"));
        assert!(element.classlist().contains("three"));

        assert_eq!(element.classlist().as_vec(), ["one", "two", "three"]);
    }

    #[test]
    fn element_with_invalid_named_id() {
        let mut stream = ByteStream::from_str(
            "<div id=\"my id\"></div> \
             <div id=\"\"></div>",
            Encoding::UTF8,
        );

        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let _ = Parser::parse_document(&mut stream, &mut doc, None);

        // Whitespace makes an id non-conforming but it is still the element's id. An empty id
        // is no id at all.
        assert!(doc.node_by_named_id("my id").is_some());
        assert!(doc.node_by_named_id("my").is_none());
        assert!(doc.node_by_named_id("").is_none());
    }

    #[test]
    fn element_with_named_id() {
        let mut stream = ByteStream::from_str(
            "<div id=\"myid\"></div> \
             <p id=\"myid\"></p>",
            Encoding::UTF8,
        );

        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let _ = Parser::parse_document(&mut stream, &mut doc, None);

        // we are expecting the div (ID: 4) and p would be ignored
        let div = doc.node_by_named_id("myid").unwrap();
        assert_eq!(div, NodeId::from(4usize));
        assert_eq!(doc.tag_name(div), Some("div"));
    }

    #[test]
    fn ids_inside_a_template_are_not_found() {
        let mut stream = ByteStream::from_str(
            "<template id=\"t\"><span id=\"inside\"></span><div><p id=\"deep\"></p></div></template>",
            Encoding::UTF8,
        );

        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let _ = Parser::parse_document(&mut stream, &mut doc, None);

        assert!(
            doc.node_by_named_id("t").is_some(),
            "the template itself is in the tree"
        );
        assert!(doc.node_by_named_id("inside").is_none());
        assert!(doc.node_by_named_id("deep").is_none());
    }

    #[test]
    fn a_duplicate_id_skips_template_contents() {
        let mut stream = ByteStream::from_str(
            "<template><span id=\"dup\"></span></template><div id=\"dup\"></div><p id=\"dup\"></p>",
            Encoding::UTF8,
        );

        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let _ = Parser::parse_document(&mut stream, &mut doc, None);

        let found = doc.node_by_named_id("dup").unwrap();
        assert_eq!(doc.tag_name(found), Some("div"));
    }

    /// Answers with a fixed stylesheet and records what it was asked for.
    #[derive(Debug, Default)]
    struct RecordingSheets {
        asked: parking_lot::Mutex<Vec<String>>,
    }

    impl StylesheetSource for RecordingSheets {
        fn fetch_blocking(&self, urls: &[String]) -> Vec<Option<Vec<u8>>> {
            self.asked.lock().extend(urls.iter().cloned());
            urls.iter().map(|_| Some(b"p { color: red; }".to_vec())).collect()
        }
    }

    /// Parse `html` with a source in place, and report what the parse asked that source for.
    fn sheets_awaited_by(html: &str) -> Vec<String> {
        let source = Arc::new(RecordingSheets::default());
        let options = Html5ParserOptions {
            stylesheets: Some(source.clone()),
            ..Default::default()
        };
        let mut stream = ByteStream::from_str(html, Encoding::UTF8);
        let mut doc = DocumentBuilderImpl::new_document::<Config>(Some(
            url::Url::parse("http://example.test/page.html").unwrap(),
        ));
        let _ = Html5Parser::<Config>::parse_document(&mut stream, &mut doc, Some(options));

        let asked = source.asked.lock().clone();
        asked
    }

    /// The rule this exists for: a classic script may not run until the stylesheets written
    /// before it have applied, so reaching one is what forces the wait.
    #[test]
    fn a_blocking_script_waits_for_the_stylesheets_before_it() {
        let asked = sheets_awaited_by(
            r#"<html><head><link rel="stylesheet" href="/a.css"><script>1</script></head><body></body></html>"#,
        );
        assert_eq!(asked, vec!["http://example.test/a.css".to_string()]);
    }

    /// `async` and `defer` say the script is not in the way, so nothing waits for it.
    #[test]
    fn a_deferred_script_waits_for_nothing() {
        let asked = sheets_awaited_by(
            r#"<html><head><link rel="stylesheet" href="/a.css"><script src="/s.js" defer></script></head><body></body></html>"#,
        );
        assert!(asked.is_empty(), "expected no wait, got {asked:?}");

        let asked = sheets_awaited_by(
            r#"<html><head><link rel="stylesheet" href="/a.css"><script src="/s.js" async></script></head><body></body></html>"#,
        );
        assert!(asked.is_empty(), "expected no wait, got {asked:?}");
    }

    /// ... but they say nothing about a script with no `src`: there is nothing to fetch, so
    /// it runs where it stands and everything before it has to be ready.
    #[test]
    fn an_inline_script_waits_even_when_marked_defer() {
        let asked = sheets_awaited_by(
            r#"<html><head><link rel="stylesheet" href="/a.css"><script defer>1</script></head><body></body></html>"#,
        );
        assert_eq!(asked, vec!["http://example.test/a.css".to_string()]);
    }

    /// Only what came before it. A link after the script has not been seen yet when the
    /// wait happens, and is left for the pipeline to resolve after the parse.
    #[test]
    fn a_script_does_not_wait_for_stylesheets_written_after_it() {
        let asked = sheets_awaited_by(
            r#"<html><head><link rel="stylesheet" href="/first.css"><script>1</script><link rel="stylesheet" href="/second.css"></head><body></body></html>"#,
        );
        assert_eq!(asked, vec!["http://example.test/first.css".to_string()]);
    }

    /// A page with no script never asks, which is the common case and must stay free.
    #[test]
    fn a_page_without_scripts_never_waits() {
        let asked =
            sheets_awaited_by(r#"<html><head><link rel="stylesheet" href="/a.css"></head><body></body></html>"#);
        assert!(asked.is_empty(), "expected no wait, got {asked:?}");
    }

    /// Parses `bytes` the way the engine does: a byte stream with a tentative (or, with
    /// `certain`, a BOM/transport-given) encoding. Returns the stream and the printed tree.
    fn parse_bytes(bytes: &[u8], certain: bool) -> (ByteStream, String) {
        let mut stream = ByteStream::new(Encoding::UTF8, None);
        stream.read_from_bytes(bytes).unwrap();
        if certain {
            stream.set_confidence(Confidence::Certain);
        }
        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let _ = Parser::parse_document(&mut stream, &mut doc, None);
        let tree = format!("{doc}");
        (stream, tree)
    }

    /// Only the first `<meta charset>` changes a tentative encoding; it makes the
    /// encoding certain, so the second one neither switches back nor re-decodes.
    #[test]
    fn only_the_first_meta_charset_changes_the_encoding() {
        let (stream, tree) = parse_bytes(
            "<meta charset=iso-8859-1><meta charset=utf-8><p>é</p>".as_bytes(),
            false,
        );
        assert_eq!(stream.encoding(), Encoding::Latin1);
        assert_eq!(stream.confidence(), Confidence::Certain);
        assert!(
            tree.contains("\u{00C3}\u{00A9}"),
            "expected latin-1 decoding, got {tree}"
        );
    }

    /// A meta naming the encoding already in use makes it certain without a change,
    /// so a later, different declaration is ignored.
    #[test]
    fn meta_charset_matching_the_encoding_makes_it_certain() {
        let (stream, tree) = parse_bytes(
            "<meta charset=utf-8><meta charset=iso-8859-1><p>é</p>".as_bytes(),
            false,
        );
        assert_eq!(stream.encoding(), Encoding::UTF8);
        assert_eq!(stream.confidence(), Confidence::Certain);
        assert!(tree.contains('é'), "expected utf-8 decoding, got {tree}");
    }

    /// An encoding from a BOM or the transport layer is certain, so meta is ignored.
    #[test]
    fn meta_charset_is_ignored_when_the_encoding_is_certain() {
        let (stream, tree) = parse_bytes(
            "\u{FEFF}<meta http-equiv=Content-Type content='text/html; charset=iso-8859-1'><p>é</p>".as_bytes(),
            true,
        );
        assert_eq!(stream.encoding(), Encoding::UTF8);
        assert!(tree.contains('é'), "expected utf-8 decoding, got {tree}");
    }
}

#[cfg(test)]
mod text_location_tests {
    use crate::document::document_impl::DocumentImpl;
    use crate::parser::Html5Parser;
    use gosub_css3::system::Css3System;
    use gosub_interface::config::ModuleConfiguration;
    use gosub_interface::document::Document;
    use gosub_shared::byte_stream::{ByteStream, Encoding};
    use gosub_shared::node::NodeId;

    #[derive(Clone, Debug, PartialEq)]
    struct Config;
    impl ModuleConfiguration for Config {
        type CssSystem = Css3System;
        type Document = DocumentImpl<Self>;
        type HtmlParser = Html5Parser<'static, Self>;
    }

    /// First node, depth first, whose text or comment value is `value`.
    fn find_value(doc: &DocumentImpl<Config>, id: NodeId, value: &str) -> Option<NodeId> {
        if doc.text_value(id) == Some(value) || doc.comment_value(id) == Some(value) {
            return Some(id);
        }
        doc.children(id).iter().find_map(|&c| find_value(doc, c, value))
    }

    /// Line and column of the text or comment node holding `value`.
    fn location_of(html: &str, value: &str) -> (usize, usize) {
        let doc = crate::html_compile::<Config>(html);
        let id = find_value(&doc, NodeId::root(), value).unwrap();
        let location = doc.node_by_id(id).unwrap().location();
        (location.line, location.column)
    }

    /// Line and column of every parse error with this message.
    fn errors_at(html: &str, message: &str) -> Vec<(usize, usize)> {
        let mut stream = ByteStream::from_str(html, Encoding::UTF8);
        let mut doc = crate::document::builder::DocumentBuilderImpl::new_document::<Config>(None);
        let errors = Html5Parser::<Config>::parse_document(&mut stream, &mut doc, None).unwrap();
        errors
            .iter()
            .filter(|e| e.message == message)
            .map(|e| (e.location.line, e.location.column))
            .collect()
    }

    #[test]
    fn text_in_body() {
        assert_eq!(location_of("<p>hello<b>x", "hello"), (1, 4));
        assert_eq!(location_of("<p>hello<b>x", "x"), (1, 12));
        assert_eq!(location_of("<p>a\n  bc</p>", "a\n  bc"), (1, 4));
    }

    #[test]
    fn text_in_rcdata_rawtext_script_and_plaintext() {
        assert_eq!(location_of("<title>ab</title>", "ab"), (1, 8));
        assert_eq!(location_of("<textarea>ab</textarea>", "ab"), (1, 11));
        assert_eq!(location_of("<style>x</style>", "x"), (1, 8));
        assert_eq!(location_of("<script>x</script>", "x"), (1, 9));
        assert_eq!(location_of("<xmp>x</xmp>", "x"), (1, 6));
        assert_eq!(location_of("<plaintext>x</p>", "x</p>"), (1, 12));
    }

    #[test]
    fn text_in_cdata_section() {
        assert_eq!(location_of("<svg><![CDATA[x]]></svg>", "x"), (1, 15));
    }

    #[test]
    fn leading_newline_dropped_after_pre_and_textarea() {
        assert_eq!(location_of("<pre>\nfoo</pre>", "foo"), (2, 1));
        assert_eq!(location_of("<pre>foo</pre>", "foo"), (1, 6));
        assert_eq!(location_of("<textarea>\nab</textarea>", "ab"), (2, 1));
    }

    #[test]
    fn split_text_pieces_start_at_their_first_character() {
        // The whitespace before "x" is dropped, "x" itself goes into the body.
        assert_eq!(location_of("<html>  x", "x"), (1, 9));
        assert_eq!(location_of("<html>\n\n  x", "x"), (3, 3));
        assert_eq!(location_of("<html> \u{e9}\u{e9} x", "\u{e9}\u{e9} x"), (1, 8));
    }

    #[test]
    fn null_in_body_text() {
        assert_eq!(location_of("<body>a\0b", "ab"), (1, 7));
        assert_eq!(
            errors_at("<body>a\0b", "null character not allowed in in body insertion mode"),
            vec![(1, 8)]
        );
        assert_eq!(
            errors_at("<body>a\n\0b", "null character not allowed in in body insertion mode"),
            vec![(2, 1)]
        );
    }

    #[test]
    fn null_in_foreign_content() {
        assert_eq!(location_of("<svg>\0</svg>", "\u{fffd}"), (1, 6));
    }

    const FOREIGN_NULL: &str = "null character not allowed in foreign content";
    const BODY_NULL: &str = "null character not allowed in in body insertion mode";
    const SELECT_NULL: &str = "null character not allowed in in select insertion mode";
    const TABLE_TEXT_NULL: &str = "null character not allowed in in table text insertion mode";

    #[test]
    fn each_null_in_a_run_is_its_own_parse_error() {
        assert_eq!(errors_at("<svg>a\0\0\0b", FOREIGN_NULL), vec![(1, 7), (1, 8), (1, 9)]);
        assert_eq!(errors_at("<svg>\0\0\0", FOREIGN_NULL), vec![(1, 6), (1, 7), (1, 8)]);
        assert_eq!(errors_at("<math>a\0\0\0b", FOREIGN_NULL), vec![(1, 8), (1, 9), (1, 10)]);
        assert_eq!(errors_at("<body>a\0\0\0b", BODY_NULL), vec![(1, 8), (1, 9), (1, 10)]);
        assert_eq!(
            errors_at("<select>a\0\0\0b", SELECT_NULL),
            vec![(1, 10), (1, 11), (1, 12)]
        );
        assert_eq!(
            errors_at("<table>\0\0\0", TABLE_TEXT_NULL),
            vec![(1, 8), (1, 9), (1, 10)]
        );
    }

    #[test]
    fn each_null_in_a_run_is_its_own_parse_error_in_content_that_goes_through_in_body() {
        assert_eq!(errors_at("<template>\0\0", BODY_NULL), vec![(1, 11), (1, 12)]);
        assert_eq!(errors_at("<math><mi>\0\0", BODY_NULL), vec![(1, 11), (1, 12)]);
        assert_eq!(errors_at("<svg><foreignObject>\0\0", BODY_NULL), vec![(1, 21), (1, 22)]);
    }

    #[test]
    fn null_runs_split_by_other_text_each_count() {
        assert_eq!(
            errors_at("<select> \0\0 \0", SELECT_NULL),
            vec![(1, 10), (1, 11), (1, 13)]
        );
        assert_eq!(errors_at("<body>\0\na\0", BODY_NULL), vec![(1, 7), (2, 2)]);
    }

    #[test]
    fn a_single_null_is_still_one_parse_error() {
        assert_eq!(errors_at("<svg>a\0b", FOREIGN_NULL), vec![(1, 7)]);
        assert_eq!(errors_at("<table>\0", TABLE_TEXT_NULL), vec![(1, 8)]);
    }

    #[test]
    fn nulls_in_cdata_are_tree_builder_errors_only() {
        assert_eq!(errors_at("<svg><![CDATA[\0\0]]>", FOREIGN_NULL), vec![(1, 15), (1, 16)]);
        assert!(errors_at("<svg><![CDATA[\0\0]]>", "unexpected-null-character").is_empty());
    }

    #[test]
    fn tokenizer_null_errors_are_unchanged() {
        assert_eq!(errors_at("<body>a\0\0\0b", "unexpected-null-character").len(), 3);
    }

    #[test]
    fn pending_table_text() {
        // Fostered out of the table
        assert_eq!(location_of("<table>  x</table>", "  x"), (1, 8));
        // Whitespace only, stays in the table
        assert_eq!(location_of("<table>\n  <tr>", "\n  "), (1, 8));
    }

    #[test]
    fn comment_nodes() {
        assert_eq!(location_of("<p>a<!--c-->", "c"), (1, 5));
        assert_eq!(location_of("<p>a<!c>", "c"), (1, 5));
        assert_eq!(location_of("<p>a\n<?c>", "?c"), (2, 1));
    }
}
