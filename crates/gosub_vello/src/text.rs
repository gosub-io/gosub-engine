use gosub_render_backend::geo::FP;
use gosub_render_backend::layout::{Layouter, TextLayout};
use gosub_render_backend::{RenderText, Text as TText};
use vello::glyph::Glyph;
use vello::kurbo::Affine;
use vello::peniko::{Fill, Font, StyleRef};
use vello::skrifa::FontRef;
use vello::Scene;

use crate::VelloBackend;

pub struct Text {
    glyphs: Vec<Glyph>,
    font: Font,
    fs: FP,
}

impl TText for Text {
    type Font = Font;
    fn new<TL: TextLayout>(layout: &TL) -> Self
    where
        TL::Font: Into<Font>,
    {
        let font = layout.font().clone().into();
        let fs = layout.font_size();

        let glyphs = layout
            .glyphs()
            .iter()
            .map(|g| Glyph {
                id: g.id as u32,
                x: g.x,
                y: g.y,
            })
            .collect();

        Self { glyphs, font, fs }
    }
}

impl Text {
    pub(crate) fn show(scene: &mut Scene, render: &RenderText<VelloBackend>) {
        let brush = &render.brush.0;
        let style: StyleRef = Fill::NonZero.into();

        let transform = render.transform.map(|t| t.0).unwrap_or(Affine::IDENTITY);
        let brush_transform = render.brush_transform.map(|t| t.0);

        let x = render.rect.0.x0;
        let y = render.rect.0.y0 + render.rect.0.height();

        let transform = transform.with_translation((x, y).into());

        scene
            .draw_glyphs(&render.text.font)
            .font_size(render.text.fs)
            .transform(transform)
            .glyph_transform(brush_transform)
            .brush(brush)
            .draw(style, render.text.glyphs.iter().copied());
    }
}

fn to_font_ref(font: &Font) -> Option<FontRef<'_>> {
    use vello::skrifa::raw::FileRef;
    let file_ref = FileRef::new(font.data.as_ref()).ok()?;
    match file_ref {
        FileRef::Font(font) => Some(font),
        FileRef::Collection(collection) => collection.get(font.index).ok(),
    }
}
