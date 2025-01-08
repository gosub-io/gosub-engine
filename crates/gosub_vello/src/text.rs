use crate::VelloBackend;
use gosub_interface::layout::{Decoration, TextLayout};
use gosub_interface::render_backend::{RenderText, Text as TText};
use gosub_shared::geo::{Point, FP};
use vello::kurbo::{Affine, Line, Stroke};
use vello::peniko::{Brush, Color, Fill, Font, StyleRef};
use vello::skrifa::instance::NormalizedCoord;
use vello::Scene;
use vello_encoding::Glyph;

#[derive(Clone)]
pub struct Text {
    glyphs: Vec<Glyph>,
    font: Font,
    fs: FP,
    coords: Vec<NormalizedCoord>,
    decoration: Decoration,
    offset: Point,
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

        let coords = layout.coords().iter().map(|c| NormalizedCoord::from_bits(*c)).collect();

        Self {
            glyphs,
            font,
            fs,
            coords,
            decoration: layout.decorations().clone(),
            offset: layout.offset(),
        }
    }
}

impl Text {
    pub(crate) fn show(scene: &mut Scene, render: &RenderText<VelloBackend>) {
        let brush = &render.brush.0;
        let style: StyleRef = Fill::NonZero.into();

        let transform = render.transform.map(|t| t.0).unwrap_or(Affine::IDENTITY);
        let brush_transform = render.brush_transform.map(|t| t.0);

        let x = render.rect.0.x0;
        let y = render.rect.0.y0;

        let transform = transform.with_translation((x, y).into());

        for text in &render.text {
            let transform = transform.then_translate((text.offset.x as f64, text.offset.y as f64).into());

            scene
                .draw_glyphs(&text.font)
                .font_size(text.fs)
                .transform(transform)
                .glyph_transform(brush_transform)
                .normalized_coords(&text.coords)
                .brush(brush)
                .draw(style, text.glyphs.iter().copied());

            {
                let decoration = &text.decoration;

                let stroke = Stroke::new(decoration.width as f64);

                let c = decoration.color;

                let brush = Brush::Solid(Color::rgba(c.0 as f64, c.1 as f64, c.2 as f64, c.3 as f64));

                let offset = decoration.x_offset as f64;

                if decoration.underline {
                    let y = y + decoration.underline_offset as f64 + render.rect.0.height();

                    let line = Line::new((x + offset, y), (x + render.rect.0.width(), y));

                    scene.stroke(&stroke, Affine::IDENTITY, &brush, None, &line);
                }

                if decoration.overline {
                    let line = Line::new((x + offset, y), (x + render.rect.0.width(), y));

                    scene.stroke(&stroke, Affine::IDENTITY, &brush, None, &line);
                }

                if decoration.line_through {
                    let y = y + render.rect.0.height() / 2.0;

                    let line = Line::new((x + offset, y), (x + render.rect.0.width(), y));

                    scene.stroke(&stroke, Affine::IDENTITY, &brush, None, &line);
                }
            }
        }
    }
}
