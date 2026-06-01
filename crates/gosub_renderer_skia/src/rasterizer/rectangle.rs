use gosub_render_pipeline::painter::commands::border::BorderStyle;
use gosub_render_pipeline::painter::commands::brush::Brush;
use gosub_render_pipeline::painter::commands::rectangle::Rectangle;
use gosub_render_pipeline::tiler::Tile;
use skia_safe::{Canvas, Color4f, Paint, Rect};

pub fn do_paint_rectangle(canvas: &Canvas, _tile: &Tile, cmd: &Rectangle) {
    let r = cmd.rect();

    if let Some(brush) = cmd.background() {
        let mut paint = Paint::new(brush_to_color4f(brush), None);
        paint.set_anti_alias(true);
        draw_rect_or_rounded(canvas, cmd, r.x as f32, r.y as f32, r.width as f32, r.height as f32, &paint);
    }

    let border = cmd.border();
    if border.width() > 0.0 && !matches!(border.style(), BorderStyle::None | BorderStyle::Hidden) {
        let brush = border.brush();
        let mut paint = Paint::new(brush_to_color4f(&brush), None);
        paint.set_anti_alias(true);
        paint.set_stroke_width(border.width());
        paint.set_style(skia_safe::paint::Style::Stroke);
        draw_rect_or_rounded(canvas, cmd, r.x as f32, r.y as f32, r.width as f32, r.height as f32, &paint);
    }
}

fn draw_rect_or_rounded(canvas: &Canvas, cmd: &Rectangle, x: f32, y: f32, w: f32, h: f32, paint: &Paint) {
    let skia_rect = Rect::from_xywh(x, y, w, h);
    if cmd.is_rounded() {
        let (r_tl, _r_tr, _r_br, _r_bl) = cmd.radius_x();
        canvas.draw_round_rect(skia_rect, r_tl as f32, r_tl as f32, paint);
    } else {
        canvas.draw_rect(skia_rect, paint);
    }
}

fn brush_to_color4f(brush: &Brush) -> Color4f {
    match brush {
        Brush::Solid(color) => Color4f::new(color.r(), color.g(), color.b(), color.a()),
        Brush::Image(_) => Color4f::new(1.0, 0.0, 1.0, 1.0),
    }
}
