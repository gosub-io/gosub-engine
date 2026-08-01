//! Javascript engine functionality

use thiserror::Error;

pub mod js;

#[derive(Debug, Error)]
pub enum Error {
    #[error("js: {0}")]
    JS(#[from] js::JSError),
}
