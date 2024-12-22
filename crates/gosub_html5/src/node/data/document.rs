use core::fmt::{Debug, Formatter};
use gosub_interface::node::{DocumentDataType, QuirksMode};
use std::fmt;

#[derive(PartialEq, Clone)]
/// Data structure for document nodes
pub struct DocumentData {
    quirks_mode: QuirksMode,
}

impl Default for DocumentData {
    fn default() -> Self {
        Self {
            quirks_mode: QuirksMode::NoQuirks,
        }
    }
}

impl Debug for DocumentData {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let mut debug = f.debug_struct("DocumentData");
        debug.finish()
    }
}

impl DocumentData {
    #[must_use]
    pub(crate) fn new(quirks_mode: QuirksMode) -> Self {
        Self { quirks_mode }
    }
}

impl DocumentDataType for DocumentData {
    fn quirks_mode(&self) -> QuirksMode {
        self.quirks_mode
    }

    fn set_quirks_mode(&mut self, quirks_mode: QuirksMode) {
        self.quirks_mode = quirks_mode;
    }
}
