//! Error results that can be returned from the engine
use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("net: generic error: {0}")]
    Generic(String),
}
