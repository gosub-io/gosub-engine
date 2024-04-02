//! Error results that can be returned from the Gosub styling module
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
//    #[error("css: generic error: {0}")]
//    CssGeneric(String),

    #[allow(dead_code)]
    #[error("css: compilation error: {0}")]
    CssCompile(String),
}
