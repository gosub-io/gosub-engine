//! Error results that can be returned from the engine
use thiserror::Error;

/// Parser error that defines an error (message) on the given position
#[derive(Clone, Debug, PartialEq)]
pub struct ParseError {
    /// Parse error message
    pub message: String,
    /// Line number (1-based) of the error
    pub line: usize,
    // Column (1-based) on line of the error
    pub col: usize,
    // Position (0-based) of the error in the input stream
    pub offset: usize,
}

/// Serious errors and errors from third-party libraries
#[derive(Debug, Error)]
pub enum Error {
    #[error("parse error: {0}")]
    Parse(String),

    #[error("utf8 conversion error: {0}")]
    Utf8(#[from] std::string::FromUtf8Error),

    #[error("document task error: {0}")]
    DocumentTask(String),

    #[error("query: generic error: {0}")]
    Query(String),
}