static DEFAULT_FONT: &str = "Times New Roman";

#[derive(Debug, PartialEq)]
pub struct TextNode {
    /// Body of the text node that will be drawn
    pub value: String,
    // TODO: change this to a stack-allocated string
    pub font: String,
    pub font_size: f64,
    pub is_bold: bool,
    // TODO: color, styles, visiblity, etc
}

impl TextNode {
    /*
        NOTE: I got the default font sizes from https://stackoverflow.com/a/70720104
    */

    fn new(fs: f64) -> Self {
        Self {
            value: "".to_owned(),
            font: DEFAULT_FONT.to_owned(),
            font_size: fs,
            is_bold: false,
        }
    }

    pub fn new_heading1() -> Self {
        TextNode::new(37.)
    }

    pub fn new_heading2() -> Self {
        TextNode::new(27.5)
    }

    pub fn new_heading3() -> Self {
        TextNode::new(21.5)
    }

    pub fn new_heading4() -> Self {
        TextNode::new(18.5)
    }

    pub fn new_heading5() -> Self {
        TextNode::new(15.5)
    }

    pub fn new_heading6() -> Self {
        TextNode::new(12.)
    }

    pub fn new_paragraph() -> Self {
        TextNode::new(18.5)
    }
}
