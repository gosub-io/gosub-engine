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

    pub fn new_with_value(value: &str) -> Self {
        let mut text_data = TextData::new();
        text_data.set_value(value);

        text_data
    }

    pub fn get_value(&self) -> String {
        self.value.clone()
    }

    pub fn set_value(&mut self, new_value: &str) {
        self.value = new_value.to_owned();
    }

    pub fn push_str(&mut self, content: &str) {
        self.value.push_str(content);
    }
}
