//! HTML5 tokenizer and parser
//!
//! The parser's job is to take a stream of bytes and turn it into a DOM tree. The parser is
//! implemented as a state machine and runs in the current thread.
pub mod dom;
pub mod element_class;
pub mod error_logger;
pub mod input_stream;
pub mod node;
pub mod parser;
pub mod tokenizer;
