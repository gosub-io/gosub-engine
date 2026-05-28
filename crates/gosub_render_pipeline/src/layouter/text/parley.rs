use crate::common::font::parley::get_parley_layout;
use crate::common::font::FontInfo;
use crate::common::geo::Dimension;

pub fn get_text_layout(
    text: &str,
    font_info: &FontInfo,
    max_width: f64,
    _dpi_scale_factor: f32,
) -> Result<Dimension, anyhow::Error> {
    let layout = get_parley_layout(text, font_info, max_width);

    Ok(Dimension {
        width: layout.width() as f64,
        height: layout.height() as f64,
    })
}
