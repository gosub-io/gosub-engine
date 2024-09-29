//! HTML5 tokenizer and parser
//!
//! The parser's job is to take a stream of bytes and turn it into a DOM tree. The parser is
//! implemented as a state machine and runs in the current thread.
use crate::document::builder::DocumentBuilderImpl;
use crate::document::document_impl::DocumentImpl;
use crate::parser::Html5Parser;
use gosub_shared::byte_stream::{ByteStream, Encoding};
use gosub_shared::document::DocumentHandle;
use gosub_shared::traits::css3::CssSystem;
use gosub_shared::traits::document::DocumentBuilder as _;

pub mod document;
pub mod dom;
pub mod errors;
pub mod node;
pub mod parser;
pub mod tokenizer;
#[allow(dead_code)]
pub mod writer;

/// Parses the given HTML string and returns a handle to the resulting DOM tree.
pub fn html_compile<C: CssSystem>(html: &str) -> DocumentHandle<DocumentImpl<C>, C> {
    let mut stream = ByteStream::new(Encoding::UTF8, None);
    stream.read_from_str(html, Some(Encoding::UTF8));
    stream.close();

    let doc_handle = DocumentBuilderImpl::new_document(None);
    let _ = Html5Parser::parse_document(&mut stream, doc_handle.clone(), None);

    doc_handle
}
