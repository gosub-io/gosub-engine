//! HTML5 tokenizer and parser
//!
//! The parser's job is to take a stream of bytes and turn it into a DOM tree. The parser is
//! implemented as a state machine and runs in the current thread.
use crate::parser::document::{Document, DocumentBuilder, DocumentHandle};
use crate::parser::Html5Parser;
use gosub_shared::byte_stream::{ByteStream, Encoding};

pub mod dom;
pub mod element_class;
pub mod error_logger;
mod errors;
pub mod node;
pub mod parser;
pub mod tokenizer;
pub mod util;
pub mod visit;
pub mod writer;

/// Parses the given HTML string and returns a handle to the resulting DOM tree.
pub fn html_compile(html: &str) -> DocumentHandle {
    let mut stream = ByteStream::new(Encoding::UTF8, None);
    stream.read_from_str(html, Some(Encoding::UTF8));
    stream.close();

    let document = DocumentBuilder::new_document(None);
    let _ = Html5Parser::parse_document(&mut stream, Document::clone(&document), None);

    document
}
