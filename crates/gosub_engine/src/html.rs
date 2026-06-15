//! HTML parsing and related utilities.
//!
//! This module provides functionality to parse HTML documents, extract resource hints,
//! and handle various HTML configurations.
mod parser;

pub use parser::parse_main_document_stream;
pub use parser::{DocumentError, DummyDocument, DummyHtml5Config, ResourceHint};

use gosub_css3::system::Css3System;
use gosub_html5::document::document_impl::DocumentImpl;
use gosub_html5::parser::Html5Parser;
use gosub_interface::config::ModuleConfiguration;
use gosub_interface::document::Document as _;
use gosub_interface::node::NodeType;
use gosub_shared::node::NodeId;

/// The engine's default [`ModuleConfiguration`], wiring the gosub_html5 document
/// implementation together with the gosub_css3 style system.
///
/// Embedders that don't supply their own config get this one: `GosubEngine` (and the
/// rest of the engine) are generic over `C: ModuleConfiguration` and default to
/// `DefaultConfig`.
#[derive(Clone, Debug, PartialEq)]
pub struct DefaultConfig;

impl ModuleConfiguration for DefaultConfig {
    type CssSystem = Css3System;
    type Document = DocumentImpl<Self>;
    type HtmlParser = Html5Parser<'static, Self>;
}

/// A [`ModuleConfiguration`] this engine can actually drive.
///
/// The engine is generic over `C: EngineConfig`. This is just `ModuleConfiguration` pinned so the
/// config's `Document` is `DocumentImpl<Self>` — the HTML parser produces that concrete document
/// type, so it is coupled to the parser rather than independently swappable (see the design doc:
/// `Document` is internal plumbing). Stating it once here keeps the bound off every engine
/// signature; engine code bounds on `C: EngineConfig` and the public `ModuleConfiguration` stays
/// fully general.
pub trait EngineConfig: ModuleConfiguration<Document = DocumentImpl<Self>> {}
impl<C: ModuleConfiguration<Document = DocumentImpl<C>>> EngineConfig for C {}

/// The parsed document type used by the engine for a given config (defaults to [`DefaultConfig`]).
pub type EngineDocument<C = DefaultConfig> = DocumentImpl<C>;

/// Extract the text content of the first `<title>` element in the document.
pub fn document_title<C: EngineConfig>(doc: &EngineDocument<C>) -> Option<String> {
    find_title(doc, doc.root())
}

fn find_title<C: EngineConfig>(doc: &EngineDocument<C>, node_id: NodeId) -> Option<String> {
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
