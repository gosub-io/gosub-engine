//! Error results that can be returned from the Gosub styling module
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("css: compilation error: {0}")]
    CssCompile(String),
}
