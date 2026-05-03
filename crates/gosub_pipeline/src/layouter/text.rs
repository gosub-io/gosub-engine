#[cfg(feature = "text_parley")]
pub mod parley;
#[cfg(feature = "text_pango")]
pub mod pango;

#[cfg(feature = "text_parley")]
pub use crate::layouter::text::parley::get_text_layout;
#[cfg(feature = "text_pango")]
pub use crate::layouter::text::pango::get_text_layout;

// Stub used when no text backend feature is enabled. Returns zero size.
#[cfg(not(any(feature = "text_parley", feature = "text_pango")))]
pub fn get_text_layout(
    _text: &str,
    _font_info: &crate::common::font::FontInfo,
    _max_width: f64,
    _dpi_scale_factor: f32,
) -> Result<crate::common::geo::Dimension, anyhow::Error> {
    Ok(crate::common::geo::Dimension::ZERO)
}
