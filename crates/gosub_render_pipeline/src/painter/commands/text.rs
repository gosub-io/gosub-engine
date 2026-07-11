use crate::common::font::FontInfo;
use crate::common::geo::Rect;
use crate::painter::commands::brush::Brush;
use gosub_interface::font_system::ShapedText;

#[derive(Clone, Debug)]
pub struct Text {
    /// The rectangle in which the text should be drawn
    pub rect: Rect,
    pub font_info: FontInfo,
    /// Actual text
    pub text: String,
    /// Brush to paint the text with
    pub brush: Brush,
    /// The container width (CSS px) that the layout engine used as the word-wrap limit.
    /// Renderers should use this instead of `rect.width` to avoid metric-mismatch wrapping.
    pub available_width: f64,
    /// Positioned glyph runs, shaped once at paint-command build time by the configured
    /// `FontSystem` — the same instance the layouter measured with, so the glyphs painted are
    /// by construction the glyphs that were measured. Glyph-based rasterizers (`text_glyphs`)
    /// paint exactly these runs; engine-native rasterizers (Pango, Parley, Skia textlayout)
    /// re-shape from `text` + `font_info` instead and ignore this field.
    pub shaped: ShapedText,
}

impl Text {
    pub fn new(
        rect: Rect,
        text: &str,
        font_info: &FontInfo,
        brush: Brush,
        available_width: f64,
        shaped: ShapedText,
    ) -> Self {
        Text {
            rect,
            text: text.to_string(),
            font_info: font_info.clone(),
            brush,
            available_width,
            shaped,
        }
    }
}
