use crate::common::font::FontInfo;
use crate::common::geo::Dimension;
use gosub_interface::font::FontStyle;
use gosub_interface::font_system::{FontStretch, FontSystem, FontWeight, TextStyle};

/// Measure `text` through the swappable [`FontSystem`] abstraction.
///
/// Builds a neutral [`TextStyle`] from `font_info` and asks the configured font system for the
/// laid-out bounding box. Measurement goes through whichever engine the config selected (Parley,
/// Pango, Skia, …), so layout boxes are sized by the same engine that will draw the text.
pub fn get_text_layout(
    text: &str,
    font_info: &FontInfo,
    max_width: f64,
    font_system: &mut dyn FontSystem,
) -> Result<Dimension, anyhow::Error> {
    let style = TextStyle {
        family: font_info.family.clone(),
        size: font_info.size as f32,
        weight: FontWeight(font_info.weight.clamp(1, 1000) as u16),
        style: FontStyle::Normal,
        stretch: FontStretch::NORMAL,
        line_height: Some(font_info.line_height as f32),
        max_width: Some(max_width as f32),
        // The layouter works in CSS pixels; DPI scaling is applied later in the pipeline.
        display_scale: 1.0,
    };

    let (width, height) = font_system.measure(text, &style);

    Ok(Dimension {
        width: width as f64,
        height: height as f64,
    })
}
