//! HTML5 tokenizer and parser
//!
//! The parser's job is to take a stream of bytes and turn it into a DOM tree. The parser is
//! implemented as a state machine and runs in the current thread.
use crate::document::builder::DocumentBuilderImpl;
use crate::parser::Html5Parser;
use gosub_interface::config::HasDocument;
use gosub_interface::document::DocumentBuilder;

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
#[must_use]
pub fn html_compile<C: HasDocument>(html: &str) -> C::Document {
    let mut stream = ByteStream::new(Encoding::UTF8, None);
    stream.read_from_str(html, Some(Encoding::UTF8));
    stream.close();

    let mut doc = <DocumentBuilderImpl as DocumentBuilder<C>>::new_document(None);
    let _ = Html5Parser::<C>::parse_document(&mut stream, &mut doc, None);

    doc
}
