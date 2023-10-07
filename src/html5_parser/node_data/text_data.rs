#[derive(Debug, PartialEq, Clone)]
pub struct TextData {
    value: String,
}

impl Default for TextData {
    fn default() -> Self {
        Self::new()
    }
}

impl TextData {
    pub fn new() -> Self {
        TextData {
            value: "".to_string(),
        }
    }
}
