//! HTML parsing and related utilities.
//!
//! This module provides functionality to parse HTML documents, extract resource hints,
//! and handle various HTML configurations.
mod parser;

pub use parser::parse_main_document_stream;
pub use parser::{DocumentError, DummyDocument, DummyHtml5Config, ResourceHint};

use gosub_css3::system::Css3System;
use gosub_html5::document::document_impl::DocumentImpl;
use gosub_interface::config::{HasCssSystem, HasDocument};
use gosub_interface::document::Document as _;
use gosub_interface::node::NodeType;
use gosub_shared::node::NodeId;

/// Concrete type-system configuration that wires together the gosub_html5 document
/// implementation with the gosub_css3 style system.
///
/// This is the `C: HasDocument` type parameter used throughout the engine wherever
/// a concrete document + CSS pairing is required (parsing, pipeline, rendering).
#[derive(Clone, Debug, PartialEq)]
pub struct HtmlEngineConfig;

impl HasCssSystem for HtmlEngineConfig {
    type CssSystem = Css3System;
}

impl HasDocument for HtmlEngineConfig {
    type Document = DocumentImpl<Self>;
}

/// The real parsed document type used by the engine.
pub type EngineDocument = DocumentImpl<HtmlEngineConfig>;

/// Extract the text content of the first `<title>` element in the document.
pub fn document_title(doc: &EngineDocument) -> Option<String> {
    find_title(doc, doc.root())
}

fn find_title(doc: &EngineDocument, node_id: NodeId) -> Option<String> {
    for &child in doc.children(node_id) {
        if doc.node_type(child) == NodeType::ElementNode {
            if doc
                .tag_name(child)
                .map(|t| t.eq_ignore_ascii_case("title"))
                .unwrap_or(false)
            {
                // Collect text from title's children
                let mut text = String::new();
                for &t in doc.children(child) {
                    if doc.node_type(t) == NodeType::TextNode {
                        if let Some(v) = doc.text_value(t) {
                            text.push_str(v);
                        }
                    }
                }
                let trimmed = text.trim().to_string();
                if !trimmed.is_empty() {
                    return Some(trimmed);
                }
            } else if let Some(found) = find_title(doc, child) {
                return Some(found);
            }
        }
    }
    None
}
