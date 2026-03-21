#[cfg(not(any(feature = "text_parley", feature = "text_pango", feature = "text_skia")))]
compile_error!("Either the 'text_parley' 'text_skia' or 'text_pango' feature must be enabled");

#[cfg(feature = "text_parley")]
pub mod parley;
#[cfg(feature = "text_parley")]
pub use crate::rasterizer::vello::text::parley::do_paint_text;

#[cfg(feature = "text_pango")]
pub mod pango;
#[cfg(feature = "text_pango")]
pub use crate::rasterizer::vello::text::pango::do_paint_text;

#[cfg(feature = "text_skia")]
pub mod skia;
#[cfg(feature = "text_skia")]
pub use crate::rasterizer::vello::text::skia::do_paint_text;
