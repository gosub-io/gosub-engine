#[cfg(not(any(feature = "text_parley", feature = "text_pango", feature = "text_skia")))]
compile_error!("Either the 'text_parley' 'text_skia' or 'text_pango' feature must be enabled");

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

//
// /// Text alignment
// #[derive(Clone, Debug, Copy)]
// pub enum Alignment {
//     /// Alignment of text is at the start (depends on LTR)
//     Start,
//     /// Alignment of text is at the end (depends on LTR)
//     End,
//     /// Alignment is centered
//     Middle,
//     /// alignment is justified (full column width)
//     Justified,
// }