use indexmap::IndexMap;
use thiserror::Error;

/// Generic error types that can be returned from the library.
#[derive(Error, Debug)]
pub enum Error {
    #[error("ureq error")]
    Request(#[from] Box<ureq::Error>),

    #[error("io error: {0}")]
    IO(#[from] std::io::Error),

    #[error("parse error: {0}")]
    Parse(String),

    #[error("utf8 conversion error: {0}")]
    Utf8(#[from] std::string::FromUtf8Error),

    #[error("json parsing error: {0}")]
    JsonSerde(#[from] serde_json::Error),
}

/// Result that can be returned which holds either T or an Error
pub type Result<T> = std::result::Result<T, Error>;

/// Element attributes
pub type AttributeMap = IndexMap<String, String>;
