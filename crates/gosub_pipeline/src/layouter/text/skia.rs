use std::fmt::Error;
use crate::common::font::FontInfo;
use crate::common::font::skia::get_skia_paragraph;
use crate::common::geo::Dimension;


pub fn get_text_layout(text: &str, font_info: &FontInfo, max_width: f64, dpi_scale_factor: f32) -> Result<Dimension, Error> {
    let paragraph = get_skia_paragraph(text, font_info, max_width, None, dpi_scale_factor);

    Ok(Dimension {
        width: paragraph.max_width() as f64,
        height: paragraph.height() as f64,
    })
}
