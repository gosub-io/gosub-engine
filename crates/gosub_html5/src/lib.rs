//! HTML5 tokenizer and parser
use crate::document::builder::DocumentBuilderImpl;
use crate::parser::Html5Parser;
use gosub_interface::config::HasDocument;

use gosub_shared::byte_stream::{ByteStream, Encoding};

pub mod document;
pub mod dom;
pub mod errors;
pub mod node;
pub mod parser;
pub mod testing;
pub mod tokenizer;
#[allow(dead_code)]
pub mod writer;

/// Parses the given HTML string and returns a handle to the resulting DOM tree.
///
/// TODO: make the parser incremental / push-driven so callers can feed chunks as they
/// arrive from the network instead of requiring the full document upfront. This would
/// allow the engine to start building the DOM and dispatching sub-resource fetches
/// (images, stylesheets) before the HTML response has finished downloading.
/// See: gosub_engine async fetch/parse pipeline plan (async-resource-pipeline branch).
#[must_use]
pub fn html_compile<C: HasDocument>(html: &str) -> C::Document {
    let mut stream = ByteStream::from_str(html, Encoding::UTF8);

    let mut doc = DocumentBuilderImpl::new_document::<C>(None);
    let _ = Html5Parser::<C>::parse_document(&mut stream, &mut doc, None);

    doc
}
