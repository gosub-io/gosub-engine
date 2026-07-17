pub mod parley;

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
    /// Extra spacing between characters in px (CSS `letter-spacing`; 0 = `normal`)
    pub letter_spacing: f64,
    pub alignment: FontAlignment,
    pub underline: bool,
    pub line_through: bool,
}
