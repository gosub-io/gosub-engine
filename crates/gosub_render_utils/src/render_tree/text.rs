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
            value: String::new(),
            font: "Times New Roman".to_owned(),
            font_size: fs,
            is_bold: bold,
        }
    }

    #[must_use]
    pub fn new_heading1() -> Self {
        Self::new(37., true)
    }

    #[must_use]
    pub fn new_heading2() -> Self {
        Self::new(27.5, true)
    }

    #[must_use]
    pub fn new_heading3() -> Self {
        Self::new(21.5, true)
    }

    #[must_use]
    pub fn new_heading4() -> Self {
        Self::new(18.5, true)
    }

    #[must_use]
    pub fn new_heading5() -> Self {
        Self::new(15.5, true)
    }

    #[must_use]
    pub fn new_heading6() -> Self {
        Self::new(12., true)
    }

    #[must_use]
    pub fn new_paragraph() -> Self {
        Self::new(18.5, false)
    }
}
