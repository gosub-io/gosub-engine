use gosub_interface::layout::{Decoration, TextLayout as TLayout};
use gosub_shared::font::Font as TFont;
use gosub_shared::font::Glyph;
use gosub_shared::geo::{Point, Size};
use parley::Font as PFont;
use std::fmt;
use std::fmt::{Debug, Formatter};

#[derive(Debug, Clone)]
pub struct Font(pub PFont);

impl TFont for Font {
    fn to_bytes(&self) -> &[u8] {
        self.0.data.data()
    }
}

impl From<Font> for PFont {
    fn from(font: Font) -> Self {
        font.0
    }
}

pub struct TextLayout {
    pub glyphs: Vec<Glyph>,
    pub font: Font,
    pub font_size: f32,
    pub size: Size,
    pub coords: Vec<i16>,
    pub decoration: Decoration,
    pub offset: Point,
}

impl Debug for TextLayout {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("TextLayout")
            .field("size", &self.size)
            .field("font_size", &self.font_size)
            .field("decoration", &self.decoration)
            .field("offset", &self.offset)
            .finish()
    }
}

impl TLayout for TextLayout {
    type Font = Font;

    fn size(&self) -> Size {
        self.size
    }

    fn glyphs(&self) -> &[Glyph] {
        &self.glyphs
    }

    fn font(&self) -> &Self::Font {
        &self.font
    }

    fn font_size(&self) -> f32 {
        self.font_size
    }

    fn coords(&self) -> &[i16] {
        &self.coords
    }

    fn decorations(&self) -> &Decoration {
        &self.decoration
    }

    fn offset(&self) -> Point {
        self.offset
    }
}
