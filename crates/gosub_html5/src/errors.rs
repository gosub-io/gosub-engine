//! Error results that can be returned from the engine
use gosub_shared::byte_stream::Location;
use thiserror::Error;

/// Parser error that defines an error (message) on the given position
#[derive(Clone, Debug, PartialEq)]
pub struct ParseError {
    /// Parse error message
    pub message: String,
    pub location: Location,
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
