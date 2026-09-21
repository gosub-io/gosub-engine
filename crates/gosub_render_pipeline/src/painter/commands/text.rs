use crate::common::font::FontInfo;
use crate::common::geo::Rect;
use crate::painter::commands::brush::Brush;
use gosub_interface::font_system::ShapedText;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct Text {
    pub rect: Rect,
    pub font_info: FontInfo,
    pub text: String,
    pub brush: Brush,
    /// Word-wrap limit (CSS px) the layouter used. Renderers must prefer this over `rect.width`
    /// to avoid metric-mismatch wrapping.
    pub available_width: f64,
    /// Shaped by the same `FontSystem` instance the layouter measured with, so painted glyphs are
    /// by construction the measured ones. Glyph-based rasterizers paint these runs; engine-native
    /// ones (Pango, Parley, Skia textlayout) re-shape from `text` + `font_info` and ignore this.
    ///
    /// Shared: an element straddling tiles hands the same runs to each of them, and the painter's
    /// shape cache hands them to the next pass as well.
    pub shaped: Arc<ShapedText>,
}

impl Text {
    pub fn new(
        rect: Rect,
        text: &str,
        font_info: &FontInfo,
        brush: Brush,
        available_width: f64,
        shaped: Arc<ShapedText>,
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
