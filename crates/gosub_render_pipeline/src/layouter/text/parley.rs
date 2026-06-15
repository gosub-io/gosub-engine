use crate::common::font::FontInfo;
use crate::common::geo::Dimension;
use gosub_interface::font::FontStyle;
use gosub_interface::font_system::{FontQuery, FontStretch, FontSystem, FontWeight};

/// Measure `text` through the swappable [`FontSystem`] abstraction.
///
/// The font system resolves `font_info` to a concrete font and returns the laid-out
/// bounding box. Shaping goes through the trait rather than the backend-specific font
/// context, so the layouter is independent of which font system is in use.
pub fn get_text_layout(
    text: &str,
    font_info: &FontInfo,
    max_width: f64,
    font_system: &mut dyn FontSystem,
) -> Result<Dimension, anyhow::Error> {
    // Append a generic fallback so resolution always succeeds even when the primary
    // family is unavailable (mirrors CSS's implicit fallback to a generic family).
    let families = [font_info.family.as_str(), "sans-serif"];
    let query = FontQuery {
        families: &families,
        style: FontStyle::Normal,
        weight: FontWeight(font_info.weight.clamp(1, 1000) as u16),
        stretch: FontStretch::NORMAL,
    };

    let resolved = font_system
        .resolve(&query)
        .map_err(|e| anyhow::anyhow!("font resolution failed: {e:?}"))?;

    // display_scale is 1.0: the layouter works in CSS pixels (DPI scaling is applied
    // later in the pipeline). Kept as an explicit argument for when that changes.
    let (width, height) = font_system.measure(
        text,
        &resolved,
        font_info.size as f32,
        Some(font_info.line_height as f32),
        Some(max_width as f32),
        1.0,
    );

    Ok(Dimension {
        width: width as f64,
        height: height as f64,
    })
}
