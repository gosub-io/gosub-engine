//! HTML parsing and related utilities.
//!
//! This module bridges `gosub_html5` async stream parsing with the engine's net types.
mod parser;

pub use gosub_html5::async_parse::{
    extract_title, parse_html_stream, HintKind, HintPriority, Html5ParseConfig, ParseStreamError, ResourceHint,
};
pub use parser::hint_to_net;
