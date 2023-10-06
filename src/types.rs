use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("ureq error")]
    Request(#[from] Box<ureq::Error>),

    #[error("io error: {0}")]
    IO(#[from] std::io::Error),

    #[error("parse error: {0}")]
    Parse(String),
}

pub type Result<T> = std::result::Result<T, Error>;
