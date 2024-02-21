//! Javascript engine functionality
//!
//! This crate adds the ability to run javascript code in the gosub engine.
//!

use thiserror::Error;

#[allow(unused, unused_imports, unused_variables, dead_code)]
#[allow(dead_code, unused)]
pub mod js;

#[cfg(test)]
mod test;

#[derive(Debug, Error)]
pub enum Error {
    #[error("js: {0}")]
    JS(#[from] js::JSError),
}
