//! Simple built-in text measurer.
//!
//! Estimates text dimensions using character count × average character width.
//! No external libraries required. Suitable for development / CI use.

use crate::common::font::FontInfo;
use crate::common::geo::Dimension;
use anyhow::Result;

/// Estimate the layout size of `text` rendered with `font_info`.
///
/// The estimate is: `avg_char_width × len` capped at `max_width`, then wrapped.
pub fn get_text_layout(
    text: &str,
    font_info: &FontInfo,
    max_width: f64,
    _dpi_scale_factor: f32,
) -> Result<Dimension> {
    if text.is_empty() {
        return Ok(Dimension::ZERO);
    }

    // Approximate: average char width is ~0.55 × font_size for a proportional font.
    let avg_char_width = font_info.size * 0.55;
    let total_width = text.chars().count() as f64 * avg_char_width;

    let effective_width = if max_width > 0.0 && total_width > max_width {
        max_width
    } else {
        total_width
    };

    // Height: line_height × number of wrapped lines
    let lines = if max_width > 0.0 && total_width > max_width {
        (total_width / max_width).ceil()
    } else {
        1.0
    };

    let height = font_info.line_height * lines;

    Ok(Dimension::new(effective_width, height))
}
