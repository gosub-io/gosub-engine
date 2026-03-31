#[cfg(not(any(feature = "text_simple", feature = "text_parley", feature = "text_pango", feature = "text_skia")))]
compile_error!("One of 'text_simple', 'text_parley', 'text_pango', or 'text_skia' must be enabled");

// text_simple is always compiled in (it is in the default feature set), but only re-exported
// when no more specific text backend is active.
#[cfg(feature = "text_simple")]
pub mod simple;
#[cfg(all(feature = "text_simple", not(any(feature = "text_parley", feature = "text_pango", feature = "text_skia"))))]
pub use crate::layouter::text::simple::get_text_layout;

#[cfg(feature = "text_parley")]
pub mod parley;
#[cfg(feature = "text_parley")]
pub use crate::layouter::text::parley::get_text_layout;

#[cfg(feature = "text_pango")]
pub mod pango;
#[cfg(feature = "text_pango")]
pub use crate::layouter::text::pango::get_text_layout;

#[cfg(feature = "text_skia")]
pub mod skia;
#[cfg(feature = "text_skia")]
pub use crate::layouter::text::skia::get_text_layout;
