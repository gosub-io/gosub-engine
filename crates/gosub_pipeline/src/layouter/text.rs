#[cfg(feature = "text_pango")]
pub mod pango;
#[cfg(feature = "text_parley")]
pub mod parley;
#[cfg(feature = "text_skia")]
pub mod skia;

#[cfg(all(feature = "text_pango", not(feature = "text_parley"), not(feature = "text_skia")))]
pub use crate::layouter::text::pango::get_text_layout;
#[cfg(feature = "text_parley")]
pub use crate::layouter::text::parley::get_text_layout;
#[cfg(all(feature = "text_skia", not(feature = "text_parley")))]
pub use crate::layouter::text::skia::get_text_layout;

#[cfg(not(any(feature = "text_parley", feature = "text_pango", feature = "text_skia")))]
pub fn get_text_layout(
    _text: &str,
    _font_info: &crate::common::font::FontInfo,
    _max_width: f64,
    _dpi_scale_factor: f32,
) -> Result<crate::common::geo::Dimension, anyhow::Error> {
    anyhow::bail!("No text backend feature enabled (enable text_pango, text_parley, or text_skia)")
}
