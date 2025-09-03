#[cfg(not(any(feature = "text_parley", feature = "text_pango", feature = "text_skia")))]
compile_error!("Either the 'text_parley' 'text_skia' or 'text_pango' feature must be enabled");

#[cfg(feature = "text_parley")]
pub mod parley;
#[cfg(feature = "text_pango")]
pub mod pango;
#[cfg(feature = "text_skia")]
pub mod skia;

#[derive(Debug, Clone)]
pub enum FontAlignment {
    /// Start of the line (left for LTR, right for RTL)
    Start,
    Center,
    /// End of the line (right for LTR, left for RTL)
    End,
    Justify,
}

#[derive(Debug, Clone)]
pub struct FontInfo {
    /// Font family name(s)
    pub family: String,
    /// Font size in px
    pub size: f64,
    /// Font weight (100-900)
    pub weight: i32,
    /// Font width (100-900)
    pub width: i32,
    /// Font slant (0-1000)
    pub slant: i32,
    /// Line height in px
    pub line_height: f64,
    /// Font alignment
    pub alignment: FontAlignment,
}