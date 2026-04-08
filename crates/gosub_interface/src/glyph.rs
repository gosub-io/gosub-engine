pub type GlyphID = u16;

/// A fully positioned glyph
#[derive(Debug, Clone)]
pub struct Glyph {
    pub id: GlyphID,
    pub x: f32,
    pub y: f32,
}
