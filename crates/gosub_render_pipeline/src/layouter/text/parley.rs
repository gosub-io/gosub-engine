use crate::layouter::text::Alignment;
use std::fmt::Error;
use crate::common::font::parley::get_parley_layout;
use crate::common::geo::Dimension;


pub fn get_text_layout(text: &str, font_family: &str, font_size: f64, _font_weight: usize, line_height: f64, max_width: f64, alignment: Alignment) -> Result<Dimension, Error> {
    let layout = get_parley_layout(text, font_family, font_size, line_height, max_width, alignment);

    Ok(Dimension {
        width: layout.width() as f64,
        height: layout.height() as f64,
    })
}
