use gosub_render_pipeline::painter::commands::border::BorderStyle;
use gosub_render_pipeline::painter::commands::brush::Brush;
use gosub_render_pipeline::painter::commands::gradient::{Gradient, LinearGradient};
use gosub_render_pipeline::painter::commands::rectangle::Rectangle;
use gosub_render_pipeline::tiler::Tile;
use skia_safe::{gradient_shader, Canvas, Color, Color4f, Paint, Point, Rect, TileMode};

pub fn do_paint_rectangle(canvas: &Canvas, _tile: &Tile, cmd: &Rectangle) {
    let r = cmd.rect();

    if let Some(brush) = cmd.background() {
        let mut paint = Paint::new(brush_to_color4f(brush), None);
        paint.set_anti_alias(true);
        if let Brush::Gradient(Gradient::Linear(g)) = brush {
            apply_linear_gradient(&mut paint, g, r.x as f32, r.y as f32, r.width as f32, r.height as f32);
        }
        draw_rect_or_rounded(
            canvas,
            cmd,
            r.x as f32,
            r.y as f32,
            r.width as f32,
            r.height as f32,
            &paint,
        );
    }

    let border = cmd.border();
    if !border.is_uniform() {
        paint_per_side_border(canvas, cmd);
    } else if border.width() > 0.0 && !matches!(border.style(), BorderStyle::None | BorderStyle::Hidden) {
        let brush = border.brush();
        let mut paint = Paint::new(brush_to_color4f(&brush), None);
        paint.set_anti_alias(true);
        paint.set_stroke_width(border.width());
        paint.set_style(skia_safe::paint::Style::Stroke);
        draw_rect_or_rounded(
            canvas,
            cmd,
            r.x as f32,
            r.y as f32,
            r.width as f32,
            r.height as f32,
            &paint,
        );
    }
}

/// Paints a non-uniform border (e.g. `border-bottom` only) by filling each visible side as a
/// solid edge rectangle. Side order is `[top, right, bottom, left]`.
fn paint_per_side_border(canvas: &Canvas, cmd: &Rectangle) {
    let r = cmd.rect();
    let widths = cmd.border().widths();
    let styles = cmd.border().styles();
    let brushes = cmd.border().brushes();

    let edges = [
        (r.x as f32, r.y as f32, r.width as f32, widths[0]),
        (r.x as f32 + r.width as f32 - widths[1], r.y as f32, widths[1], r.height as f32),
        (r.x as f32, r.y as f32 + r.height as f32 - widths[2], r.width as f32, widths[2]),
        (r.x as f32, r.y as f32, widths[3], r.height as f32),
    ];

    for i in 0..4 {
        if widths[i] <= 0.0 || styles[i].is_invisible() {
            continue;
        }
        let (x, y, w, h) = edges[i];
        let mut paint = Paint::new(brush_to_color4f(&brushes[i]), None);
        paint.set_anti_alias(true);
        paint.set_style(skia_safe::paint::Style::Fill);
        canvas.draw_rect(Rect::from_xywh(x, y, w, h), &paint);
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
        Brush::Image(_) | Brush::Gradient(_) => Color4f::new(1.0, 0.0, 1.0, 1.0),
    }
}

/// Install a linear-gradient shader on `paint` for a box at `(x, y)` of size `w`×`h`.
/// Falls back to leaving the paint's solid colour when the shader can't be built.
fn apply_linear_gradient(paint: &mut Paint, g: &LinearGradient, x: f32, y: f32, w: f32, h: f32) {
    if g.stops.is_empty() {
        return;
    }
    let ((x0, y0), (x1, y1)) = g.line(w, h);
    let colors: Vec<Color> = g
        .stops
        .iter()
        .map(|s| Color::from_argb(s.color.a8(), s.color.r8(), s.color.g8(), s.color.b8()))
        .collect();
    let positions: Vec<f32> = g.stops.iter().map(|s| s.offset).collect();

    let shader = gradient_shader::linear(
        (Point::new(x + x0, y + y0), Point::new(x + x1, y + y1)),
        colors.as_slice(),
        Some(positions.as_slice()),
        TileMode::Clamp,
        None,
        None,
    );
    if let Some(shader) = shader {
        paint.set_shader(shader);
    }
}
