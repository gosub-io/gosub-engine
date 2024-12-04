use crate::{Brush, Color, Font, Scene, Transform};
use gosub_shared::render_backend::{Brush as _, Color as _, Transform as _};
use gosub_shared::types::Point;

pub fn render_text_simple(scene: &mut Scene, text: &str, point: Point<f32>, font_size: f32) {
    let font = Font {
        family: "Roboto".to_string(),
        slant: cairo::FontSlant::Normal,
        weight: cairo::FontWeight::Normal,
    };

    render_text(
        scene,
        text,
        point,
        font_size,
        &font,
        &Brush::color(Color::BLACK),
    );
}

pub fn render_text<'a>(
    scene: &mut Scene,
    text: &str,
    point: Point<f32>,
    font_size: f32,
    font: &Font,
    brush: &Brush,
) {
    let transform = Transform::translate(point.x.into(), point.y.into());

    render_text_var(
        scene,
        text,
        font_size,
        font,
        brush,
        transform,
        Transform::IDENTITY,
        &[],
    )
}

#[allow(clippy::too_many_arguments)]
pub fn render_text_var<'a>(
    _scene: &mut Scene,
    _text: &str,
    _font_size: f32,
    _font: &Font,
    _brush: &Brush,
    _transform: Transform,
    _glyph_transform: Transform,
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