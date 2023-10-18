#[derive(Debug, PartialEq, Clone)]
/// Data structure for comment nodes
pub struct CommentData {
    /// The actual comment value
    pub(crate) value: String,
}

impl Default for CommentData {
    fn default() -> Self {
        Self::new()
    }
}

impl CommentData {
    pub(crate) fn new() -> Self {
        Self {
            value: "".to_string(),
        }
    }

    pub(crate) fn with_value(value: &str) -> Self {
        Self {
            value: value.to_string(),
        }
    }

    pub fn value(&self) -> &str {
        &self.value
    }
}
