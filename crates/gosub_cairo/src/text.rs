use crate::{CairoBackend, Scene};
use gosub_shared::render_backend::geo::FP;
use gosub_shared::render_backend::layout::{Decoration, TextLayout};
use gosub_shared::render_backend::{RenderText, Text as TText};
use kurbo::{Line, Stroke};
use peniko::{Brush, Color};
use skrifa::instance::NormalizedCoord;
use gosub_typeface::font::Glyph;

pub struct Font {
    pub(crate) family: String,
    pub(crate) slant: cairo::FontSlant,
    pub(crate) weight: cairo::FontWeight,
}

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

        dbg!(&layout.glyphs());

        // let glyphs = layout
        //     .glyphs()
        //     .iter()
        //     .map(|g| Glyph {
        //         id: g.id as u32,
        //         x: g.x,
        //         y: g.y,
        //     })
        //     .collect();
        //
        let coords = layout.coords().iter().map(|c| NormalizedCoord::from_bits(*c)).collect();

        Self {
            glyphs: vec!(),
            font,
            fs,
            coords,
            decoration: layout.decorations().clone(),
        }
    }
}

impl Text {
    pub(crate) fn show(scene: &mut Scene, render: &RenderText<CairoBackend>) {

        scene.crc.render(|cr| {
            cr.set_font_size(render.text.fs.into());
            cr.select_font_face(
                &render.text.font.family,
                render.text.font.slant,
                render.text.font.weight,
            );
        });

        // let brush = &render.brush;
        // let style: StyleRef = Fill::NonZero.into();
        //
        // let transform = render.transform.map(|t| t).unwrap_or(Transform::IDENTITY);
        // let brush_transform = render.brush_transform.map(|t| t);

        let x = render.rect.x;
        let y = render.rect.y;

        
        // oh fun.. not we need to return a layout from the render function. If that's possible at all...
        let layout = scene.crc.render(|cr| {
            cr.set_font_size(render.text.fs.into());
            pangocairo::functions::create_layout(&cr);
        });
        let markup = "[Gosub Text] :) 🐟";
        layout.set_markup(markup);

        let font_desc = pango::FontDescription::from_string("Sans 12");
        layout.set_font_description(Some(&font_desc));

        scene.crc.render(|cr| {
            cr.move_to(x, y);
            pangocairo::functions::show_layout(&cr, &layout);
        });

        
        // Set decoration (underline, overline, line-through)
        {
            let decoration = &render.text.decoration;
            let _stroke = Stroke::new(decoration.width as f64);

            let c = decoration.color;
            let _brush = Brush::Solid(Color::rgba(c.0 as f64, c.1 as f64, c.2 as f64, c.3 as f64));

            let offset = decoration.x_offset as f64;
            if decoration.underline {
                let y = y + decoration.underline_offset as f64;
                let _line = Line::new((x + offset, y), (x + render.rect.width, y));
                // scene.stroke(&stroke, Affine::IDENTITY, &brush, None, &line);
            }

            if decoration.overline {
                let y = y - render.rect.height;
                let _line = Line::new((x + offset, y), (x + render.rect.width, y));
                // scene.stroke(&stroke, Affine::IDENTITY, &brush, None, &line);
            }

            if decoration.line_through {
                let y = y - render.rect.height / 2.0;

                let _line = Line::new((x + offset, y), (x + render.rect.width, y));
                // scene.stroke(&stroke, Affine::IDENTITY, &brush, None, &line);
            }
        }
    }
}
