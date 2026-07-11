#[cfg(not(any(feature = "text_skia", feature = "text_glyphs")))]
compile_error!("Either the 'text_skia' or 'text_glyphs' feature must be enabled");

// `text_glyphs` is the engine-neutral glyph-run painter (works with any configured FontSystem)
// and wins over the Skia-native textlayout variant when enabled.
#[cfg(feature = "text_glyphs")]
pub mod glyphs;
#[cfg(feature = "text_glyphs")]
pub use crate::rasterizer::text::glyphs::do_paint_text;

#[cfg(all(feature = "text_skia", not(feature = "text_glyphs")))]
pub mod skia;
#[cfg(all(feature = "text_skia", not(feature = "text_glyphs")))]
pub use crate::rasterizer::text::skia::do_paint_text;
