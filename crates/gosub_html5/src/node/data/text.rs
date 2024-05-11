#[derive(Clone, Debug, PartialEq, Eq)]
/// Data structure for text nodes
pub struct TextData {
    /// Actual text
    pub value: String,
}

impl Default for TextData {
    fn default() -> Self {
        Self::new()
    }
}

impl TextData {
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

    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }
}
