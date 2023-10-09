use thiserror::Error;

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
}

pub type Result<T> = std::result::Result<T, Error>;
