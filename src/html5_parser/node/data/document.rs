#[derive(Debug, PartialEq, Clone)]
/// Data structure for document nodes
pub struct DocumentData {}

impl Default for DocumentData {
    fn default() -> Self {
        Self::new()
    }
}

impl DocumentData {
    pub(crate) fn new() -> Self {
        DocumentData {}
    }
}
