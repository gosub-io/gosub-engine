#[derive(Debug, PartialEq, Clone)]
pub struct CommentData {
    value: String,
}

impl Default for CommentData {
    fn default() -> Self {
        Self::new()
    }
}

impl CommentData {
    pub fn new() -> Self {
        CommentData {
            value: "".to_string(),
        }
    }

    pub fn new_with_value(value: &str) -> Self {
        let mut comment_data = CommentData::new();
        comment_data.set_value(value);

        comment_data
    }

    pub fn get_value(&self) -> String {
        self.value.clone()
    }

    pub fn set_value(&mut self, new_value: &str) {
        self.value = new_value.to_owned();
    }
}
