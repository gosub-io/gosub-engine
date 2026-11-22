use core::cell::{Cell, RefCell};
use core::option::Option::Some;
use gosub_shared::attributes::Attributes;
use std::borrow::Cow;
#[cfg(all(feature = "debug_parser", test))]
use std::io::Write;
use std::rc::Rc;
use std::sync::Arc;

use crate::node::{HTML_NAMESPACE, MATHML_NAMESPACE, SVG_NAMESPACE};
use crate::parser::attr_replacements::{
    MATHML_ADJUSTMENTS, SVG_ADJUSTMENTS_ATTRIBUTES, SVG_ADJUSTMENTS_TAGS, XML_ADJUSTMENTS,
};
use crate::parser::errors::{ErrorLogger, ParserError};
use crate::parser::helper::{
    is_html_integration_point, is_mathml_text_integration_point, is_special, matches_tag_and_attrs_without_order,
};
use crate::parser::open_elements::OpenElements;
use crate::tokenizer::state::State;
use crate::tokenizer::token::Token;
use crate::tokenizer::{ParserData, Tokenizer, CHAR_REPLACEMENT};
use cow_utils::CowUtils;
use gosub_interface::config::HasDocument;
use gosub_interface::css3::{CssOrigin, CssSystem};
use gosub_interface::document::{Document, DocumentType, PendingStylesheet};
use gosub_interface::node::{NodeType, ShadowRootInit, ShadowRootMode, SlotAssignmentMode};

use gosub_interface::html5::ParserOptions;
use gosub_interface::node::QuirksMode;
use gosub_interface::script::{ScriptHost, ScriptOrigin};
use gosub_shared::byte_stream::{ByteStream, Encoding, Location};
use gosub_shared::config::{Context, ParserConfig};
use gosub_shared::node::NodeId;
use gosub_shared::timing::{self, Timer};
use gosub_shared::types::{ParseError, Result};
use log::warn;
use url::Url;

mod attr_replacements;
pub mod errors;
mod open_elements;
mod quirks;
#[cfg(test)]
mod tree_construction_tests;

// ------------------------------------------------------------

/// Whether a stylesheet loaded from `base` may import `resolved`.
///
/// `Url::join` returns the request unchanged when it carries its own scheme, so an `@import` can
/// name any URL it likes - including `file:`, which the fetch path would read straight off disk.
/// A document fetched over the network must never be able to reach the local filesystem that way,
/// so a remote sheet may only import over the network. A local sheet is left unrestricted: it can
/// already read the filesystem it came from.
#[cfg(not(target_arch = "wasm32"))]
fn import_scheme_allowed(base: Option<&Url>, resolved: &Url) -> bool {
    let base_is_remote = base.is_some_and(|u| u.scheme() != "file");
    !base_is_remote || matches!(resolved.scheme(), "http" | "https")
}

/// Whether `adjusted_attribute_name` renames `name`, without building the new name.
fn is_adjusted_attribute(table: Option<&phf::Map<&str, &str>>, name: &str) -> bool {
    table.is_some_and(|t| t.contains_key(name)) || XML_ADJUSTMENTS.contains_key(name)
}

/// The name `name` gets from `table` (MathML or SVG) and then from the foreign attribute
/// adjustments, or `None` when neither renames it.
fn adjusted_attribute_name(table: Option<&phf::Map<&str, &str>>, name: &str) -> Option<String> {
    let renamed = table.and_then(|t| t.get(name).copied());
    if let Some((prefix, local_name, _namespace)) = XML_ADJUSTMENTS.get(renamed.unwrap_or(name)) {
        return Some(format!("{prefix} {local_name}"));
    }
    renamed.map(str::to_owned)
}

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
            ActiveElement::Marker => None,
        }
    }
}

/// A script element the parser runs.
enum Script {
    Inline(String),
    External {
        url: String,
        /// `defer` or `async`: it does not hold up the parse.
        deferred: bool,
    },
}

/// The JavaScript MIME type essences.
const JAVASCRIPT_MIME_TYPES: [&str; 16] = [
    "application/ecmascript",
    "application/javascript",
    "application/x-ecmascript",
    "application/x-javascript",
    "text/ecmascript",
    "text/javascript",
    "text/javascript1.0",
    "text/javascript1.1",
    "text/javascript1.2",
    "text/javascript1.3",
    "text/javascript1.4",
    "text/javascript1.5",
    "text/jscript",
    "text/livescript",
    "text/x-ecmascript",
    "text/x-javascript",
];

/// Whether a script element with these `type` and `language` attributes is a classic
/// script: no type, or a JavaScript MIME type. A module or a data block is not.
fn is_classic_script_type(type_attr: Option<&str>, language: Option<&str>) -> bool {
    let is_javascript = |mime: &str| JAVASCRIPT_MIME_TYPES.iter().any(|js| mime.eq_ignore_ascii_case(js));

    match (type_attr, language) {
        (Some(""), _) | (None, None | Some("")) => true,
        // a type is matched after trimming, a language as it is written
        (Some(type_attr), _) => is_javascript(type_attr.trim_matches(|c: char| c.is_ascii_whitespace())),
        (None, Some(language)) => is_javascript(&format!("text/{language}")),
    }
}

pub struct Html5ParserOptions {
    pub scripting_enabled: bool,
    /// Where the bytes of a linked stylesheet come from when a script has to wait for one.
    ///
    /// `None` means nothing waits: the sheets are resolved after the parse instead, which
    /// is right for a document with no scripts in it and for every test in this crate.
    pub stylesheets: Option<Arc<dyn StylesheetSource>>,
    /// Runs the document's inline classic scripts, each when its end tag is seen. `None`
    /// means no script runs.
    pub scripts: Option<Box<dyn ScriptHost + Send>>,
    /// Where the text of a `<script src>` comes from. `None` means external scripts do not
    /// run. Only asked when `scripts` is set.
    pub script_sources: Option<Arc<dyn ScriptSource>>,
}

/// Somewhere the parser can get a linked stylesheet from, when it has to have one *now*.
///
/// The parser does not fetch, but a classic script may not run until the sheets before it
/// have applied, so it asks and blocks. That is only acceptable because the parse runs on the
/// blocking pool; on a runtime worker it would starve the fetches it waits for.
pub trait StylesheetSource: Send + Sync + std::fmt::Debug {
    /// Block until each URL has been fetched, and answer in the same order.
    ///
    /// `None` for a sheet that could not be had. A failure must come back as `None` rather
    /// than never returning: a page whose stylesheet 404s still has to finish parsing.
    fn fetch_blocking(&self, urls: &[String]) -> Vec<Option<Vec<u8>>>;
}

/// Somewhere the parser can get an external script from, when it is time to run it.
///
/// Like [`StylesheetSource`]: the parser does not fetch, it asks and blocks.
pub trait ScriptSource: Send + Sync + std::fmt::Debug {
    /// Block until the script at `url` has been fetched, and answer with its text.
    ///
    /// `None` for a script that could not be had; it must not hang instead.
    fn fetch_blocking(&self, url: &str) -> Option<String>;
}

impl ParserOptions for Html5ParserOptions {
    fn new(scripting: bool) -> Self {
        Self {
            scripting_enabled: scripting,
            stylesheets: None,
            scripts: None,
            script_sources: None,
        }
    }
}

impl Default for Html5ParserOptions {
    fn default() -> Self {
        Self {
            scripting_enabled: true,
            stylesheets: None,
            scripts: None,
            script_sources: None,
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
    /// What runs the document's scripts, if anything does.
    script_host: Option<Box<dyn ScriptHost + Send>>,
    script_source: Option<Arc<dyn ScriptSource>>,
    /// URLs of the `defer` and `async` scripts seen so far, in document order. They run
    /// when the document has been parsed.
    deferred_scripts: Vec<String>,
    /// Time spent running scripts and waiting for external ones, in microseconds: not part
    /// of the time the parse itself took.
    script_us: u64,
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

/// Defines the scopes for `in_scope()`
#[derive(Clone, Copy)]
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
        let (stylesheet_source, script_host, script_source) =
            options.map_or((None, None, None), |o| (o.stylesheets, o.scripts, o.script_sources));
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
            stylesheet_source,
            script_host,
            script_source,
            deferred_scripts: Vec::new(),
            script_us: 0,
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
            script_host: None,
            script_source: None,
            deferred_scripts: Vec::new(),
            script_us: 0,
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

    /// Parses a fragment of HTML instead of a whole document. It will run the parser in a slightly different mode.
    /// This is used for parsing innerHTML and document fragments.
    pub fn parse_fragment(
        stream: &mut ByteStream,
        document: &mut C::Document,
        context_node_id: NodeId,
        options: Option<Html5ParserOptions>,
        start_location: Location,
    ) -> Result<Vec<ParseError>> {
        // https://html.spec.whatwg.org/multipage/parsing.html#parsing-html-fragments

        // 1.
        document.set_doctype(DocumentType::HTML);

        // 2.
        // Obsoleted: if doc_weak is none for some reason, it will default to no quirks mode

        // 3.
        let error_logger = Rc::new(RefCell::new(ErrorLogger::new()));

        let tokenizer = Tokenizer::new(stream, None, error_logger.clone());
        let mut parser = Html5Parser::<C>::init(tokenizer, document, error_logger, options);

        // Fragment parsing is what `innerHTML` runs on, and that must not build shadow roots.
        parser.allow_declarative_shadow_roots = false;

        // 4. / 12.
        parser.initialize_fragment_case(context_node_id);

        // 5. / 6. Create a new html element and append it to the Document node.
        let html_id = parser
            .document
            .create_element("html", Some(HTML_NAMESPACE), Attributes::new(), start_location);
        let root = parser.document.root();
        parser.attach_node(html_id, root, None);

        // 7.
        parser.open_elements.push(html_id);

        // 8.
        if parser.document.tag_name(context_node_id).unwrap_or_default() == "template" {
            parser.template_insertion_mode.push(InsertionMode::InTemplate);
        }

        // 9.
        // @Todo: this does not do anything yet
        let node_attributes = parser.document.attributes(context_node_id).cloned().unwrap_or_default();
        let _token = Token::StartTag {
            name: parser
                .document
                .tag_name(context_node_id)
                .unwrap_or_default()
                .to_string(),
            is_self_closing: false,
            attributes: node_attributes,
            location: start_location,
        };

        // 10.
        parser.reset_insertion_mode();

        // 11. Set the parser's form element pointer to the nearest node to the context element that is a form element (going straight up the ancestor chain, and including the element itself, if it is a form element), if any. (If there is no such form element, the form element pointer keeps its initial value, null.)
        let mut cur_id = Some(context_node_id);
        while let Some(id) = cur_id {
            if parser.document.tag_name(id) == Some("form") && parser.document.namespace(id) == Some(HTML_NAMESPACE) {
                parser.form_element = Some(id);
                break;
            }
            // Template contents are a fragment of their own in the spec, so the chain ends there.
            // Here they are the template's children, which `parent` would walk straight out of.
            cur_id = parser
                .document
                .parent(id)
                .filter(|&parent| parser.document.template_contents(parent) != Some(parent));
        }

        // 13. / 14.
        parser.do_parse()
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
        // recorded under `script.blocked_on_css` and taken back out here, as is the time the
        // scripts themselves took.
        let mut wall = Timer::new(Some(context.clone()));
        let ret = parser.do_parse();
        wall.end();

        let parse_us = wall
            .duration()
            .saturating_sub(parser.blocked_on_css_us.get())
            .saturating_sub(parser.script_us);
        timing::record(gosub_shared::timing::Timing::DecodeHtml, parse_us, Some(context));

        parser.run_deferred_scripts();

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
            self.display_debug_info();
        }

        Ok(self.error_logger.borrow().get_errors())
    }

    // Process token in foreign content (svg, mathml)
    fn process_foreign_content(&mut self) {
        // Move the token out as `process_html_content` does. The arms that hand the token
        // to HTML processing only set `delegate`; the token is put back before that call,
        // as `process_html_content` re-reads `self.current_token`.
        let loc = self.current_token.get_location();
        let mut current_token = std::mem::replace(&mut self.current_token, Token::Eof { location: loc });

        let mut handle_as_script_endtag = false;
        let mut delegate = Delegate::None;

        match &current_token {
            Token::Text { text: value, location } if current_token.is_mixed() => {
                let tokens = self.split_mixed_token(value, *location);
                self.tokenizer.insert_tokens_at_queue_start(tokens);
                return;
            }
            Token::Text { text: value, .. } if current_token.is_null() => {
                self.null_parse_errors(value, loc, "null character not allowed in foreign content");
                // A run of NULs arrives as one token; each NUL gets its own replacement
                self.insert_text_element(&Token::Text {
                    text: CHAR_REPLACEMENT.to_string().repeat(value.chars().count()),
                    location: loc,
                });
            }
            Token::Text { .. } if current_token.is_empty_or_white() => {
                self.insert_text_element(&current_token);
            }
            Token::Text { .. } => {
                self.insert_text_element(&current_token);

                self.frameset_ok = false;
            }
            Token::Comment { .. } => {
                self.insert_comment_element(&current_token, None);
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
                delegate = Delegate::UnexpectedHtmlTag;
            }
            Token::StartTag { name, attributes, .. }
                if name == "font"
                    && (attributes.contains_key("color")
                        || attributes.contains_key("face")
                        || attributes.contains_key("size")) =>
            {
                delegate = Delegate::UnexpectedHtmlTag;
            }
            Token::EndTag { name, .. } if name == "br" || name == "p" => {
                delegate = Delegate::UnexpectedHtmlTag;
            }
            Token::StartTag {
                name, is_self_closing, ..
            } => {
                let is_self_closing = *is_self_closing;
                let is_script = name == "script";

                let acn_id = self.get_adjusted_current_node_id();
                let acn_ns = self.document.namespace(acn_id).unwrap_or(HTML_NAMESPACE).to_owned();
                if acn_ns == SVG_NAMESPACE {
                    self.adjust_svg_tag_names(&mut current_token);
                }
                self.adjust_foreign_attributes(&mut current_token, &acn_ns);
                self.insert_foreign_element(&mut current_token, &acn_ns);

                if is_self_closing {
                    let cn_id = current_node_id!(self);
                    if is_script && node_ns!(self, cn_id) == Some(SVG_NAMESPACE) {
                        self.ack_self_closing = true;
                        handle_as_script_endtag = true;
                    } else {
                        self.open_elements.pop();
                        self.ack_self_closing = true;
                    }
                }
            }
            Token::EndTag { name, .. }
                if name == "script"
                    && node_tag!(self, current_node_id!(self)) == "script"
                    && node_ns!(self, current_node_id!(self)) == Some(SVG_NAMESPACE) =>
            {
                handle_as_script_endtag = true;
            }
            Token::EndTag { name, .. } => {
                if self.open_elements.is_empty() {
                    return;
                }

                let mut node_idx = self.open_elements.len() - 1;
                let mut cur_node_id = self.open_elements[node_idx];

                if node_tag!(self, cur_node_id).cow_to_lowercase() != *name {
                    self.parse_error("end tag does not match current node");
                }

                loop {
                    // Fragment case is when the first element in the stack is this node
                    match self.open_elements.first() {
                        // fragment case
                        Some(first_id) if *first_id == cur_node_id => return,
                        _ => {}
                    }

                    if node_tag!(self, cur_node_id).cow_to_lowercase() == *name {
                        while let Some(node_id) = self.open_elements.pop() {
                            if node_id == cur_node_id {
                                break;
                            }
                        }
                        return;
                    }

                    node_idx -= 1;
                    cur_node_id = self.open_elements[node_idx];

                    if node_ns!(self, cur_node_id) != Some(HTML_NAMESPACE) {
                        continue;
                    }

                    delegate = Delegate::HtmlContent;
                    break;
                }
            }
            Token::Eof { .. } => {
                return;
            }
        }

        match delegate {
            Delegate::None => {}
            Delegate::UnexpectedHtmlTag => {
                self.current_token = current_token;
                self.process_unexpected_html_tag();
                return;
            }
            Delegate::HtmlContent => {
                self.current_token = current_token;
                self.process_html_content();
                return;
            }
        }

        if handle_as_script_endtag {
            let script = current_node_id!(self);
            self.open_elements.pop();

            let old_insertion_point = self.insertion_point;
            self.insertion_point = Some(self.tokenizer.get_location().offset);

            self.script_nesting_level += 1;

            let ran = self.run_script(script);

            self.script_nesting_level -= 1;
            if self.script_nesting_level == 0 {
                self.parser_pause_flag = false;
                if ran {
                    self.script_checkpoint();
                }
            }

            self.insertion_point = old_insertion_point;
        }
    }

    /// Process a token in HTML content
    fn process_html_content(&mut self) {
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
            InsertionMode::Initial => {
                let mut anything_else = false;

                match &current_token {
                    Token::Text { text: value, location } if current_token.is_mixed() => {
                        let tokens = self.split_mixed_token(value, *location);
                        self.tokenizer.insert_tokens_at_queue_start(tokens);
                        return;
                    }
                    Token::Text { .. } if current_token.is_empty_or_white() => {
                        // ignore token
                    }
                    Token::Comment { .. } => {
                        self.insert_comment_element(&current_token, Some(NodeId::root()));
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

                        self.insert_doctype_element(&current_token);

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
                    if !self.parser_cannot_change_mode {
                        self.set_quirks_mode(QuirksMode::Quirks);
                    }

                    self.insertion_mode = InsertionMode::BeforeHtml;
                    self.reprocess_token = true;
                }
            }
            InsertionMode::BeforeHtml => {
                let mut anything_else = false;

                match &current_token {
                    Token::DocType { .. } => {
                        self.parse_error("doctype not allowed in before html insertion mode");
                    }
                    Token::Comment { .. } => {
                        self.insert_comment_element(&current_token, Some(NodeId::root()));
                    }
                    Token::Text { text: value, location } if current_token.is_mixed() => {
                        let tokens = self.split_mixed_token(value, *location);
                        self.tokenizer.insert_tokens_at_queue_start(tokens);
                        return;
                    }
                    Token::Text { .. } if current_token.is_empty_or_white() => {
                        // ignore token
                    }
                    Token::StartTag { name, .. } if name == "html" => {
                        self.insert_document_element(&mut current_token);

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
                    let mut token = Token::StartTag {
                        name: "html".to_string(),
                        is_self_closing: false,
                        attributes: Attributes::new(),
                        location: current_token.get_location(),
                    };
                    self.insert_document_element(&mut token);

                    self.insertion_mode = InsertionMode::BeforeHead;
                    self.reprocess_token = true;
                }
            }
            InsertionMode::BeforeHead => {
                let mut anything_else = false;

                match &current_token {
                    Token::Text { text: value, location } if current_token.is_mixed() => {
                        let tokens = self.split_mixed_token(value, *location);
                        self.tokenizer.insert_tokens_at_queue_start(tokens);
                        return;
                    }
                    Token::Text { .. } if current_token.is_empty_or_white() => {
                        // ignore token
                    }
                    Token::Comment { .. } => {
                        self.insert_comment_element(&current_token, None);
                    }
                    Token::DocType { .. } => {
                        self.parse_error("doctype not allowed in before head insertion mode");
                        // ignore token
                    }
                    Token::StartTag { name, .. } if name == "html" => {
                        self.handle_in_body(&mut current_token);
                    }
                    Token::StartTag { name, .. } if name == "head" => {
                        let node_id = self.insert_html_element(&mut current_token);
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
                    let mut token = Token::StartTag {
                        name: "head".to_string(),
                        is_self_closing: false,
                        attributes: Attributes::new(),
                        location: current_token.get_location(),
                    };
                    let node_id = self.insert_html_element(&mut token);
                    self.head_element = Some(node_id);
                    self.insertion_mode = InsertionMode::InHead;
                    self.reprocess_token = true;
                }
            }
            InsertionMode::InHead => self.handle_in_head(&mut current_token),
            InsertionMode::InHeadNoscript => {
                let mut anything_else = false;

                match &current_token {
                    Token::DocType { .. } => {
                        self.parse_error("doctype not allowed in 'head no script' insertion mode");
                        // ignore token
                        return;
                    }
                    Token::StartTag { name, .. } if name == "html" => {
                        self.handle_in_body(&mut current_token);
                    }
                    Token::EndTag { name, .. } if name == "noscript" => {
                        self.pop_check("noscript");
                        self.insertion_mode = InsertionMode::InHead;
                    }
                    Token::Text { text: value, location } if current_token.is_mixed() => {
                        let tokens = self.split_mixed_token(value, *location);
                        self.tokenizer.insert_tokens_at_queue_start(tokens);
                        return;
                    }
                    Token::Text { .. } if current_token.is_empty_or_white() => {
                        self.handle_in_head(&mut current_token);
                    }
                    Token::Comment { .. } => {
                        self.handle_in_head(&mut current_token);
                    }
                    Token::StartTag { name, .. }
                        if name == "basefont"
                            || name == "bgsound"
                            || name == "link"
                            || name == "meta"
                            || name == "noframes"
                            || name == "style" =>
                    {
                        self.handle_in_head(&mut current_token);
                    }
                    Token::EndTag { name, .. } if name == "br" => {
                        anything_else = true;
                    }
                    Token::StartTag { name, .. } if name == "head" || name == "noscript" => {
                        self.parse_error("head or noscript tag not allowed in after head insertion mode");
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

                    self.insertion_mode = InsertionMode::InHead;
                    self.reprocess_token = true;
                }
            }
            InsertionMode::AfterHead => {
                let mut anything_else = false;

                match &current_token {
                    Token::Text { text: value, location } if current_token.is_mixed() => {
                        let tokens = self.split_mixed_token(value, *location);
                        self.tokenizer.insert_tokens_at_queue_start(tokens);
                        return;
                    }
                    Token::Text { .. } if current_token.is_empty_or_white() => {
                        self.insert_text_element(&current_token);
                    }
                    Token::Comment { .. } => {
                        self.insert_comment_element(&current_token, None);
                    }
                    Token::DocType { .. } => {
                        self.parse_error("doctype not allowed in after head insertion mode");
                        // ignore token
                    }
                    Token::StartTag { name, .. } if name == "html" => {
                        self.handle_in_body(&mut current_token);
                    }
                    Token::StartTag { name, .. } if name == "body" => {
                        self.insert_html_element(&mut current_token);

                        self.frameset_ok = false;
                        self.insertion_mode = InsertionMode::InBody;
                    }
                    Token::StartTag { name, .. } if name == "frameset" => {
                        self.insert_html_element(&mut current_token);

                        self.insertion_mode = InsertionMode::InFrameset;
                    }
                    Token::StartTag { name, .. }
                        if [
                            "base", "basefont", "bgsound", "link", "meta", "noframes", "script", "style", "template",
                            "title",
                        ]
                        .contains(&name.as_str()) =>
                    {
                        self.parse_error("invalid start tag in after head insertion mode");

                        if let Some(node_id) = self.head_element {
                            self.open_elements.push(node_id);
                        }

                        self.handle_in_head(&mut current_token);

                        // Remove the node pointed to by the head element pointer from the stack of open elements (might not be current node at this point)
                        if let Some(node_id) = self.head_element {
                            self.open_elements_remove(node_id);
                        }
                    }
                    Token::EndTag { name, .. } if name == "template" => {
                        self.handle_in_head(&mut current_token);
                    }
                    Token::EndTag { name, .. } if name == "body" || name == "html" || name == "br" => {
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
                    let mut token = Token::StartTag {
                        name: "body".to_string(),
                        is_self_closing: false,
                        attributes: Attributes::new(),
                        location: current_token.get_location(),
                    };
                    self.insert_html_element(&mut token);

                    self.insertion_mode = InsertionMode::InBody;
                    self.reprocess_token = true;
                }
            }
            InsertionMode::InBody => self.handle_in_body(&mut current_token),
            InsertionMode::Text => {
                match &current_token {
                    Token::Text { .. } => {
                        self.insert_text_element(&current_token);
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
                        if let Some(mut stylesheet) = self.load_inline_stylesheet(CssOrigin::Author, style_text_node_id)
                        {
                            if let Some(media) = self.document.attribute(style_node_id, "media") {
                                C::CssSystem::set_stylesheet_media(&mut stylesheet, media);
                            }
                            self.add_stylesheet_for(style_node_id, stylesheet);
                        }

                        self.open_elements.pop();
                        self.insertion_mode = self.original_insertion_mode;
                    }
                    Token::EndTag { name, .. } if name == "script" => {
                        // The microtask checkpoint the spec has here finds nothing to do: no
                        // script is on the stack, and the one before this ran its own.
                        let script = current_node_id!(self);
                        self.open_elements.pop();
                        self.insertion_mode = self.original_insertion_mode;

                        let old_insertion_point = self.insertion_point;
                        self.insertion_point = Some(self.tokenizer.get_location().offset);

                        // A blocking script waits for the stylesheets before it, whether or
                        // not anything runs it.
                        if self.current_script_blocks {
                            self.settle_stylesheets();
                        }

                        self.script_nesting_level += 1;

                        let ran = self.run_script(script);

                        self.script_nesting_level -= 1;
                        if self.script_nesting_level == 0 {
                            self.parser_pause_flag = false;
                            if ran {
                                self.script_checkpoint();
                            }
                        }

                        self.insertion_point = old_insertion_point;
                    }
                    _ => {
                        self.open_elements.pop();
                        self.insertion_mode = self.original_insertion_mode;
                    }
                }
            }
            InsertionMode::InTable => self.handle_in_table(&mut current_token),
            InsertionMode::InTableText => {
                match &current_token {
                    Token::Text { text: value, location } if current_token.is_mixed() => {
                        let tokens = self.split_mixed_token(value, *location);
                        self.tokenizer.insert_tokens_at_queue_start(tokens);
                    }
                    Token::Text { text: value, location } if current_token.is_null() => {
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
                            let tmp = current_token.clone();
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
            InsertionMode::InCaption => {
                let mut process_incaption_body = false;

                match &current_token {
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
                    _ => self.handle_in_body(&mut current_token),
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
            InsertionMode::InColumnGroup => {
                match &current_token {
                    Token::Text { text: value, location } if current_token.is_mixed() => {
                        let tokens = self.split_mixed_token(value, *location);
                        self.tokenizer.insert_tokens_at_queue_start(tokens);
                    }
                    Token::Text { .. } if current_token.is_empty_or_white() => {
                        self.insert_text_element(&current_token);
                    }
                    Token::Comment { .. } => {
                        self.insert_comment_element(&current_token, None);
                    }
                    Token::DocType { .. } => {
                        self.parse_error("doctype not allowed in column group insertion mode");
                        // ignore token
                    }
                    Token::StartTag { name, .. } if name == "html" => {
                        self.handle_in_body(&mut current_token);
                    }
                    Token::StartTag {
                        name, is_self_closing, ..
                    } if name == "col" => {
                        self.acknowledge_closing_tag(*is_self_closing);

                        self.insert_html_element(&mut current_token);
                        self.open_elements.pop();
                    }
                    Token::StartTag { name, .. } if name == "template" => {
                        self.handle_in_head(&mut current_token);
                    }
                    Token::EndTag { name, .. } if name == "template" => {
                        self.handle_in_head(&mut current_token);
                    }
                    Token::Eof { .. } => {
                        self.handle_in_body(&mut current_token);
                    }
                    Token::EndTag { name, .. } if name == "colgroup" => {
                        if node_tag!(self, current_node_id!(self)) != "colgroup" {
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
            InsertionMode::InTableBody => {
                match &current_token {
                    Token::StartTag { name, .. } if name == "tr" => {
                        self.clear_stack_back_to_table_body_context();

                        self.insert_html_element(&mut current_token);

                        self.insertion_mode = InsertionMode::InRow;
                    }
                    Token::StartTag { name, .. } if name == "th" || name == "td" => {
                        self.parse_error("th or td tag not allowed in in table body insertion mode");

                        self.clear_stack_back_to_table_body_context();

                        let mut token = Token::StartTag {
                            name: "tr".to_string(),
                            is_self_closing: false,
                            attributes: Attributes::new(),
                            location: current_token.get_location(),
                        };
                        self.insert_html_element(&mut token);

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
                        if ["body", "caption", "col", "colgroup", "html", "td", "th", "tr"]
                            .contains(&name.as_str()) =>
                    {
                        self.parse_error("end tag not allowed in in table body insertion mode");
                        // ignore token
                    }
                    _ => {
                        self.handle_in_table(&mut current_token);
                    }
                }
            }
            InsertionMode::InRow => {
                match &current_token {
                    Token::StartTag { name, .. } if name == "th" || name == "td" => {
                        self.clear_stack_back_to_table_row_context();

                        self.insert_html_element(&mut current_token);

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
                    _ => self.handle_in_table(&mut current_token),
                }
            }
            InsertionMode::InCell => {
                match &current_token {
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
                    _ => self.handle_in_body(&mut current_token),
                }
            }
            InsertionMode::InSelect => self.handle_in_select(&mut current_token),
            InsertionMode::InSelectInTable => {
                match &current_token {
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
                    _ => self.handle_in_select(&mut current_token),
                }
            }
            InsertionMode::InTemplate => self.handle_in_template(&mut current_token),
            InsertionMode::AfterBody => {
                match &current_token {
                    Token::Text { text: value, location } if current_token.is_mixed() => {
                        let tokens = self.split_mixed_token(value, *location);
                        self.tokenizer.insert_tokens_at_queue_start(tokens);
                    }
                    Token::Text { .. } if current_token.is_empty_or_white() => {
                        self.handle_in_body(&mut current_token);
                    }
                    Token::Comment { .. } => {
                        let html_node_id = self.open_elements.first().unwrap_or_default();
                        self.insert_comment_element(&current_token, Some(*html_node_id));
                    }
                    Token::DocType { .. } => {
                        self.parse_error("doctype not allowed in after body insertion mode");
                        // ignore token
                    }
                    Token::StartTag { name, .. } if name == "html" => {
                        self.handle_in_body(&mut current_token);
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
            InsertionMode::InFrameset => {
                match &current_token {
                    Token::Text { text: value, location } if current_token.is_mixed() => {
                        let tokens = self.split_mixed_token(value, *location);
                        self.tokenizer.insert_tokens_at_queue_start(tokens);
                    }
                    Token::Text { .. } if current_token.is_empty_or_white() => {
                        self.insert_text_element(&current_token);
                    }
                    Token::Comment { .. } => {
                        self.insert_comment_element(&current_token, None);
                    }
                    Token::DocType { .. } => {
                        self.parse_error("doctype not allowed in frameset insertion mode");
                        // ignore token
                    }
                    Token::StartTag { name, .. } if name == "html" => {
                        self.handle_in_body(&mut current_token);
                    }
                    Token::StartTag { name, .. } if name == "frameset" => {
                        self.insert_html_element(&mut current_token);
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

                        self.insert_html_element(&mut current_token);
                        self.open_elements.pop();
                    }
                    Token::StartTag { name, .. } if name == "noframes" => {
                        self.handle_in_head(&mut current_token);
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
            InsertionMode::AfterFrameset => {
                match &current_token {
                    Token::Text { text: value, location } if current_token.is_mixed() => {
                        let tokens = self.split_mixed_token(value, *location);
                        self.tokenizer.insert_tokens_at_queue_start(tokens);
                    }
                    Token::Text { .. } if current_token.is_empty_or_white() => {
                        self.insert_text_element(&current_token);
                    }
                    Token::Comment { .. } => {
                        self.insert_comment_element(&current_token, None);
                    }
                    Token::DocType { .. } => {
                        self.parse_error("doctype not allowed in frameset insertion mode");
                        // ignore token
                    }
                    Token::StartTag { name, .. } if name == "html" => {
                        self.handle_in_body(&mut current_token);
                    }
                    Token::EndTag { name, .. } if name == "html" => {
                        self.insertion_mode = InsertionMode::AfterAfterFrameset;
                    }
                    Token::StartTag { name, .. } if name == "noframes" => {
                        self.handle_in_head(&mut current_token);
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
            InsertionMode::AfterAfterBody => match &current_token {
                Token::Comment { .. } => {
                    self.insert_comment_element(&current_token, Some(NodeId::root()));
                }
                Token::DocType { .. } => {
                    self.handle_in_body(&mut current_token);
                }
                Token::Text { text: value, location } if current_token.is_mixed() => {
                    let tokens = self.split_mixed_token(value, *location);
                    self.tokenizer.insert_tokens_at_queue_start(tokens);
                }
                Token::Text { .. } if current_token.is_empty_or_white() => {
                    self.handle_in_body(&mut current_token);
                }
                Token::StartTag { name, .. } if name == "html" => {
                    self.handle_in_body(&mut current_token);
                }
                Token::Eof { .. } => {
                    self.stop_parsing();
                }
                _ => {
                    self.parse_error("anything else not allowed in after after body insertion mode");
                    self.insertion_mode = InsertionMode::InBody;
                    self.reprocess_token = true;
                }
            },
            InsertionMode::AfterAfterFrameset => {
                match &current_token {
                    Token::Comment { .. } => {
                        self.insert_comment_element(&current_token, Some(NodeId::root()));
                    }
                    Token::DocType { .. } => {
                        self.handle_in_body(&mut current_token);
                    }
                    Token::Text { text: value, location } if current_token.is_mixed() => {
                        let tokens = self.split_mixed_token(value, *location);
                        self.tokenizer.insert_tokens_at_queue_start(tokens);
                    }
                    Token::Text { .. } if current_token.is_empty_or_white() => {
                        self.handle_in_body(&mut current_token);
                    }
                    Token::StartTag { name, .. } if name == "html" => {
                        self.handle_in_body(&mut current_token);
                    }
                    Token::Eof { .. } => {
                        self.stop_parsing();
                    }
                    Token::StartTag { name, .. } if name == "noframes" => {
                        self.handle_in_head(&mut current_token);
                    }
                    _ => {
                        self.parse_error("anything else not allowed in after after frameset insertion mode");
                        // ignore token
                    }
                }
            }
        }

        // If this token is to be reprocessed, restore it so the next iteration re-reads
        // the real token rather than the placeholder we swapped in. When an arm installed
        // a replacement in place (the `<image>` -> `<img>` rewrite), that replacement must
        // survive instead, so we skip the restore in that case.
        if self.reprocess_token && !self.current_token_rewritten {
            self.current_token = current_token;
        }
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

    /// Returns true when the open elements have $name
    fn open_elements_has(&self, name: &str) -> bool {
        self.open_elements
            .iter()
            .rev()
            .any(|&node_id| self.document.tag_name(node_id).unwrap_or_default() == name)
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

    /// Reset insertion mode based on all kind of rules
    fn reset_insertion_mode(&mut self) {
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

    /// Pop all elements back to a table context
    fn clear_stack_back_to_table_context(&mut self) {
        while !self.open_elements.is_empty() {
            if ["table", "template", "html"].contains(&node_tag!(self, current_node_id!(self))) {
                return;
            }
            self.open_elements.pop();
        }
    }

    /// Pop all elements back to a table context
    fn clear_stack_back_to_table_body_context(&mut self) {
        while !self.open_elements.is_empty() {
            if ["tbody", "tfoot", "thead", "template", "html"].contains(&node_tag!(self, current_node_id!(self))) {
                return;
            }
            self.open_elements.pop();
        }
    }

    /// Pop all elements back to a table row context
    fn clear_stack_back_to_table_row_context(&mut self) {
        while !self.open_elements.is_empty() {
            let cn_id = current_node_id!(self);
            if ["tr", "template", "html"].contains(&node_tag!(self, cn_id)) {
                return;
            }
            self.open_elements.pop();
        }
    }

    /// Checks if the given element is in given scope
    fn is_in_scope(&self, tag: &str, namespace: &str, scope: Scope) -> bool {
        // Asked before every block start tag, so it is cached. See `OpenElements`.
        if matches!(scope, Scope::Button) && tag == "p" && namespace == HTML_NAMESPACE {
            return self
                .open_elements
                .has_p_in_button_scope(|node_id| self.scope_step(node_id, tag, namespace, scope));
        }
        self.open_elements
            .iter()
            .rev()
            .find_map(|&node_id| self.scope_step(node_id, tag, namespace, scope))
            .unwrap_or(false)
    }

    /// Whether `target` itself is in `scope`. Unlike `is_in_scope`, another element with the
    /// same name above it does not count.
    fn is_element_in_scope(&self, target: NodeId, scope: Scope) -> bool {
        for &node_id in self.open_elements.iter().rev() {
            if node_id == target {
                return true;
            }
            if self.ends_scope(node_id, scope) {
                return false;
            }
        }
        false
    }

    /// One step of the scope walk. `Some(true)` when `node_id` is the element looked for,
    /// `Some(false)` when it ends the scope, `None` to go on to the next entry down.
    fn scope_step(&self, node_id: NodeId, tag: &str, namespace: &str, scope: Scope) -> Option<bool> {
        if self.document.node_type(node_id) == NodeType::ElementNode
            && self.document.tag_name(node_id).unwrap_or_default() == tag
            && self.document.namespace(node_id).unwrap_or(HTML_NAMESPACE) == namespace
        {
            return Some(true);
        }
        self.ends_scope(node_id, scope).then_some(false)
    }

    /// Whether `node_id` is one of the elements that end `scope`.
    fn ends_scope(&self, node_id: NodeId, scope: Scope) -> bool {
        if self.document.node_type(node_id) != NodeType::ElementNode {
            return true;
        }

        let node_tag = self.document.tag_name(node_id).unwrap_or_default();
        let node_ns = self.document.namespace(node_id).unwrap_or(HTML_NAMESPACE);

        let default_html_scope = [
            "applet", "caption", "html", "table", "td", "th", "marquee", "object", "template",
        ];
        let default_mathml_scope = ["mo", "mi", "ms", "mn", "mtext", "annotation-xml"];
        let default_svg_scope = ["foreignObject", "desc", "title"];
        match scope {
            Scope::Regular => {
                if (node_ns == HTML_NAMESPACE && default_html_scope.contains(&node_tag))
                    || (node_ns == MATHML_NAMESPACE && default_mathml_scope.contains(&node_tag))
                    || (node_ns == SVG_NAMESPACE && default_svg_scope.contains(&node_tag))
                {
                    return true;
                }
            }
            Scope::ListItem => {
                if (node_ns == HTML_NAMESPACE
                    && (default_html_scope.contains(&node_tag) || ["ol", "ul"].contains(&node_tag)))
                    || (node_ns == MATHML_NAMESPACE && default_mathml_scope.contains(&node_tag))
                    || (node_ns == SVG_NAMESPACE && default_svg_scope.contains(&node_tag))
                {
                    return true;
                }
            }
            Scope::Button => {
                if (node_ns == HTML_NAMESPACE && (default_html_scope.contains(&node_tag) || node_tag == "button"))
                    || (node_ns == MATHML_NAMESPACE && default_mathml_scope.contains(&node_tag))
                    || (node_ns == SVG_NAMESPACE && default_svg_scope.contains(&node_tag))
                {
                    return true;
                }
            }
            Scope::Table => {
                if node_ns == HTML_NAMESPACE && ["html", "template", "table"].contains(&node_tag) {
                    return true;
                }
            }
            Scope::Select => {
                if !(node_ns == HTML_NAMESPACE && ["optgroup", "option"].contains(&node_tag)) {
                    return true;
                }
            }
        }
        false
    }

    /// Closes a table cell and switches the insertion mode to `InRow`
    fn close_cell(&mut self) {
        self.generate_implied_end_tags(None, false);

        let cn_id = current_node_id!(self);
        let tag = node_tag!(self, cn_id);

        if tag != "td" && tag != "th" {
            self.parse_error("current node should be td or th");
        }

        self.pop_until_any(&["td", "th"]);

        self.active_formatting_elements_clear_until_marker();
        self.insertion_mode = InsertionMode::InRow;
    }

    /// Handle insertion mode "`in_body`"
    fn handle_in_body(&mut self, token: &mut Token) {
        match &*token {
            Token::Text { text: value, location } if token.is_mixed_null() => {
                let tokens = self.split_mixed_token_null(value, *location);
                self.tokenizer.insert_tokens_at_queue_start(tokens);
            }
            Token::Text { text: value, location } if token.is_null() => {
                self.null_parse_errors(value, *location, "null character not allowed in in body insertion mode");
                // ignore token
            }
            Token::Text { .. } => {
                self.reconstruct_formatting();

                self.insert_text_element(token);

                // If this mixed token does not have whitespace chars, set frameset_ok to false
                if !token.is_empty_or_white() {
                    self.frameset_ok = false;
                }
            }
            Token::Comment { .. } => {
                self.insert_comment_element(token, None);
            }
            Token::DocType { .. } => {
                self.parse_error("doctype not allowed in in body insertion mode");
                // ignore token
            }
            Token::StartTag { name, attributes, .. } if name == "html" => {
                self.parse_error("html tag not allowed in in body insertion mode");

                if self.open_elements_has("template") {
                    // ignore token
                    return;
                }

                // Add attributes to html element
                let Some(&first_node_id) = self.open_elements.first() else {
                    return;
                };
                if self.document.node_type(first_node_id) == NodeType::ElementNode {
                    for (key, value) in attributes {
                        if self.document.attribute(first_node_id, key).is_none() {
                            self.document.set_attribute(first_node_id, key, value);
                        }
                    }
                }
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
            Token::StartTag { name, attributes, .. } if name == "body" => {
                self.parse_error("body tag not allowed in in body insertion mode");

                if self.open_elements.len() <= 1
                    || self.document.tag_name(self.open_elements[1]).unwrap_or_default() != "body"
                    || self.open_elements_has("template")
                {
                    // fragment case
                    // ignore token
                    return;
                }

                self.frameset_ok = false;

                let body_node_id = self.open_elements.iter().copied().find(|&node_id| {
                    self.document.tag_name(node_id).unwrap_or_default() == "body"
                        && self.document.namespace(node_id) == Some(HTML_NAMESPACE)
                });

                if let Some(body_node_id) = body_node_id {
                    if self.document.node_type(body_node_id) == NodeType::ElementNode {
                        for (key, value) in attributes {
                            if self.document.attribute(body_node_id, key).is_none() {
                                self.document.set_attribute(body_node_id, key, value);
                            }
                        }
                    }
                }
            }
            Token::StartTag { name, .. } if name == "frameset" => {
                self.parse_error("frameset tag not allowed in in body insertion mode");

                if self.open_elements.len() <= 1
                    || self.document.tag_name(self.open_elements[1]).unwrap_or_default() != "body"
                {
                    // ignore token
                    return;
                }

                if !self.frameset_ok {
                    // ignore token
                    return;
                }

                if self.open_elements.len() > 1 {
                    let second_node_id = self.open_elements[1];
                    if self.document.parent(second_node_id).is_some() {
                        self.document.detach(second_node_id);
                    }
                }

                while node_tag!(self, current_node_id!(self)) != "html" {
                    self.open_elements.pop();
                }

                self.insert_html_element(token);

                self.insertion_mode = InsertionMode::InFrameset;
            }
            Token::Eof { .. } => {
                if self.template_insertion_mode.is_empty() {
                    self.check_open_elements_at_body_end();
                    self.stop_parsing();
                } else {
                    self.handle_in_template(token);
                }
            }
            Token::EndTag { name, .. } if name == "body" => {
                if !self.is_in_scope("body", HTML_NAMESPACE, Scope::Regular) {
                    self.parse_error("body end tag not in scope");
                    // ignore token
                    return;
                }

                self.check_open_elements_at_body_end();
                self.insertion_mode = InsertionMode::AfterBody;
            }
            Token::EndTag { name, .. } if name == "html" => {
                if !self.is_in_scope("body", HTML_NAMESPACE, Scope::Regular) {
                    self.parse_error("body end tag not in scope");
                    // ignore token
                    return;
                }

                self.check_open_elements_at_body_end();
                self.insertion_mode = InsertionMode::AfterBody;
                self.reprocess_token = true;
            }
            Token::StartTag { name, attributes, .. }
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
                if self.is_in_scope("p", HTML_NAMESPACE, Scope::Button) {
                    self.close_p_element();
                }

                self.insert_html_element(token);
            }
            Token::StartTag { name, .. }
                if name == "h1" || name == "h2" || name == "h3" || name == "h4" || name == "h5" || name == "h6" =>
            {
                if self.is_in_scope("p", HTML_NAMESPACE, Scope::Button) {
                    self.close_p_element();
                }

                if ["h1", "h2", "h3", "h4", "h5", "h6"].contains(&node_tag!(self, current_node_id!(self))) {
                    self.parse_error("h1-h6 not allowed in in body insertion mode");
                    self.open_elements.pop();
                }

                self.insert_html_element(token);
            }
            Token::StartTag { name, .. } if name == "pre" || name == "listing" => {
                if self.is_in_scope("p", HTML_NAMESPACE, Scope::Button) {
                    self.close_p_element();
                }

                self.insert_html_element(token);

                self.ignore_lf = true;

                self.frameset_ok = false;
            }
            Token::StartTag { name, .. } if name == "form" => {
                if self.form_element.is_some() && !self.open_elements_has("template") {
                    self.parse_error("error with template, form shzzl");
                    // ignore token
                    return;
                }

                if self.is_in_scope("p", HTML_NAMESPACE, Scope::Button) {
                    self.close_p_element();
                }

                let node_id = self.insert_html_element(token);
                if !self.open_elements_has("template") {
                    self.form_element = Some(node_id);
                }
            }
            Token::StartTag { name, .. } if name == "li" => {
                self.frameset_ok = false;

                if self.open_elements.is_empty() {
                    return;
                }
                let mut idx = self.open_elements.len() - 1;
                loop {
                    let node_id = self.open_elements[idx];
                    let tag = self.document.tag_name(node_id).unwrap_or_default();

                    if tag == "li" {
                        self.generate_implied_end_tags(Some("li"), false);

                        if node_tag!(self, current_node_id!(self)) != "li" {
                            self.parse_error("li tag not at top of stack");
                        }

                        self.pop_until_named("li");
                        break;
                    }

                    if !["address", "div", "p"].contains(&tag) && is_special::<C>(self.document, node_id) {
                        break;
                    }

                    if idx == 0 {
                        break;
                    }
                    idx -= 1;
                }

                if self.is_in_scope("p", HTML_NAMESPACE, Scope::Button) {
                    self.close_p_element();
                }

                self.insert_html_element(token);
            }
            Token::StartTag { name, .. } if name == "dd" || name == "dt" => {
                self.frameset_ok = false;

                if self.open_elements.is_empty() {
                    return;
                }
                let mut idx = self.open_elements.len() - 1;
                loop {
                    let node_id = self.open_elements[idx];
                    let tag = self.document.tag_name(node_id).unwrap_or_default().to_owned();

                    if ["dd", "dt"].contains(&tag.as_str()) {
                        self.generate_implied_end_tags(Some(&tag), false);

                        if node_tag!(self, current_node_id!(self)) != tag.as_str() {
                            self.parse_error("{tag} tag not at top of stack");
                        }

                        self.pop_until_named(&tag);
                        break;
                    }

                    if !["address", "div", "p"].contains(&tag.as_str()) && is_special::<C>(self.document, node_id) {
                        break;
                    }

                    if idx == 0 {
                        break;
                    }
                    idx -= 1;
                }

                if self.is_in_scope("p", HTML_NAMESPACE, Scope::Button) {
                    self.close_p_element();
                }

                self.insert_html_element(token);
            }
            Token::StartTag { name, .. } if name == "plaintext" => {
                if self.is_in_scope("p", HTML_NAMESPACE, Scope::Button) {
                    self.close_p_element();
                }

                self.insert_html_element(token);

                self.tokenizer.state = State::PLAINTEXT;
            }
            Token::StartTag { name, .. } if name == "button" => {
                if self.is_in_scope("button", HTML_NAMESPACE, Scope::Regular) {
                    self.parse_error("button tag not allowed in in body insertion mode");
                    self.generate_implied_end_tags(None, false);
                    self.pop_until_named("button");
                }

                self.reconstruct_formatting();
                self.insert_html_element(token);
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
                if !self.is_in_scope(name, HTML_NAMESPACE, Scope::Regular) {
                    self.parse_error("end tag not in scope");
                    // ignore token
                    return;
                }

                self.generate_implied_end_tags(None, false);

                if node_tag!(self, current_node_id!(self)) != *name {
                    self.parse_error("end tag not at top of stack");
                }

                self.pop_until_named(name);
            }
            Token::EndTag { name, .. } if name == "form" => {
                if self.open_elements_has("template") {
                    if !self.is_in_scope(name, HTML_NAMESPACE, Scope::Regular) {
                        self.parse_error("end tag not in scope");
                        // ignore token
                        return;
                    }

                    self.generate_implied_end_tags(None, false);

                    if node_tag!(self, current_node_id!(self)) != *name {
                        self.parse_error("end tag not at top of stack");
                    }

                    self.pop_until_named(name);
                } else {
                    let node_id = self.form_element;
                    self.form_element = None;

                    // The pointer's own element, not any form: another form can be in scope
                    // while the pointer's form is not.
                    let Some(node_id) = node_id.filter(|&id| self.is_element_in_scope(id, Scope::Regular)) else {
                        self.parse_error("end tag not in scope");
                        // ignore token
                        return;
                    };

                    self.generate_implied_end_tags(None, false);

                    if node_tag!(self, current_node_id!(self)) != *name {
                        self.parse_error("end tag not at top of stack");
                    }

                    if node_id != current_node_id!(self) {
                        self.parse_error("end tag not at top of stack");
                    }
                    self.open_elements_remove(node_id);
                }
            }
            Token::EndTag { name, .. } if name == "p" => {
                if !self.is_in_scope(name, HTML_NAMESPACE, Scope::Button) {
                    self.parse_error("end tag not in scope");

                    let mut token = Token::StartTag {
                        name: "p".to_string(),
                        is_self_closing: false,
                        attributes: Attributes::new(),
                        location: token.get_location(),
                    };
                    self.insert_html_element(&mut token);
                }

                self.close_p_element();
            }
            Token::EndTag { name, .. } if name == "li" => {
                if !self.is_in_scope(name, HTML_NAMESPACE, Scope::ListItem) {
                    self.parse_error("end tag not in scope");
                    // ignore token
                    return;
                }

                self.generate_implied_end_tags(Some("li"), false);

                if node_tag!(self, current_node_id!(self)) != *name {
                    self.parse_error("end tag not at top of stack");
                }

                self.pop_until_named(name);
            }
            Token::EndTag { name, .. } if name == "dd" || name == "dt" => {
                if !self.is_in_scope(name, HTML_NAMESPACE, Scope::Regular) {
                    self.parse_error("end tag not in scope");
                    // ignore token
                    return;
                }

                self.generate_implied_end_tags(Some(name), false);

                if node_tag!(self, current_node_id!(self)) != *name {
                    self.parse_error("end tag not at top of stack");
                }

                self.pop_until_named(name);
            }
            Token::EndTag { name, .. }
                if name == "h1" || name == "h2" || name == "h3" || name == "h4" || name == "h5" || name == "h6" =>
            {
                if ["h1", "h2", "h3", "h4", "h5", "h6"]
                    .iter()
                    .any(|tag| self.is_in_scope(tag, HTML_NAMESPACE, Scope::Regular))
                {
                    self.generate_implied_end_tags(Some(name), false);

                    if node_tag!(self, current_node_id!(self)) != *name {
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
                    self.adoption_agency_algorithm(token);

                    // Remove from lists if not done already by the adoption agency
                    self.open_elements_remove(node_id);
                    self.active_formatting_elements_remove(node_id);
                }

                self.reconstruct_formatting();

                let node_id = self.insert_html_element(token);
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

                let node_id = self.insert_html_element(token);
                self.active_formatting_elements_push(node_id);
            }
            Token::StartTag { name, .. } if name == "nobr" => {
                self.reconstruct_formatting();

                if self.is_in_scope("nobr", HTML_NAMESPACE, Scope::Regular) {
                    self.parse_error("nobr tag in scope");
                    self.adoption_agency_algorithm(token);
                    self.reconstruct_formatting();
                }

                let node_id = self.insert_html_element(token);
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
                self.adoption_agency_algorithm(token);

                #[cfg(all(feature = "debug_parser", test))]
                self.display_debug_info();
            }
            Token::StartTag { name, .. } if name == "applet" || name == "marquee" || name == "object" => {
                self.reconstruct_formatting();

                self.insert_html_element(token);

                self.active_formatting_elements_push_marker();
                self.frameset_ok = false;
            }
            Token::EndTag { name, .. } if name == "applet" || name == "marquee" || name == "object" => {
                if !self.is_in_scope(name, HTML_NAMESPACE, Scope::Regular) {
                    self.parse_error("end tag not in scope");
                    // ignore token
                    return;
                }

                self.generate_implied_end_tags(None, false);

                if node_tag!(self, current_node_id!(self)) != *name {
                    self.parse_error("end tag not at top of stack");
                }

                self.pop_until_named(name);
                self.active_formatting_elements_clear_until_marker();
            }
            Token::StartTag { name, .. } if name == "table" => {
                if self.document.quirks_mode() != QuirksMode::Quirks
                    && self.is_in_scope("p", HTML_NAMESPACE, Scope::Button)
                {
                    self.close_p_element();
                }

                self.insert_html_element(token);

                self.frameset_ok = false;
                self.insertion_mode = InsertionMode::InTable;
            }
            Token::EndTag { name, .. } if name == "br" => {
                self.parse_error("br end tag not allowed");
                self.reconstruct_formatting();

                // Remove attributes if any
                let mut br = token.clone();
                if let Token::StartTag { attributes, .. } = &mut br {
                    attributes.clear();
                }

                self.insert_html_element(&mut br);

                self.open_elements.pop();
                self.frameset_ok = false;
            }
            Token::StartTag {
                name, is_self_closing, ..
            } if name == "area"
                || name == "br"
                || name == "embed"
                || name == "img"
                || name == "keygen"
                || name == "wbr" =>
            {
                let is_self_closing = *is_self_closing;
                self.reconstruct_formatting();

                self.insert_html_element(token);
                self.open_elements.pop();

                self.acknowledge_closing_tag(is_self_closing);
                self.frameset_ok = false;
            }
            Token::StartTag {
                name,
                is_self_closing,
                attributes,
                ..
            } if name == "input" => {
                let is_self_closing = *is_self_closing;
                // Read before the insert: the element takes the token's attributes.
                let not_hidden = attributes.get("type").is_none_or(|t| t.cow_to_lowercase() != *"hidden");
                self.reconstruct_formatting();

                self.insert_html_element(token);
                self.open_elements.pop();

                self.acknowledge_closing_tag(is_self_closing);

                if not_hidden {
                    self.frameset_ok = false;
                }
            }
            Token::StartTag {
                name, is_self_closing, ..
            } if name == "param" || name == "source" || name == "track" => {
                let is_self_closing = *is_self_closing;
                self.insert_html_element(token);
                self.open_elements.pop();

                self.acknowledge_closing_tag(is_self_closing);
            }
            Token::StartTag {
                name, is_self_closing, ..
            } if name == "hr" => {
                let is_self_closing = *is_self_closing;
                if self.is_in_scope("p", HTML_NAMESPACE, Scope::Button) {
                    self.close_p_element();
                }

                self.insert_html_element(token);
                self.open_elements.pop();

                self.acknowledge_closing_tag(is_self_closing);
                self.frameset_ok = false;
            }
            Token::StartTag {
                name,
                is_self_closing,
                attributes,
                ..
            } if name == "image" => {
                self.parse_error("image tag not allowed");
                self.current_token = Token::StartTag {
                    name: "img".to_string(),
                    attributes: attributes.clone(),
                    is_self_closing: *is_self_closing,
                    location: token.get_location(),
                };
                // We replaced current_token in place; keep it (not the token moved out for
                // matching) so the reprocess below sees the rewritten <img> tag.
                self.current_token_rewritten = true;
                self.reprocess_token = true;
            }
            Token::StartTag { name, .. } if name == "textarea" => {
                self.insert_html_element(token);

                self.ignore_lf = true;

                self.tokenizer.state = State::RCDATA;
                self.original_insertion_mode = self.insertion_mode;
                self.frameset_ok = false;
                self.insertion_mode = InsertionMode::Text;
            }
            Token::StartTag { name, .. } if name == "xmp" => {
                if self.is_in_scope("p", HTML_NAMESPACE, Scope::Button) {
                    self.close_p_element();
                }

                self.reconstruct_formatting();

                self.frameset_ok = false;
                self.parse_raw_data(token);
            }
            Token::StartTag { name, .. } if name == "iframe" => {
                self.frameset_ok = false;
                self.parse_raw_data(token);
            }
            Token::StartTag { name, .. } if name == "noembed" => {
                self.parse_raw_data(token);
            }
            Token::StartTag { name, .. } if name == "noscript" && self.scripting_enabled => {
                self.parse_raw_data(token);
            }
            Token::StartTag { name, .. } if name == "select" => {
                self.reconstruct_formatting();

                self.insert_html_element(token);
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
                if node_tag!(self, current_node_id!(self)) == "option" {
                    self.open_elements.pop();
                }

                self.reconstruct_formatting();

                self.insert_html_element(token);
            }
            Token::StartTag { name, .. } if name == "rb" || name == "rtc" => {
                if self.is_in_scope("ruby", HTML_NAMESPACE, Scope::Regular) {
                    self.generate_implied_end_tags(None, false);
                }

                if node_tag!(self, current_node_id!(self)) != "ruby" {
                    self.parse_error("rb or rtc not in scope");
                }

                self.insert_html_element(token);
            }
            Token::StartTag { name, .. } if name == "rp" || name == "rt" => {
                if self.is_in_scope("ruby", HTML_NAMESPACE, Scope::Regular) {
                    self.generate_implied_end_tags(Some("rtc"), false);
                }

                if node_tag!(self, current_node_id!(self)) != "rtc" && node_tag!(self, current_node_id!(self)) != "ruby"
                {
                    self.parse_error("rp or rt not in scope");
                }

                self.insert_html_element(token);
            }
            Token::StartTag {
                name, is_self_closing, ..
            } if name == "math" => {
                let is_self_closing = *is_self_closing;
                self.reconstruct_formatting();

                self.adjust_foreign_attributes(token, MATHML_NAMESPACE);

                self.insert_foreign_element(token, MATHML_NAMESPACE);

                if is_self_closing {
                    self.open_elements.pop();
                    self.acknowledge_closing_tag(is_self_closing);
                }
            }
            Token::StartTag {
                name, is_self_closing, ..
            } if name == "svg" => {
                let is_self_closing = *is_self_closing;
                self.reconstruct_formatting();

                self.adjust_foreign_attributes(token, SVG_NAMESPACE);
                self.insert_foreign_element(token, SVG_NAMESPACE);

                if is_self_closing {
                    self.open_elements.pop();
                    self.acknowledge_closing_tag(is_self_closing);
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
                self.insert_html_element(token);
            }
            Token::EndTag { name, .. } => {
                self.handle_in_body_any_other_end_tag(name);
            }
        }
    }

    /// Handle insertion mode "`in_head`"
    fn handle_in_head(&mut self, token: &mut Token) {
        let mut anything_else = false;

        match &*token {
            Token::Text { text: value, location } if token.is_mixed() => {
                let tokens = self.split_mixed_token(value, *location);
                self.tokenizer.insert_tokens_at_queue_start(tokens);
                return;
            }
            Token::Text { .. } if token.is_empty_or_white() => {
                self.insert_text_element(token);
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
            Token::StartTag {
                name,
                is_self_closing,
                attributes,
                ..
            } if name == "base" || name == "basefont" || name == "bgsound" || name == "link" => {
                if name == "link" {
                    // Handle link elements, as it depends on rel/itemprop attributes and other factors
                    self.handle_link_element(attributes);
                }

                self.acknowledge_closing_tag(*is_self_closing);

                self.insert_html_element(token);
                self.open_elements.pop();
            }
            Token::StartTag {
                name, is_self_closing, ..
            } if name == "meta" => {
                self.acknowledge_closing_tag(*is_self_closing);

                // Read before the insert: the element takes the token's attributes.
                let encoding = meta_charset_encoding(token);
                self.insert_html_element(token);
                self.open_elements.pop();

                // The speculative parser is not implemented, so we always proceed.
                // Change the encoding if the meta element declares one. The stream only
                // does so while its confidence is tentative and makes it certain after,
                // so later declarations are ignored (WHATWG HTML 13.2.3.4).
                if let Some(enc) = encoding {
                    self.tokenizer.stream.change_encoding(enc);
                }
            }
            Token::StartTag { name, .. } if name == "title" => {
                self.parse_rcdata(token);
            }
            Token::StartTag { name, .. } if name == "noscript" && self.scripting_enabled => {
                self.parse_raw_data(token);
            }
            Token::StartTag { name, .. } if name == "noframes" => {
                self.parse_raw_data(token);
            }
            Token::StartTag { name, .. } if name == "style" => {
                self.parse_raw_data(token);
            }

            Token::StartTag { name, .. } if name == "noscript" && !self.scripting_enabled => {
                self.insert_html_element(token);
                self.insertion_mode = InsertionMode::InHeadNoscript;
            }
            Token::StartTag { name, attributes, .. } if name == "script" => {
                // An inline script always blocks: `async` and `defer` say nothing about a
                // script with no `src` to fetch. One with a `src` blocks
                // only when it carries neither.
                self.current_script_blocks = match attributes.get("src") {
                    None => true,
                    Some(_) => !attributes.contains_key("async") && !attributes.contains_key("defer"),
                };

                let insert_position = self.appropriate_place_insert(None);
                let node_id = self.create_node(token, HTML_NAMESPACE);
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
            Token::StartTag {
                name,
                attributes,
                location,
                ..
            } if name == "template" => {
                self.active_formatting_elements_push_marker();
                self.frameset_ok = false;
                self.insertion_mode = InsertionMode::InTemplate;
                self.template_insertion_mode.push(InsertionMode::InTemplate);

                if let Some(shadow_root_id) = self.attach_declarative_shadow_root(attributes, *location) {
                    // The template declared a shadow root, so it never enters the tree itself -
                    // only the shadow root it created does. The element is still created and
                    // pushed onto the stack of open elements, because everything downstream keys
                    // off it: the `</template>` end tag, "in template" insertion mode, and the
                    // template-contents redirection in `appropriate_place_insert`. Pointing its
                    // template contents at the shadow root is what routes the children in.
                    let node_id = self.create_node(token, HTML_NAMESPACE);
                    self.open_elements.push(node_id);
                    self.document.set_template_contents(node_id, shadow_root_id);
                } else {
                    let node_id = self.insert_html_element(token);

                    // An ordinary template holds its own contents: the redirection above then
                    // lands back on the template element itself.
                    let cn_id = current_node_id!(self);
                    if self.document.node_type(node_id) == NodeType::ElementNode {
                        self.document.set_template_contents(node_id, cn_id);
                    }
                }
            }
            Token::EndTag { name, .. } if name == "template" => {
                if !self.open_elements_has("template") {
                    self.parse_error("could not find template tag in open element stack");
                    // ignore token
                    return;
                }

                self.generate_implied_end_tags(None, true);

                if node_tag!(self, current_node_id!(self)) != "template" {
                    self.parse_error("template end tag not at top of stack");
                }

                self.pop_until_named("template");
                self.active_formatting_elements_clear_until_marker();
                self.template_insertion_mode.pop();
                self.reset_insertion_mode();
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

    /// Handle insertion mode "`in_template`"
    fn handle_in_template(&mut self, token: &mut Token) {
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
                self.reset_insertion_mode();
                self.template_insertion_mode.pop();
                self.reprocess_token = true;
            }
        }
    }

    /// Handle insertion mode "`in_table`"
    fn handle_in_table(&mut self, token: &mut Token) {
        let mut anything_else = false;

        match &*token {
            Token::Text { .. }
                if ["table", "tbody", "template", "tfoot", "tr"]
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

    /// Handle insertion mode "`in_select`"
    fn handle_in_select(&mut self, token: &mut Token) {
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
                    if self.document.tag_name(node_id).unwrap_or_default() == tag {
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

    /// Remove the given `node_id` from the active formatting elements list. Will do nothing when the node is not found
    fn active_formatting_elements_remove(&mut self, target_node_id: NodeId) {
        self.active_formatting_elements.retain(|node_id| match node_id {
            ActiveElement::Node(node_id) => *node_id != target_node_id,
            ActiveElement::Marker => true,
        });
    }

    /// Push a node onto the active formatting stack, make sure only max 3 of them can be added (between markers)
    fn active_formatting_elements_push(&mut self, node_id: NodeId) {
        let mut matched = 0;
        let mut first_matched = None;

        for entry in self.active_formatting_elements.iter().rev() {
            match entry {
                ActiveElement::Marker => break,
                &ActiveElement::Node(id) => {
                    if matches_tag_and_attrs_without_order::<C>(self.document, node_id, id) {
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

        self.active_formatting_elements.push(ActiveElement::Node(node_id));
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

        if entry.node_id().is_some_and(|id| self.open_elements.contains(&id)) {
            return;
        }

        loop {
            // If it's a marker or in the stack of open elements, nothing to reconstruct.
            let entry = self.active_formatting_elements[entry_index];
            if let ActiveElement::Marker = entry {
                entry_index += 1;
                break;
            }

            if entry.node_id().is_some_and(|id| self.open_elements.contains(&id)) {
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
            let Some(node_id) = entry.node_id() else {
                // Marker found. This should not happen!
                break;
            };

            let new_node_id = self.insert_element_from_node(node_id, None);

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

        if node_tag!(self, current_node_id!(self)) != "p" {
            self.parse_error("p element not at top of stack");
        }

        self.pop_until_named("p");
    }

    /// Adjusts tag name in the given token for SVG
    fn adjust_svg_tag_names(&self, token: &mut Token) {
        if let Token::StartTag { name, .. } = token {
            if let Some(new_name) = SVG_ADJUSTMENTS_TAGS.get(name) {
                (*new_name).clone_into(name);
            }
        }
    }

    /// Adjusts the attribute names of a foreign start tag: the MathML or SVG adjustments
    /// for `namespace`, then the foreign (xlink, xml, xmlns) ones. The attributes are only
    /// rebuilt when a name changes.
    fn adjust_foreign_attributes(&self, token: &mut Token, namespace: &str) {
        let Token::StartTag { attributes, .. } = token else {
            return;
        };
        let table = if namespace == MATHML_NAMESPACE {
            Some(&MATHML_ADJUSTMENTS)
        } else if namespace == SVG_NAMESPACE {
            Some(&SVG_ADJUSTMENTS_ATTRIBUTES)
        } else {
            None
        };
        if attributes.keys().all(|name| !is_adjusted_attribute(table, name)) {
            return;
        }
        *attributes = std::mem::take(attributes)
            .into_iter()
            .map(|(name, value)| (adjusted_attribute_name(table, &name).unwrap_or(name), value))
            .collect();
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
    fn display_debug_info(&self) {
        println!("-----------------------------------------\n");
        println!("current token   : '{}'", self.current_token);
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
            if self.document.tag_name(node_id).unwrap_or_default() == tag_name {
                self.generate_implied_end_tags(Some(tag_name), false);

                // It might be possible that the last item is not our node_id. Emit parse error if so
                if current_node_id!(self) != node_id {
                    self.parse_error("end tag not at top of stack");
                }

                // Pop until we reach node_id
                while current_node_id!(self) != node_id {
                    self.open_elements.pop();
                }
                // Pop node_id as well
                self.open_elements.pop();

                break;
            }

            if is_special::<C>(self.document, node_id) {
                self.parse_error("special node");
                // ignore token
                return;
            }
        }
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

    /// Records the tree scope a freshly parsed stylesheet belongs to, then hands it to the
    /// document.
    ///
    /// `owner` is the `<style>` or `<link>` element the sheet came from. Its scope is the shadow
    /// root at the top of its ancestor chain, or `None` when that chain reaches the document -
    /// a shadow root has no parent, so the walk stops there by itself.
    fn add_stylesheet_for(&mut self, owner: NodeId, mut stylesheet: <C::CssSystem as CssSystem>::Stylesheet) {
        let mut root = owner;
        while let Some(parent) = self.document.parent(root) {
            root = parent;
        }
        let scope = (self.document.node_type(root) == NodeType::ShadowRootNode).then_some(root);

        C::CssSystem::set_stylesheet_scope(&mut stylesheet, scope);
        self.document.add_stylesheet(stylesheet);
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

    /// Finds the node where to place an unexpected html tag. This can only be done on a mathml
    /// insertion point, a `svg_html` insertion point, or at a regular html namespaced node.
    fn process_unexpected_html_tag(&mut self) {
        self.parse_error("process_unexpected_html_tag");

        loop {
            let cn_id = current_node_id!(self);
            if is_mathml_text_integration_point::<C>(self.document, cn_id)
                || is_html_integration_point::<C>(self.document, cn_id)
                || self.document.namespace(cn_id) == Some(HTML_NAMESPACE)
            {
                break;
            }
            self.open_elements.pop();
            if self.open_elements.is_empty() {
                return;
            }
        }

        // Process as HTML content
        self.process_html_content();
    }

    /// Find the correct tokenizer state when we are about to parse a fragment case
    fn find_initial_state_for_context(&self, context_node_id: NodeId) -> State {
        if self.document.namespace(context_node_id) != Some(HTML_NAMESPACE) {
            return State::Data;
        }

        match self.document.tag_name(context_node_id).unwrap_or_default() {
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
    fn initialize_fragment_case(&mut self, context_node_id: NodeId) {
        self.is_fragment_case = true;

        self.tokenizer
            .set_state(self.find_initial_state_for_context(context_node_id));

        self.context_node_id = Some(context_node_id);
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

    /// Wait for every stylesheet linked so far, and put it in the cascade.
    ///
    /// Called before a script that blocks. Everything still pending is a `<link>` before this
    /// script, since a later one has not been tokenised yet, so all of them block it.
    ///
    /// Does nothing without a [`StylesheetSource`]: the sheets stay pending and whoever
    /// drives the parse resolves them afterwards, which is what happens for a document with
    /// no blocking scripts in it.
    /// Runs the script element whose end tag was just seen, if it is one this parser runs
    /// now: a classic script, inline or blocking, in a document whose scripts run. A `defer`
    /// or `async` script is kept for the end of the parse. Returns whether a script ran.
    ///
    /// A script that fails does not stop the parse; the host reports its own errors.
    fn run_script(&mut self, script: NodeId) -> bool {
        if self.script_host.is_none() || !self.scripting_enabled || self.is_fragment_case {
            return false;
        }
        // the contents of a template belong to a document that runs nothing
        if self.open_elements.iter().any(|&id| node_tag!(self, id) == "template") {
            return false;
        }

        let mut spent = Timer::new(None);
        let ran = self.run_classic_script(script);
        spent.end();
        self.script_us += spent.duration();

        ran
    }

    fn run_classic_script(&mut self, script: NodeId) -> bool {
        match self.classic_script(script) {
            Some(Script::Inline(source)) => {
                // The tokenizer is just past the end tag; the source starts as many lines
                // up as it has line breaks.
                let end_line = u32::try_from(self.tokenizer.get_location().line).unwrap_or(0);
                let line_breaks = u32::try_from(source.matches('\n').count()).unwrap_or(0);
                let origin = ScriptOrigin {
                    url: self.document.url().map(|url| url.to_string()).unwrap_or_default(),
                    line: end_line.saturating_sub(1).saturating_sub(line_breaks),
                };
                self.execute_script(&source, &origin)
            }
            Some(Script::External { url, deferred: false }) => self.run_external_script(&url),
            Some(Script::External { url, deferred: true }) => {
                self.deferred_scripts.push(url);
                false
            }
            None => false,
        }
    }

    /// Fetches and runs an external script. One that cannot be loaded is skipped.
    fn run_external_script(&mut self, url: &str) -> bool {
        let Some(source) = self
            .script_source
            .as_ref()
            .and_then(|source| source.fetch_blocking(url))
        else {
            warn!("Could not load script from {url}");
            return false;
        };

        let origin = ScriptOrigin {
            url: url.to_owned(),
            line: 0,
        };
        self.execute_script(&source, &origin)
    }

    fn execute_script(&mut self, source: &str, origin: &ScriptOrigin) -> bool {
        let Some(host) = self.script_host.as_mut() else {
            return false;
        };
        if let Err(e) = host.execute(source, origin) {
            log::debug!("script in {} failed: {e}", origin.url);
        }

        true
    }

    /// Runs the `defer` and `async` scripts, in document order, now that the document has
    /// been parsed. `async` scripts do not run earlier than this yet.
    fn run_deferred_scripts(&mut self) {
        // they wait for the stylesheets of the document, as a blocking script does
        if !self.deferred_scripts.is_empty() {
            self.settle_stylesheets();
        }
        for url in std::mem::take(&mut self.deferred_scripts) {
            if self.run_external_script(&url) {
                self.script_checkpoint();
            }
        }
    }

    /// The promise jobs a script queued run once no script is on the stack.
    fn script_checkpoint(&mut self) {
        if let Some(host) = self.script_host.as_mut() {
            if let Err(e) = host.microtask_checkpoint() {
                log::debug!("promise job failed: {e}");
            }
        }
    }

    /// What there is to run for a script element: `None` when it is not a classic script,
    /// has nothing in it, or is external with nowhere to get it from.
    fn classic_script(&self, script: NodeId) -> Option<Script> {
        let attributes = self.document.attributes(script);
        let attribute = |name: &str| attributes.and_then(|a| a.get(name)).map(String::as_str);

        if !is_classic_script_type(attribute("type"), attribute("language")) {
            return None;
        }
        // an SVG script names its file with `href`; those are not loaded yet
        let in_svg = node_ns!(self, script) == Some(SVG_NAMESPACE);
        if in_svg && (attribute("href").is_some() || attribute("xlink:href").is_some()) {
            return None;
        }

        if let Some(src) = attribute("src").filter(|_| !in_svg) {
            self.script_source.as_ref()?;
            if src.is_empty() {
                return None;
            }
            // resolved the way a stylesheet link is
            let url = match Url::parse(src) {
                Ok(url) => url,
                Err(_) => self.document.url()?.join(src).ok()?,
            };

            return Some(Script::External {
                url: url.to_string(),
                deferred: attribute("defer").is_some() || attribute("async").is_some(),
            });
        }

        let source: String = self
            .document
            .children(script)
            .iter()
            .filter_map(|&child| self.document.text_value(child))
            .collect();

        (!source.is_empty()).then_some(Script::Inline(source))
    }

    fn settle_stylesheets(&mut self) {
        let Some(source) = self.stylesheet_source.clone() else {
            return;
        };
        let pending = self.document.take_pending_stylesheets();
        if pending.is_empty() {
            return;
        }

        let urls: Vec<String> = pending.iter().map(|sheet| sheet.url.clone()).collect();
        let mut waited = Timer::new(Some(format!("{} stylesheet(s)", urls.len())));
        let bodies = source.fetch_blocking(&urls);
        waited.end();

        // What a script waiting on CSS actually cost, which is the number worth having when
        // a page loads slowly and nobody can say why.
        timing::record(
            gosub_shared::timing::Timing::ScriptBlockedOnCss,
            waited.duration(),
            Some(urls.join(" ")),
        );
        self.blocked_on_css_us
            .set(self.blocked_on_css_us.get() + waited.duration());
        log::debug!(
            "script blocked on {} stylesheet(s) for {:.1}ms",
            urls.len(),
            waited.duration() as f64 / 1000.0
        );

        let mut inserted = 0usize;
        for (PendingStylesheet { position, url, media }, body) in pending.into_iter().zip(bodies) {
            let Some(bytes) = body else {
                warn!("Could not load external stylesheet from {url}");
                continue;
            };
            let Ok(css) = String::from_utf8(bytes) else {
                warn!("External stylesheet from {url} is not valid UTF-8");
                continue;
            };
            let config = ParserConfig {
                source: Some(url.clone()),
                ignore_errors: true,
                ..Default::default()
            };
            match C::CssSystem::parse_str(&css, config, CssOrigin::Author, &url) {
                // Each sheet already placed shifts this one along by one; see
                // `Document::add_pending_stylesheet` for what the position means.
                Ok(mut sheet) => {
                    if let Some(media) = &media {
                        C::CssSystem::set_stylesheet_media(&mut sheet, media);
                    }
                    self.document.insert_stylesheet(position + inserted, sheet);
                    inserted += 1;
                }
                Err(err) => warn!("Error while parsing CSS stylesheet from {url}: {err}"),
            }
        }
    }

    /// Load an inline stylesheet from the <style>-node (identified by NodeId)
    fn load_inline_stylesheet(
        &self,
        origin: CssOrigin,
        node_id: NodeId,
    ) -> Option<<C::CssSystem as CssSystem>::Stylesheet> {
        // Must be a text node
        let text = self.document.text_value(node_id)?;

        let source_url = match self.document.url() {
            Some(url) => format!("{url}#inline"),
            None => "<unknown>#inline".into(),
        };

        let config = ParserConfig {
            context: Context::Stylesheet,
            location: Default::default(),
            source: Some(source_url.clone()),
            ignore_errors: true,
            match_values: false,
        };

        match C::CssSystem::parse_str(text, config, origin, &source_url) {
            Ok(mut stylesheet) => {
                self.resolve_stylesheet_imports(&mut stylesheet);
                Some(stylesheet)
            }
            Err(err) => {
                warn!("Error while parsing CSS stylesheet: {err} ");
                None
            }
        }
    }

    /// The text of a stylesheet an `@import` asked for.
    ///
    /// Asks the [`StylesheetSource`], the same one a `<link>` goes through, so an import is
    /// fetched under the same policy as any other subresource. Without a source the import
    /// stays unresolved.
    ///
    /// Blocks, like [`Self::settle_stylesheets`]. The wait is added to `blocked_on_css_us` so
    /// `parse_document` can take it out of `decode.html`.
    fn fetch_css_text(&self, url: &Url) -> Option<String> {
        let source = self.stylesheet_source.clone()?;

        let mut waited = Timer::new(Some(url.to_string()));
        let body = source.fetch_blocking(&[url.to_string()]).pop().flatten();
        waited.end();
        self.blocked_on_css_us
            .set(self.blocked_on_css_us.get() + waited.duration());

        match body {
            Some(bytes) => match String::from_utf8(bytes) {
                Ok(css) => Some(css),
                Err(err) => {
                    warn!("Imported stylesheet from {url} is not valid UTF-8: {err}");
                    None
                }
            },
            None => {
                warn!("Could not load imported stylesheet from {url}");
                None
            }
        }
    }

    /// Pull in every `@import` the stylesheet declares.
    ///
    /// Each import is another round trip the parse waits on, and they are sequential, so a
    /// chain costs its full depth in latency. Handing imports to the resource pipeline the
    /// way `<link>` is handed to it is the fix; until then an unresolved `@import` silently
    /// loses a whole stylesheet, which is worse.
    fn resolve_stylesheet_imports(&self, sheet: &mut <C::CssSystem as CssSystem>::Stylesheet) {
        let mut fetch = |base: &str, requested: &str| {
            // A relative import resolves against the importing sheet, not the document.
            let base_url = Url::parse(base).ok();
            let resolved = base_url
                .as_ref()
                .and_then(|base| base.join(requested).ok())
                .or_else(|| Url::parse(requested).ok())?;

            if !import_scheme_allowed(base_url.as_ref(), &resolved) {
                warn!(
                    "Refusing '{}' import from remote stylesheet {}: only http(s) is allowed",
                    resolved.scheme(),
                    base
                );
                return None;
            }

            let text = self.fetch_css_text(&resolved)?;
            Some((resolved.to_string(), text))
        };
        C::CssSystem::resolve_imports(sheet, &mut fetch);
    }

    /// What a `<link>` asks of the document: today only a stylesheet does anything. What it
    /// does not do is logged, not reported as a parse error: none of it is one (the tree
    /// builder's rules for `<link>` have no error case), and a page's report should hold
    /// only what the HTML spec calls an error.
    fn handle_link_element(&mut self, attributes: &Attributes) {
        if attributes.contains_key("rel") && attributes.contains_key("itemprop") {
            // cannot have them both
            log::debug!("link element cannot have both 'rel' and 'itemprop' attributes");
            return;
        }

        if attributes.contains_key("itemprop") {
            log::debug!("link element with 'itemprop' attribute not supported yet");
            return;
        }

        let Some(rel) = attributes.get("rel").cloned() else {
            log::debug!("link element without 'rel' attribute not supported yet");
            return;
        };

        // @todo: We need to check if the link rel is body-ok
        let parser_in_body = true;
        let body_ok_types = [
            "dns-prefetch",
            "modulepreload",
            "pingback",
            "preconnect",
            "prefetch",
            "preload",
            "prerender",
            "stylesheet",
        ];
        if parser_in_body && !body_ok_types.contains(&rel.as_str()) {
            log::debug!("link element with rel attribute '{rel}' is not supported in the body");
            return;
        }

        match rel.as_str() {
            "stylesheet" => {
                // Stylesheet link may not have a href link (for instance, it could be data-href for optional loading through JS)
                let Some(href) = attributes.get("href") else {
                    return;
                };

                let css_url = match Url::parse(href) {
                    Ok(url) => url,
                    Err(_err) => {
                        // Relative URL
                        let Some(base_url) = self.document.url() else {
                            log::debug!("link element without base url not supported yet");
                            return;
                        };
                        match base_url.join(href) {
                            Ok(url) => url,
                            Err(_) => {
                                log::debug!("link element with invalid href url");
                                return;
                            }
                        }
                    }
                };
                // Recorded, not fetched. The parser has no business opening sockets: the
                // resource pipeline is already fetching this sheet (and everything else the
                // document links), and it fills the slot in once the bytes are here.
                let media = attributes.get("media").map(String::as_str);
                self.document.add_pending_stylesheet(css_url.as_str(), media);
            }
            _ => {
                log::debug!("link element with rel attribute '{rel}' is not supported");
            }
        }
    }
}

#[cfg(test)]
mod import_scheme_tests {
    use super::import_scheme_allowed;
    use url::Url;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    /// The one that matters: a sheet served over the network must not be able to name a local
    /// file. `Url::join` hands back the absolute `file:` URL, and the fetch path would read it.
    #[test]
    fn a_remote_sheet_cannot_import_a_local_file() {
        let base = url("https://example.org/a.css");
        assert!(!import_scheme_allowed(Some(&base), &url("file:///etc/passwd")));

        let base = url("http://example.org/a.css");
        assert!(!import_scheme_allowed(Some(&base), &url("file:///etc/passwd")));
    }

    #[test]
    fn a_remote_sheet_may_import_over_the_network() {
        let base = url("https://example.org/a.css");
        assert!(import_scheme_allowed(
            Some(&base),
            &url("https://cdn.example.org/b.css")
        ));
        assert!(import_scheme_allowed(Some(&base), &url("http://cdn.example.org/b.css")));
    }

    /// A local sheet keeps working: it can already read the filesystem it came from.
    #[test]
    fn a_local_sheet_is_unrestricted() {
        let base = url("file:///home/u/site/a.css");
        assert!(import_scheme_allowed(Some(&base), &url("file:///home/u/site/b.css")));
        assert!(import_scheme_allowed(
            Some(&base),
            &url("https://cdn.example.org/b.css")
        ));
    }

    /// Remote is anything not read from disk: a page served over gopher, say.
    #[test]
    fn a_sheet_from_any_other_scheme_is_remote() {
        let base = url("gopher://example.org/0/a.css");
        assert!(!import_scheme_allowed(Some(&base), &url("file:///etc/passwd")));
        assert!(import_scheme_allowed(
            Some(&base),
            &url("https://cdn.example.org/b.css")
        ));
    }

    /// An unparseable base is not known to be remote, so it is not treated as one.
    #[test]
    fn an_unknown_base_is_not_treated_as_remote() {
        assert!(import_scheme_allowed(None, &url("file:///etc/passwd")));
    }

    /// Other non-network schemes are refused from a remote sheet too, not just `file:`.
    #[test]
    fn other_non_network_schemes_are_refused_from_remote() {
        let base = url("https://example.org/a.css");
        assert!(!import_scheme_allowed(Some(&base), &url("data:text/css,body{}")));
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
mod owner_media_tests {
    use crate::document::document_impl::DocumentImpl;
    use crate::parser::Html5Parser;
    use gosub_css3::media_query::MediaEnvironment;
    use gosub_css3::system::Css3System;
    use gosub_interface::config::ModuleConfiguration;
    use gosub_interface::document::{Document, PendingStylesheet};

    #[derive(Clone, Debug, PartialEq)]
    struct Config;
    impl ModuleConfiguration for Config {
        type CssSystem = Css3System;
        type Document = DocumentImpl<Self>;
        type HtmlParser = Html5Parser<'static, Self>;
    }

    #[test]
    fn a_style_element_keeps_its_media_attribute() {
        let doc = crate::html_compile::<Config>(
            "<head><style media=print>p { color: red; }</style><style>p { color: blue; }</style></head>",
        );
        let screen = MediaEnvironment::default();
        let [print, plain] = doc.stylesheets() else {
            panic!("two sheets");
        };
        assert!(!print.rules[0].media_matches(&screen));
        assert!(plain.rules[0].media.is_none());
    }

    #[test]
    fn a_link_hands_its_media_attribute_to_whoever_fetches_it() {
        let mut doc = crate::html_compile::<Config>(
            r#"<head><base href="https://example.org/">
               <link rel=stylesheet href="https://example.org/p.css" media="print">
               <link rel=stylesheet href="https://example.org/a.css"></head>"#,
        );
        let pending = doc.take_pending_stylesheets();
        let media: Vec<_> = pending
            .iter()
            .map(|PendingStylesheet { media, .. }| media.as_deref())
            .collect();
        assert_eq!(media, [Some("print"), None]);
    }
}

#[cfg(test)]
mod foreign_content_tests {
    use crate::document::document_impl::DocumentImpl;
    use crate::node::{HTML_NAMESPACE, MATHML_NAMESPACE, SVG_NAMESPACE};
    use crate::parser::Html5Parser;
    use gosub_css3::system::Css3System;
    use gosub_interface::config::ModuleConfiguration;
    use gosub_interface::document::Document;
    use gosub_shared::node::NodeId;

    #[derive(Clone, Debug, PartialEq)]
    struct Config;
    impl ModuleConfiguration for Config {
        type CssSystem = Css3System;
        type Document = DocumentImpl<Self>;
        type HtmlParser = Html5Parser<'static, Self>;
    }

    /// First element with this tag name, depth first.
    fn find(doc: &DocumentImpl<Config>, id: NodeId, tag: &str) -> Option<NodeId> {
        if doc.tag_name(id) == Some(tag) {
            return Some(id);
        }
        doc.children(id).iter().find_map(|&c| find(doc, c, tag))
    }

    #[test]
    fn foreign_start_tags_get_adjusted_names_and_attributes() {
        let doc = crate::html_compile::<Config>(
            r##"<svg viewbox="0 0 1 1"><foreignobject></foreignobject><path xlink:href="#a"/></svg>
                <math definitionurl="u"></math>"##,
        );
        let svg = find(&doc, NodeId::root(), "svg").unwrap();
        assert_eq!(doc.namespace(svg), Some(SVG_NAMESPACE));
        assert_eq!(doc.attribute(svg, "viewBox"), Some("0 0 1 1"));
        assert!(find(&doc, svg, "foreignObject").is_some());
        let path = find(&doc, svg, "path").unwrap();
        assert_eq!(doc.attribute(path, "xlink href"), Some("#a"));

        let math = find(&doc, NodeId::root(), "math").unwrap();
        assert_eq!(doc.namespace(math), Some(MATHML_NAMESPACE));
        assert_eq!(doc.attribute(math, "definitionURL"), Some("u"));
    }

    /// Names of the attributes of `id`, in document order.
    fn attribute_names(doc: &DocumentImpl<Config>, id: NodeId) -> Vec<String> {
        doc.attributes(id).unwrap().keys().cloned().collect()
    }

    /// Renaming keeps the attributes in source order, in every place foreign attributes are
    /// adjusted: `<svg>` and `<math>` in body, and start tags inside foreign content.
    #[test]
    fn adjusted_foreign_attributes_keep_their_order() {
        let doc = crate::html_compile::<Config>(
            r##"<svg a="1" viewbox="v" xlink:href="#h" b="2"><path d="x" xml:lang="en" pathlength="3"/>
                <foreignObject><math c="3" definitionurl="u" xlink:show="s"></math></foreignObject></svg>
                <math definitionurl="u" z="1" xlink:href="#m"><mi viewbox="v" definitionurl="d"/></math>"##,
        );
        let svg = find(&doc, NodeId::root(), "svg").unwrap();
        assert_eq!(attribute_names(&doc, svg), ["a", "viewBox", "xlink href", "b"]);
        let path = find(&doc, svg, "path").unwrap();
        assert_eq!(attribute_names(&doc, path), ["d", "xml lang", "pathLength"]);

        let math_in_svg = find(&doc, svg, "math").unwrap();
        assert_eq!(attribute_names(&doc, math_in_svg), ["c", "definitionURL", "xlink show"]);

        let body = find(&doc, NodeId::root(), "body").unwrap();
        let math = doc
            .children(body)
            .iter()
            .copied()
            .find(|&c| doc.tag_name(c) == Some("math"))
            .unwrap();
        assert_eq!(attribute_names(&doc, math), ["definitionURL", "z", "xlink href"]);
        let mi = find(&doc, math, "mi").unwrap();
        assert_eq!(attribute_names(&doc, mi), ["viewbox", "definitionURL"]);
    }

    /// Tokens handed from foreign content to HTML processing must be the real token.
    #[test]
    fn tokens_handed_back_to_html_keep_their_contents() {
        let doc = crate::html_compile::<Config>(
            r#"<svg><b class="x">t</b></svg><svg><font color="red"></font></svg><div><svg><g></div>after"#,
        );
        let b = find(&doc, NodeId::root(), "b").unwrap();
        assert_eq!(doc.namespace(b), Some(HTML_NAMESPACE));
        assert_eq!(doc.attribute(b, "class"), Some("x"));

        let font = find(&doc, NodeId::root(), "font").unwrap();
        assert_eq!(doc.namespace(font), Some(HTML_NAMESPACE));
        assert_eq!(doc.attribute(font, "color"), Some("red"));

        let body = find(&doc, NodeId::root(), "body").unwrap();
        let last = *doc.children(body).last().unwrap();
        assert_eq!(doc.text_value(last), Some("after"));
    }

    /// Text of the direct text children of `id`, joined.
    fn text_of(doc: &DocumentImpl<Config>, id: NodeId) -> String {
        doc.children(id).iter().filter_map(|&c| doc.text_value(c)).collect()
    }

    /// Every NUL in foreign content becomes its own U+FFFD, not one per run.
    #[test]
    fn each_null_in_foreign_content_becomes_a_replacement_character() {
        let cases = [
            ("<svg>a\0b", "svg", "a\u{FFFD}b"),
            ("<svg>a\0\0b", "svg", "a\u{FFFD}\u{FFFD}b"),
            ("<svg>a\0\0\0b", "svg", "a\u{FFFD}\u{FFFD}\u{FFFD}b"),
            ("<svg>\0\0a", "svg", "\u{FFFD}\u{FFFD}a"),
            ("<svg>a\0\0", "svg", "a\u{FFFD}\u{FFFD}"),
            ("<svg>\0\0", "svg", "\u{FFFD}\u{FFFD}"),
            ("<svg>\0\0 \0\0", "svg", "\u{FFFD}\u{FFFD} \u{FFFD}\u{FFFD}"),
            ("<svg> \0\0 a", "svg", " \u{FFFD}\u{FFFD} a"),
            ("<svg><![CDATA[a\0\0b]]>", "svg", "a\u{FFFD}\u{FFFD}b"),
            ("<math>a\0\0b", "math", "a\u{FFFD}\u{FFFD}b"),
            ("<math>\0\0 \0", "math", "\u{FFFD}\u{FFFD} \u{FFFD}"),
            ("<svg><g>a\0\0b</g>", "g", "a\u{FFFD}\u{FFFD}b"),
        ];
        for (input, tag, expected) in cases {
            let doc = crate::html_compile::<Config>(input);
            let id = find(&doc, NodeId::root(), tag).unwrap();
            assert_eq!(text_of(&doc, id), expected, "input {input:?}");
        }
    }

    /// Text in an integration point is HTML content, where the in body rules drop NULs.
    #[test]
    fn nulls_in_integration_points_are_dropped() {
        let cases = [
            ("<math><mi>a\0\0b", "mi", "ab"),
            ("<math><mi>\0", "mi", ""),
            ("<svg><foreignObject>a\0\0b", "foreignObject", "ab"),
            ("<svg><desc>a\0\0\0b", "desc", "ab"),
        ];
        for (input, tag, expected) in cases {
            let doc = crate::html_compile::<Config>(input);
            let id = find(&doc, NodeId::root(), tag).unwrap();
            assert_eq!(text_of(&doc, id), expected, "input {input:?}");
        }
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

#[cfg(test)]
mod script_tests {
    use std::sync::Arc;

    use super::*;
    use crate::document::builder::DocumentBuilderImpl;
    use crate::document::document_impl::DocumentImpl;
    use gosub_css3::system::Css3System;
    use gosub_interface::config::HasCssSystem;
    use gosub_interface::script::{ScriptError, ScriptErrorKind, ScriptTerminator, ScriptValue};
    use parking_lot::Mutex;

    #[derive(Clone, Debug, PartialEq)]
    struct Config;

    impl HasCssSystem for Config {
        type CssSystem = Css3System;
    }
    impl HasDocument for Config {
        type Document = DocumentImpl<Self>;
    }

    #[derive(Clone, Debug, PartialEq)]
    enum Call {
        Execute {
            source: String,
            url: String,
            line: u32,
        },
        Checkpoint,
        /// The stylesheets the parser asked for.
        Sheets(Vec<String>),
    }

    /// Records what the parser asks of it. A source that starts with `throw` fails.
    struct Recorder(Arc<Mutex<Vec<Call>>>);

    impl ScriptHost for Recorder {
        fn execute(&mut self, source: &str, origin: &ScriptOrigin) -> std::result::Result<ScriptValue, ScriptError> {
            self.0.lock().push(Call::Execute {
                source: source.to_owned(),
                url: origin.url.clone(),
                line: origin.line,
            });
            if source.starts_with("throw") {
                return Err(ScriptError::new(ScriptErrorKind::Exception, "thrown"));
            }
            Ok(ScriptValue::Undefined)
        }

        fn microtask_checkpoint(&mut self) -> std::result::Result<(), ScriptError> {
            self.0.lock().push(Call::Checkpoint);
            Ok(())
        }

        fn terminator(&self) -> ScriptTerminator {
            ScriptTerminator::noop()
        }
    }

    fn calls_with(html: &str, scripting_enabled: bool) -> Vec<Call> {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let options = Html5ParserOptions {
            scripting_enabled,
            stylesheets: None,
            scripts: Some(Box::new(Recorder(Arc::clone(&calls)))),
            script_sources: None,
        };

        let mut stream = ByteStream::from_str(html, Encoding::UTF8);
        let url = Url::parse("https://example.test/page.html").unwrap();
        let mut doc = DocumentBuilderImpl::new_document::<Config>(Some(url));
        let _ = Html5Parser::<Config>::parse_document(&mut stream, &mut doc, Some(options));

        let calls = calls.lock().clone();
        calls
    }

    /// The sources the parser ran for `html`, in order.
    fn sources(html: &str) -> Vec<String> {
        calls_with(html, true)
            .into_iter()
            .filter_map(|call| match call {
                Call::Execute { source, .. } => Some(source),
                _ => None,
            })
            .collect()
    }

    fn execute(source: &str, line: u32) -> Call {
        Call::Execute {
            source: source.to_owned(),
            url: "https://example.test/page.html".to_owned(),
            line,
        }
    }

    #[test]
    fn inline_scripts_run_in_document_order_each_followed_by_a_checkpoint() {
        let html = "<html><head><script>first()</script></head><body><p>x</p><script>second()</script></body></html>";

        assert_eq!(
            calls_with(html, true),
            [
                execute("first()", 0),
                Call::Checkpoint,
                execute("second()", 0),
                Call::Checkpoint
            ]
        );
    }

    #[test]
    fn a_failing_script_does_not_stop_the_next() {
        let html = "<script>throw 1</script><script>after()</script>";

        assert_eq!(
            calls_with(html, true),
            [
                execute("throw 1", 0),
                Call::Checkpoint,
                execute("after()", 0),
                Call::Checkpoint
            ]
        );
    }

    #[test]
    fn the_origin_has_the_line_the_source_starts_on() {
        let html = "<html>\n<body>\n<script>\na()\nb()\n</script>\n<script>c()</script>";

        assert_eq!(
            calls_with(html, true),
            [
                execute("\na()\nb()\n", 2),
                Call::Checkpoint,
                execute("c()", 6),
                Call::Checkpoint
            ]
        );
    }

    #[test]
    fn a_script_in_svg_runs() {
        assert_eq!(sources("<svg><script>inSvg()</script></svg>"), ["inSvg()"]);
        assert_eq!(
            sources("<svg><script href=\"a.js\">no()</script></svg>"),
            [] as [&str; 0]
        );
        assert_eq!(
            sources("<svg><script type=\"text/plain\">no()</script></svg>"),
            [] as [&str; 0]
        );
    }

    #[test]
    fn only_javascript_types_run() {
        let run = |attrs: &str| !sources(&format!("<script {attrs}>x()</script>")).is_empty();

        for attrs in [
            "",
            "type=\"\"",
            "type=\"text/javascript\"",
            "type=\" TEXT/JavaScript \"",
            "type=\"application/javascript\"",
            "type=\"application/ecmascript\"",
            "type=\"text/jscript\"",
            "language=\"\"",
            "language=\"JavaScript\"",
            "language=\"javascript1.5\"",
            // the type decides when both are there
            "type=\"text/javascript\" language=\"vbscript\"",
            // no module support, so nomodule scripts are for this parser
            "nomodule",
        ] {
            assert!(run(attrs), "<script {attrs}> did not run");
        }

        for attrs in [
            "type=\"module\"",
            "type=\"text/plain\"",
            "type=\"application/json\"",
            "type=\"importmap\"",
            "type=\"text/javascript; charset=utf-8\"",
            "type=\"javascript\"",
            "language=\"vbscript\"",
            "language=\" javascript\"",
            "type=\"text/plain\" language=\"javascript\"",
        ] {
            assert!(!run(attrs), "<script {attrs}> ran");
        }
    }

    #[test]
    fn external_and_empty_scripts_are_not_run_here() {
        assert_eq!(sources("<script src=\"a.js\">inline()</script>"), [] as [&str; 0]);
        assert_eq!(sources("<script src=\"\">inline()</script>"), [] as [&str; 0]);
        assert_eq!(sources("<script></script>"), [] as [&str; 0]);
    }

    #[test]
    fn a_script_in_a_template_does_not_run() {
        assert_eq!(
            sources("<template><script>inTemplate()</script></template><script>outside()</script>"),
            ["outside()"]
        );
        assert_eq!(
            sources("<template><div><script>nested()</script></div></template>"),
            [] as [&str; 0]
        );
    }

    #[test]
    fn nothing_runs_with_scripting_disabled() {
        assert_eq!(calls_with("<script>x()</script>", false), []);
    }

    #[test]
    fn nothing_runs_in_a_fragment() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let options = Html5ParserOptions {
            scripting_enabled: true,
            stylesheets: None,
            scripts: Some(Box::new(Recorder(Arc::clone(&calls)))),
            script_sources: None,
        };

        // the way innerHTML parses: a fresh document, with the context element made in it
        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let context = doc.create_element("div", Some(HTML_NAMESPACE), Attributes::new(), Location::default());
        let mut fragment = ByteStream::from_str("<script>inFragment()</script>", Encoding::UTF8);
        let _ =
            Html5Parser::<Config>::parse_fragment(&mut fragment, &mut doc, context, Some(options), Location::default());

        assert_eq!(*calls.lock(), []);
    }

    #[test]
    fn script_text_is_taken_as_written() {
        assert_eq!(
            sources("<script>if (a < b && c) { d('</p>') } // <!-- x</script>"),
            ["if (a < b && c) { d('</p>') } // <!-- x"]
        );
    }

    /// Serves `/<name>.js` as `<name>()`, except `missing.js`. Records what was asked for.
    #[derive(Debug, Default)]
    struct Files(Mutex<Vec<String>>);

    impl ScriptSource for Files {
        fn fetch_blocking(&self, url: &str) -> Option<String> {
            self.0.lock().push(url.to_owned());
            let name = url.strip_prefix("https://example.test/")?.strip_suffix(".js")?;

            (name != "missing").then(|| format!("{name}()"))
        }
    }

    /// The scripts the parser ran for `html` as (source, url), and the URLs it fetched.
    fn external(html: &str) -> (Vec<(String, String)>, Vec<String>) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let files = Arc::new(Files::default());
        let options = Html5ParserOptions {
            scripting_enabled: true,
            stylesheets: None,
            scripts: Some(Box::new(Recorder(Arc::clone(&calls)))),
            script_sources: Some(Arc::clone(&files) as Arc<dyn ScriptSource>),
        };

        let mut stream = ByteStream::from_str(html, Encoding::UTF8);
        let url = Url::parse("https://example.test/dir/page.html").unwrap();
        let mut doc = DocumentBuilderImpl::new_document::<Config>(Some(url));
        let _ = Html5Parser::<Config>::parse_document(&mut stream, &mut doc, Some(options));

        let ran = calls
            .lock()
            .iter()
            .filter_map(|call| match call {
                Call::Execute { source, url, line } => {
                    assert!(
                        *line == 0 || !url.ends_with(".js"),
                        "an external script starts on line 0"
                    );
                    Some((source.clone(), url.clone()))
                }
                _ => None,
            })
            .collect();
        let fetched = files.0.lock().clone();

        (ran, fetched)
    }

    fn ran(html: &str) -> Vec<String> {
        external(html).0.into_iter().map(|(source, _)| source).collect()
    }

    #[test]
    fn a_blocking_external_script_runs_where_it_stands() {
        let (ran, fetched) =
            external("<script>before()</script><script src=\"/a.js\"></script><script>after()</script>");

        assert_eq!(
            ran,
            [
                ("before()".to_owned(), "https://example.test/dir/page.html".to_owned()),
                ("a()".to_owned(), "https://example.test/a.js".to_owned()),
                ("after()".to_owned(), "https://example.test/dir/page.html".to_owned()),
            ]
        );
        assert_eq!(fetched, ["https://example.test/a.js"]);
    }

    #[test]
    fn a_src_is_resolved_against_the_document() {
        let (_, fetched) = external(
            "<script src=\"rel.js\"></script><script src=\"../up.js\"></script>\
             <script src=\"https://example.test/abs.js\"></script>",
        );

        assert_eq!(
            fetched,
            [
                "https://example.test/dir/rel.js",
                "https://example.test/up.js",
                "https://example.test/abs.js"
            ]
        );
    }

    #[test]
    fn defer_and_async_scripts_run_after_the_parse_in_document_order() {
        let html = "<script defer src=\"/d1.js\"></script><script>inline1()</script>\
                    <script async src=\"/a1.js\"></script><script src=\"/block.js\"></script>\
                    <script defer src=\"/d2.js\"></script><script>inline2()</script>";

        assert_eq!(ran(html), ["inline1()", "block()", "inline2()", "d1()", "a1()", "d2()"]);
    }

    #[test]
    fn each_script_is_followed_by_a_checkpoint() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let options = Html5ParserOptions {
            scripting_enabled: true,
            stylesheets: None,
            scripts: Some(Box::new(Recorder(Arc::clone(&calls)))),
            script_sources: Some(Arc::new(Files::default())),
        };
        let mut stream = ByteStream::from_str(
            "<script src=\"/a.js\"></script><script defer src=\"/d.js\"></script>",
            Encoding::UTF8,
        );
        let url = Url::parse("https://example.test/").unwrap();
        let mut doc = DocumentBuilderImpl::new_document::<Config>(Some(url));
        let _ = Html5Parser::<Config>::parse_document(&mut stream, &mut doc, Some(options));

        let kinds: Vec<bool> = calls.lock().iter().map(|call| *call == Call::Checkpoint).collect();
        assert_eq!(kinds, [false, true, false, true]);
    }

    #[test]
    fn a_script_that_cannot_be_loaded_is_skipped() {
        assert_eq!(
            ran(
                "<script src=\"/missing.js\"></script><script defer src=\"/missing.js\"></script>\
                 <script src=\"/a.js\"></script><script defer src=\"/d.js\"></script>"
            ),
            ["a()", "d()"]
        );
    }

    #[test]
    fn the_text_of_an_external_script_element_is_not_run() {
        assert_eq!(ran("<script src=\"/a.js\">inline()</script>"), ["a()"]);
        assert_eq!(ran("<script src=\"/missing.js\">inline()</script>"), [] as [&str; 0]);
    }

    #[test]
    fn external_scripts_that_are_not_fetched() {
        for html in [
            "<script src=\"\"></script>",
            "<script type=\"module\" src=\"/a.js\"></script>",
            "<script type=\"text/plain\" src=\"/a.js\"></script>",
            "<template><script src=\"/a.js\"></script></template>",
            "<svg><script href=\"/a.js\"></script></svg>",
        ] {
            let (ran, fetched) = external(html);
            assert!(
                ran.is_empty() && fetched.is_empty(),
                "{html}: ran {ran:?}, fetched {fetched:?}"
            );
        }
    }

    #[test]
    fn href_only_names_a_file_on_an_svg_script() {
        assert_eq!(ran("<script href=\"/a.js\">inline()</script>"), ["inline()"]);
        assert_eq!(
            ran("<svg><script href=\"/a.js\">inline()</script></svg>"),
            [] as [&str; 0]
        );
        // and `src` only on an HTML one
        assert_eq!(ran("<svg><script src=\"/a.js\">inSvg()</script></svg>"), ["inSvg()"]);
    }

    /// Records the stylesheets asked for in the same log as the scripts, and has none.
    #[derive(Debug)]
    struct Sheets(Arc<Mutex<Vec<Call>>>);

    impl StylesheetSource for Sheets {
        fn fetch_blocking(&self, urls: &[String]) -> Vec<Option<Vec<u8>>> {
            self.0.lock().push(Call::Sheets(urls.to_vec()));
            vec![None; urls.len()]
        }
    }

    #[test]
    fn deferred_scripts_wait_for_the_stylesheets() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let options = Html5ParserOptions {
            scripting_enabled: true,
            stylesheets: Some(Arc::new(Sheets(Arc::clone(&calls)))),
            scripts: Some(Box::new(Recorder(Arc::clone(&calls)))),
            script_sources: Some(Arc::new(Files::default())),
        };
        let mut stream = ByteStream::from_str(
            "<script defer src=\"/d.js\"></script><link rel=\"stylesheet\" href=\"/late.css\">",
            Encoding::UTF8,
        );
        let url = Url::parse("https://example.test/").unwrap();
        let mut doc = DocumentBuilderImpl::new_document::<Config>(Some(url));
        let _ = Html5Parser::<Config>::parse_document(&mut stream, &mut doc, Some(options));

        assert_eq!(
            *calls.lock(),
            [
                Call::Sheets(vec!["https://example.test/late.css".to_owned()]),
                Call::Execute {
                    source: "d()".to_owned(),
                    url: "https://example.test/d.js".to_owned(),
                    line: 0
                },
                Call::Checkpoint,
            ]
        );
    }

    #[test]
    fn defer_on_an_inline_script_changes_nothing() {
        assert_eq!(
            ran("<script defer>first()</script><script async>second()</script><script src=\"/a.js\"></script>"),
            ["first()", "second()", "a()"]
        );
    }
}
