pub trait Font: Clone {
    fn to_bytes(&self) -> &[u8];
}

pub type GlyphID = u16;

/// A fully positioned glyph
#[derive(Debug)]
pub struct Glyph {
    pub id: GlyphID,
    pub x: f32,
    pub y: f32,
}
