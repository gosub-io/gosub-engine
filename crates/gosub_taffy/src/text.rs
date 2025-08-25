use gosub_interface::font::FontBlob;
use gosub_interface::layout::{Decoration, TextLayout as TLayout};
use gosub_shared::font::Glyph;
use gosub_shared::geo::{Point, Size};
use std::fmt;
use std::fmt::{Debug, Formatter};

pub struct TextLayout {
    /// Glyphs of the text that needs to be rendered. Note that glyph-ids are based on the font stored
    /// in the `font_info` field.
    pub glyphs: Vec<Glyph>,
    /// Actual font used for layouting (and thus rendering) of the text.
    pub font_data: FontBlob,
    // Font size of the text
    pub font_size: f32,
    /// Font decorations of the text
    pub decoration: Decoration,
    /// Offset
    pub offset: Point,
    /// Size of the text (?)
    pub size: Size,
    /// Coordinates of the text
    pub coords: Vec<i16>,
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
    fn glyphs(&self) -> &[Glyph] {
        &self.glyphs
    }

    fn font_data(&self) -> &FontBlob {
        &self.font_data
    }

    fn font_size(&self) -> f32 {
        self.font_size
    }

    fn decorations(&self) -> &Decoration {
        &self.decoration
    }

    fn offset(&self) -> Point {
        self.offset
    }

    fn coords(&self) -> &[i16] {
        &self.coords
    }

    fn size(&self) -> Size {
        self.size
    }
}
