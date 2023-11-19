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
    pub fn new_heading1() -> Self {
        Self {
            value: "".to_owned(),
            font: DEFAULT_FONT.to_owned(),
            font_size: 37.,
            is_bold: true,
        }
    }

    pub fn new_heading2() -> Self {
        Self {
            value: "".to_owned(),
            font: DEFAULT_FONT.to_owned(),
            font_size: 27.5,
            is_bold: true,
        }
    }

    pub fn new_heading3() -> Self {
        Self {
            value: "".to_owned(),
            font: DEFAULT_FONT.to_owned(),
            font_size: 21.5,
            is_bold: true,
        }
    }

    pub fn new_heading4() -> Self {
        Self {
            value: "".to_owned(),
            font: DEFAULT_FONT.to_owned(),
            font_size: 18.5,
            is_bold: true,
        }
    }

    pub fn new_heading5() -> Self {
        Self {
            value: "".to_owned(),
            font: DEFAULT_FONT.to_owned(),
            font_size: 15.5,
            is_bold: true,
        }
    }

    pub fn new_heading6() -> Self {
        Self {
            value: "".to_owned(),
            font: DEFAULT_FONT.to_owned(),
            font_size: 12.,
            is_bold: true,
        }
    }

    pub fn new_paragraph() -> Self {
        Self {
            value: "".to_owned(),
            font: DEFAULT_FONT.to_owned(),
            font_size: 18.5,
            is_bold: false,
        }
    }
}
