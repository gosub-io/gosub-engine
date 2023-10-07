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
}
