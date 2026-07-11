#[cfg(not(any(
    feature = "text_glyphs",
    feature = "text_parley",
    feature = "text_pango",
    feature = "text_skia"
)))]
compile_error!("One of the 'text_glyphs', 'text_parley', 'text_skia', or 'text_pango' features must be enabled");

// `text_glyphs` is the engine-neutral glyph-run painter and wins over the engine-native
// variants when enabled.
#[cfg(feature = "text_glyphs")]
pub mod glyphs;
#[cfg(feature = "text_glyphs")]
pub use crate::rasterizer::text::glyphs::do_paint_text;

#[cfg(all(feature = "text_parley", not(feature = "text_glyphs")))]
pub mod parley;
#[cfg(all(feature = "text_parley", not(feature = "text_glyphs")))]
pub use crate::rasterizer::text::parley::do_paint_text;

#[cfg(all(feature = "text_skia", not(feature = "text_glyphs")))]
pub mod skia;
#[cfg(all(feature = "text_skia", not(feature = "text_parley"), not(feature = "text_glyphs")))]
pub use crate::rasterizer::text::skia::do_paint_text;

#[cfg(all(feature = "text_pango", not(feature = "text_glyphs")))]
pub mod pango;
#[cfg(all(
    feature = "text_pango",
    not(feature = "text_parley"),
    not(feature = "text_skia"),
    not(feature = "text_glyphs")
))]
pub use crate::rasterizer::text::pango::do_paint_text;
