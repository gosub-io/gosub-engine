//! Javascript engine functionality
//!
//! This crate adds the ability to run javascript code in the gosub engine.
//!

use thiserror::Error;

pub mod js;

#[derive(Debug, Error)]
pub enum WebExecutorError {
    #[error("js: {0}")]
    JS(#[from] js::JSError),
}

pub use WebExecutorError as Error;
