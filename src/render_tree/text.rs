#[derive(Debug, PartialEq)]
#[repr(C)]
pub struct TextNode {
    /// Body of the text node that will be drawn
    pub value: String,
    pub font: String,
    pub font_size: f64,
    pub is_bold: bool,
    // TODO: color, styles, visiblity, etc
}

impl TextNode {
    #[must_use]
    fn new(fs: f64, bold: bool) -> Self {
        Self {
            value: "".to_owned(),
            font: "Times New Roman".to_owned(),
            font_size: fs,
            is_bold: bold,
        }
    }

    pub fn new_heading1() -> Self {
        TextNode::new(37., true)
    }

    pub fn new_heading2() -> Self {
        TextNode::new(27.5, true)
    }

    pub fn new_heading3() -> Self {
        TextNode::new(21.5, true)
    }

    pub fn new_heading4() -> Self {
        TextNode::new(18.5, true)
    }

    pub fn new_heading5() -> Self {
        TextNode::new(15.5, true)
    }

    pub fn new_heading6() -> Self {
        TextNode::new(12., true)
    }

    pub fn new_paragraph() -> Self {
        TextNode::new(18.5, false)
    }
}
