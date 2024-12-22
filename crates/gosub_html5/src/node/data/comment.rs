use gosub_interface::node::CommentDataType;

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
    fn new() -> Self {
        Self { value: String::new() }
    }

    pub(crate) fn with_value(value: &str) -> Self {
        Self {
            value: value.to_owned(),
        }
    }
}

impl CommentDataType for CommentData {
    fn value(&self) -> &str {
        &self.value
    }
}
