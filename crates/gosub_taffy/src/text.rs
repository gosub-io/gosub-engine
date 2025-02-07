use gosub_interface::layout::{Decoration, FontData, TextLayout as TLayout};
use gosub_shared::font::Glyph;
use gosub_shared::geo::{Point, Size};
use std::fmt;
use std::fmt::{Debug, Formatter};

pub struct TextLayout {
    pub glyphs: Vec<Glyph>,
    pub font_data: FontData,
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
    fn size(&self) -> Size {
        self.size
    }

    fn glyphs(&self) -> &[Glyph] {
        &self.glyphs
    }

    fn font_data(&self) -> &FontData {
        &self.font_data
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
