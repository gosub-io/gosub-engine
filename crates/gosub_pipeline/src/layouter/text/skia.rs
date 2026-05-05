use crate::common::font::FontInfo;
use crate::common::font::skia::get_skia_paragraph;
use crate::common::geo::Dimension;


pub fn get_text_layout(text: &str, font_info: &FontInfo, max_width: f64, dpi_scale_factor: f32) -> Dimension {
    let paragraph = get_skia_paragraph(text, font_info, max_width, None, dpi_scale_factor);

    Dimension {
        width: paragraph.longest_line() as f64,
        height: paragraph.height() as f64,
    }
}
