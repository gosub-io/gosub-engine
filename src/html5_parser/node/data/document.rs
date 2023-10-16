use core::fmt::{Debug, Formatter};
use std::fmt;

#[derive(PartialEq, Clone)]
/// Data structure for document nodes
pub struct DocumentData {}

impl Default for DocumentData {
    fn default() -> Self {
        Self::new()
    }
}

impl Debug for DocumentData {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let mut debug = f.debug_struct("DocumentData");
        debug.finish()
    }
}

impl DocumentData {
    pub(crate) fn new() -> Self {
        DocumentData {}
    }
}
