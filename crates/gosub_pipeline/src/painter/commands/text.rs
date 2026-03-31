use crate::common::font::FontInfo;
use crate::common::geo::Rect;
use crate::painter::commands::brush::Brush;

#[derive(Clone, Debug)]
pub struct Text {
    /// The rectangle in which the text should be drawn
    pub rect: Rect,
    pub font_info: FontInfo,
    /// Actual text
    pub text: String,
    /// Brush to paint the text with
    pub brush: Brush,
}

impl Text {
    pub fn new(rect: Rect, text: &str, font_info: &FontInfo, brush: Brush) -> Self {
        Text {
            rect,
            text: text.to_string(),
            font_info: font_info.clone(),
            brush,
        }
    }
}

