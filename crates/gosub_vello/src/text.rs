use crate::VelloBackend;
use gosub_render_backend::geo::FP;
use gosub_render_backend::layout::{Decoration, TextLayout};
use gosub_render_backend::{RenderText, Text as TText};
use vello::glyph::Glyph;
use vello::kurbo::{Affine, Line, Stroke};
use vello::peniko::{Brush, Color, Fill, Font, StyleRef};
use vello::skrifa::instance::NormalizedCoord;
use vello::Scene;

pub struct Text {
    glyphs: Vec<Glyph>,
    font: Font,
    fs: FP,
    coords: Vec<NormalizedCoord>,
    decoration: Decoration,
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

        let coords = layout
            .coords()
            .iter()
            .map(|c| NormalizedCoord::from_bits(*c))
            .collect();

        Self {
            glyphs,
            font,
            fs,
            coords,
            decoration: layout.decorations().clone(),
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

        scene
            .draw_glyphs(&render.text.font)
            .font_size(render.text.fs)
            .transform(transform)
            .glyph_transform(brush_transform)
            .normalized_coords(&render.text.coords)
            .brush(brush)
            .draw(style, render.text.glyphs.iter().copied());

        {
            let decoration = &render.text.decoration;

            let stroke = Stroke::new(decoration.width as f64);

            let c = decoration.color;

            let brush = Brush::Solid(Color::rgba(c.0 as f64, c.1 as f64, c.2 as f64, c.3 as f64));

            let offset = decoration.x_offset as f64;

            if decoration.underline {
                let y = y + decoration.underline_offset as f64;

                let line = Line::new((x + offset, y), (x + render.rect.0.width(), y));

                scene.stroke(&stroke, Affine::IDENTITY, &brush, None, &line);
            }

            if decoration.overline {
                let y = y - render.rect.0.height();

                let line = Line::new((x + offset, y), (x + render.rect.0.width(), y));

                scene.stroke(&stroke, Affine::IDENTITY, &brush, None, &line);
            }

            if decoration.line_through {
                let y = y - render.rect.0.height() / 2.0;

                let line = Line::new((x + offset, y), (x + render.rect.0.width(), y));

                scene.stroke(&stroke, Affine::IDENTITY, &brush, None, &line);
            }
        }
    }
}
