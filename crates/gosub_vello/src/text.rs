use crate::VelloBackend;
use gosub_interface::font::FontBlob;
use gosub_interface::layout::{Decoration, TextLayout};
use gosub_interface::render_backend::{RenderText, Text as TText};
use gosub_shared::geo::{NormalizedCoord, Point, FP};
use vello::kurbo::{Affine, Line, Stroke};
use vello::peniko::{Blob, Brush, Color, Fill, Font as PenikoFont, StyleRef};
use vello::Scene;
use vello_encoding::Glyph;

#[derive(Clone, Debug)]
pub struct Text {
    glyphs: Vec<Glyph>,
    fs: FP,
    font_data: FontBlob,
    coords: Vec<NormalizedCoord>,
    decoration: Decoration,
    offset: Point,
}

impl Text {
    pub(crate) fn show(scene: &mut Scene, render: &RenderText<VelloBackend>) {
        let brush = &render.brush.0;
        let style: StyleRef = Fill::NonZero.into();

        let transform = render.transform.map_or(Affine::IDENTITY, |t| t.0);
        let brush_transform = render.brush_transform.map(|t| t.0);

        let x = render.rect.0.x0;
        let y = render.rect.0.y0;

        let transform = transform.with_translation((x, y).into());

        for text in &render.text {
            let transform = transform.then_translate((f64::from(text.offset.x), f64::from(text.offset.y)).into());

            let peniko_font = PenikoFont::new(Blob::new(text.font_data.data.clone()), text.font_data.index);

            scene
                .draw_glyphs(&peniko_font)
                .font_size(text.fs)
                .transform(transform)
                .glyph_transform(brush_transform)
                .normalized_coords(&text.coords)
                .brush(brush)
                .draw(style, text.glyphs.iter().copied());

            {
                let decoration = &text.decoration;

                let stroke = Stroke::new(f64::from(decoration.width));

                let c = decoration.color;

                let brush = Brush::Solid(Color::from_rgba8(
                    (c.0 * 255.0) as u8,
                    (c.1 * 255.0) as u8,
                    (c.2 * 255.0) as u8,
                    (c.3 * 255.0) as u8,
                ));

                let offset = f64::from(decoration.x_offset);

                if decoration.underline {
                    let y = y + f64::from(decoration.underline_offset) + render.rect.0.height();

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

impl TText for Text {
    fn new(layout: &impl TextLayout) -> Self {
        let glyphs = layout
            .glyphs()
            .iter()
            .map(|g| Glyph {
                id: u32::from(g.id),
                x: g.x,
                y: g.y,
            })
            .collect();

        // let coords = layout.coords().iter().map(|c| NormalizedCoord::from(*c)).collect();

        Self {
            glyphs,
            font_data: layout.font_data().clone(),
            fs: layout.font_size(),
            coords: layout.coords().to_vec(),
            decoration: layout.decorations().clone(),
            offset: layout.offset(),
        }
    }
}
