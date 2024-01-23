#[derive(Debug, PartialEq, Clone)]
/// Data structure for comment nodes
pub struct CommentData {
    /// The actual comment value
    pub value: String,
}

impl Default for CommentData {
    fn default() -> Self {
        Self::new()
    }
}

impl CommentData {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            value: String::new(),
        }
    }

    pub(crate) fn with_value(value: &str) -> Self {
        Self {
            value: value.to_owned(),
        }
    }

    pub fn value(&self) -> &str {
        &self.value
    }
}
