// `text_glyphs` is the engine-neutral glyph-run painter (works with any configured FontSystem)
// and takes precedence over the engine-native variants in the rasterizer's dispatch.
#[cfg(feature = "text_glyphs")]
pub mod glyphs;
#[cfg(all(feature = "text_pango", not(feature = "text_glyphs")))]
pub mod pango;
#[cfg(all(feature = "text_parley", not(feature = "text_glyphs")))]
pub mod parley;
#[cfg(all(feature = "text_skia", not(feature = "text_glyphs")))]
pub mod skia;
