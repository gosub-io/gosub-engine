use crate::elements::brush::GsBrush;
use crate::elements::color::GsColor;
use crate::elements::transform::GsTransform;
use crate::Scene;
use gosub_interface::render_backend::{Brush as _, Color as _, Transform as _};
use gosub_shared::types::Point;
use gosub_shared::ROBOTO_FONT;
use peniko::{Blob, Font};
use std::sync::{Arc, LazyLock};

static FONT: LazyLock<Font> = LazyLock::new(|| Font::new(Blob::new(Arc::new(ROBOTO_FONT)), 0));

pub fn render_text_simple(scene: &mut Scene, text: &str, point: Point<f32>, font_size: f32) {
    render_text(scene, text, point, font_size, &FONT, &GsBrush::color(GsColor::BLACK));
}

pub fn render_text(scene: &mut Scene, text: &str, point: Point<f32>, font_size: f32, font: &Font, brush: &GsBrush) {
    let transform = GsTransform::translate(point.x.into(), point.y.into());

    render_text_var(
        scene,
        text,
        font_size,
        font,
        brush,
        transform,
        GsTransform::IDENTITY,
        &[],
    )
}

#[allow(clippy::too_many_arguments)]
pub fn render_text_var(
    _scene: &mut Scene,
    _text: &str,
    _font_size: f32,
    _font: &Font,
    _brush: &GsBrush,
    _transform: GsTransform,
    _glyph_transform: GsTransform,
    _vars: &[(&str, f32)],
) {
    todo!("Implement this function");

    // let brush: BrushRef = (&brush.0).into();
    // let style = style.into();
    // let axes = font_ref.axes();
    // let var_loc = axes.location(vars.iter().copied());
    // let charmap = font_ref.charmap();
    //
    // let fs = skrifa::instance::Size::new(font_size);
    //
    // let metrics = font_ref.metrics(fs, &var_loc);
    // let line_height = metrics.ascent - metrics.descent + metrics.leading;
    // let glyph_metrics = font_ref.glyph_metrics(fs, &var_loc);
    // let mut pen_x = 0f32;
    // let mut pen_y = 0f32;
    // scene
    //     .0
    //     .draw_glyphs(font)
    //     .font_size(font_size)
    //     .transform(transform.0)
    //     .glyph_transform(Some(glyph_transform.0))
    //     .normalized_coords(var_loc.coords())
    //     .brush(brush)
    //     .hint(false)
    //     .draw(
    //         style,
    //         text.chars().filter_map(|ch| {
    //             if ch == '\n' {
    //                 pen_y += line_height;
    //                 pen_x = 0.0;
    //                 return None;
    //             }
    //             let gid = charmap.map(ch).unwrap_or_default();
    //             let advance = glyph_metrics.advance_width(gid).unwrap_or_default();
    //             let x = pen_x;
    //             pen_x += advance;
    //             Some(Glyph {
    //                 id: gid.to_u16() as u32,
    //                 x,
    //                 y: pen_y,
    //             })
    //         }),
    //     );
}
