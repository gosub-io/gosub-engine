use gosub_shared::traits::node::TextDataType;

#[derive(Clone, Debug, PartialEq)]

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
    pub fn new() -> Self {
        Self { value: String::new() }
    }

    pub fn with_value(value: &str) -> Self {
        Self {
            value: value.to_owned(),
        }
    }
}

impl TextDataType for TextData {
    fn value(&self) -> &str {
        &self.value
    }

    fn string_value(&self) -> String {
        self.value.clone()
    }

    fn value_mut(&mut self) -> &mut String {
        &mut self.value
    }
}
