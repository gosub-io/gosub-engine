use std::fmt::Error;
use vello::Scene;
use crate::painter::commands::text::Text;
use crate::tiler::Tile;
use crate::common::font::parley::get_parley_layout;
use parley::layout::{GlyphRun, PositionedLayoutItem};
use vello::kurbo::Affine;
use vello::peniko::Fill;
use crate::common::geo::{Dimension, Rect};
use crate::painter::commands::brush::Brush;
use crate::rasterizer::vello::brush::set_brush;

pub fn do_paint_text(scene: &mut Scene,  cmd: &Text, _tile_size: Dimension, affine: Affine) -> Result<(), Error> {
    let layout = get_parley_layout(cmd.text.as_str(), cmd.font_family.as_str(), cmd.font_size, cmd.line_height, cmd.rect.width, cmd.alignment);

    for line in layout.lines() {
        for item in line.items() {
            match item {
                PositionedLayoutItem::GlyphRun(glyph_run) => {
                    render_glyph_run(scene, glyph_run, &cmd.brush, &cmd.rect, affine);
                }
                PositionedLayoutItem::InlineBox(_inline_box) => {
                    todo!("Inline boxes are not supported yet");
                }
            };
        }
    }

    Ok(())
}

fn render_glyph_run(scene: &mut Scene, glyph_run: GlyphRun<[u8;4]>, brush: &Brush, rect: &Rect, affine: Affine) {
    let vello_brush = set_brush(brush, *rect);

    // @TODO: we need font decorations like underline, strike through, maybe sub sup?

    let mut x = glyph_run.offset() + rect.x as f32;
    let y = glyph_run.baseline() + rect.y as f32;
    let run = glyph_run.run();
    let font = run.font();
    let font_size = run.font_size();
    let synthesis = run.synthesis();
    let glyph_xform = synthesis.skew().map(|angle| Affine::skew(angle.to_radians().tan() as f64, 0.0));
    let coords = run.normalized_coords();

    scene
        .draw_glyphs(font)
        .brush(&vello_brush)
        .glyph_transform(glyph_xform)
        .font_size(font_size)
        .hint(true)
        .transform(affine)
        .normalized_coords(coords)
        .draw(
            Fill::NonZero,
            glyph_run.glyphs().map(|glyph| {
                let gx = x + glyph.x;
                let gy = y + glyph.y;
                x += glyph.advance;

                vello::Glyph {
                    id: glyph.id as _,
                    x: gx.round(),
                    y: gy.round(),
                }
            })
        );

        // @TODO: Do strike through
}
