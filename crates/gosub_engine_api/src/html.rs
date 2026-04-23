//! HTML parsing and related utilities.
//!
//! This module provides functionality to parse HTML documents, extract resource hints,
//! and handle various HTML configurations.
mod parser;

pub use parser::parse_main_document_stream;
pub use parser::{DocumentError, DummyDocument, DummyHtml5Config, ResourceHint};
