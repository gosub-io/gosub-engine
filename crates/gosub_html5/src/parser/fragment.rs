//! The HTML fragment parsing algorithm.

use crate::node::HTML_NAMESPACE;
use crate::parser::errors::ErrorLogger;
use crate::parser::{Html5Parser, Html5ParserOptions, InsertionMode};
use crate::tokenizer::state::State;
use crate::tokenizer::token::Token;
use crate::tokenizer::Tokenizer;
use core::cell::RefCell;
use gosub_interface::config::HasDocument;
use gosub_interface::document::{Document, DocumentType};
use gosub_shared::attributes::Attributes;
use gosub_shared::byte_stream::{ByteStream, Location};
use gosub_shared::node::NodeId;
use gosub_shared::types::{ParseError, Result};
use std::rc::Rc;

impl<C: HasDocument> Html5Parser<'_, C> {
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

    /// Find the correct tokenizer state when we are about to parse a fragment case
    pub(crate) fn find_initial_state_for_context(&self, context_node_id: NodeId) -> State {
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
    pub(crate) fn initialize_fragment_case(&mut self, context_node_id: NodeId) {
        self.is_fragment_case = true;

        self.tokenizer
            .set_state(self.find_initial_state_for_context(context_node_id));

        self.context_node_id = Some(context_node_id);
    }
}
