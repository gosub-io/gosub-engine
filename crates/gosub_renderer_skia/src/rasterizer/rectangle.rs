use gosub_render_pipeline::common::media::{MediaId, MediaStore};
use gosub_render_pipeline::painter::commands::border::BorderStyle;
use gosub_render_pipeline::painter::commands::brush::Brush;
use gosub_render_pipeline::painter::commands::gradient::{Gradient, LinearGradient};
use gosub_render_pipeline::painter::commands::rectangle::{BlendMode as CssBlendMode, Rectangle};
use gosub_render_pipeline::tiler::Tile;
use skia_safe::gradient::{shaders, Colors as GradientColors, Gradient as SkGradient, Interpolation};
use skia_safe::{
    images, AlphaType, BlendMode as SkBlendMode, Canvas, Color, Color4f, ColorType, Data, FilterMode, ISize,
    ImageInfo, MipmapMode, Paint, Point, RRect, Rect, SamplingOptions, TileMode,
};

/// CSS `mix-blend-mode` → Skia paint blend mode. The paint blends against the canvas content
/// already drawn beneath it (the tile backdrop).
fn to_skia_blend_mode(mode: CssBlendMode) -> SkBlendMode {
    match mode {
        CssBlendMode::Normal => SkBlendMode::SrcOver,
        CssBlendMode::Multiply => SkBlendMode::Multiply,
        CssBlendMode::Screen => SkBlendMode::Screen,
        CssBlendMode::Overlay => SkBlendMode::Overlay,
        CssBlendMode::Darken => SkBlendMode::Darken,
        CssBlendMode::Lighten => SkBlendMode::Lighten,
        CssBlendMode::ColorDodge => SkBlendMode::ColorDodge,
        CssBlendMode::ColorBurn => SkBlendMode::ColorBurn,
        CssBlendMode::HardLight => SkBlendMode::HardLight,
        CssBlendMode::SoftLight => SkBlendMode::SoftLight,
        CssBlendMode::Difference => SkBlendMode::Difference,
        CssBlendMode::Exclusion => SkBlendMode::Exclusion,
        CssBlendMode::Hue => SkBlendMode::Hue,
        CssBlendMode::Saturation => SkBlendMode::Saturation,
        CssBlendMode::Color => SkBlendMode::Color,
        CssBlendMode::Luminosity => SkBlendMode::Luminosity,
    }
}

pub fn do_paint_rectangle(canvas: &Canvas, _tile: &Tile, cmd: &Rectangle, media_store: &MediaStore) {
    let r = cmd.rect();

    if let Some(brush) = cmd.background() {
        if let Brush::Image(media_id) = brush {
            // Raster images (`<img>`, background-image) are drawn from their decoded pixels;
            // the other brushes fill a solid/gradient rect.
            draw_image_brush(
                canvas,
                cmd,
                *media_id,
                media_store,
                r.x as f32,
                r.y as f32,
                r.width as f32,
                r.height as f32,
            );
        } else {
            let mut paint = Paint::new(brush_to_color4f(brush), None);
            paint.set_anti_alias(true);
            paint.set_blend_mode(to_skia_blend_mode(cmd.blend_mode()));
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
    }

    let border = cmd.border();
    if !border.is_uniform() {
        paint_per_side_border(canvas, cmd);
    } else if border.width() > 0.0 && !matches!(border.style(), BorderStyle::None | BorderStyle::Hidden) {
        let brush = border.brush();
        let mut paint = Paint::new(brush_to_color4f(&brush), None);
        paint.set_anti_alias(true);
        paint.set_blend_mode(to_skia_blend_mode(cmd.blend_mode()));
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
        (
            r.x as f32 + r.width as f32 - widths[1],
            r.y as f32,
            widths[1],
            r.height as f32,
        ),
        (
            r.x as f32,
            r.y as f32 + r.height as f32 - widths[2],
            r.width as f32,
            widths[2],
        ),
        (r.x as f32, r.y as f32, widths[3], r.height as f32),
    ];

    for i in 0..4 {
        if widths[i] <= 0.0 || styles[i].is_invisible() {
            continue;
        }
        let (x, y, w, h) = edges[i];
        let mut paint = Paint::new(brush_to_color4f(&brushes[i]), None);
        paint.set_anti_alias(true);
        paint.set_blend_mode(to_skia_blend_mode(cmd.blend_mode()));
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

/// Draw a raster image brush (decoded `<img>`/background-image pixels) into the box at
/// `(x, y)`×`w`×`h`. The decoded buffer is unpremultiplied RGBA; Skia scales it to the box with
/// linear sampling, and a rounded box clips the draw to its corner radius.
#[allow(clippy::too_many_arguments)]
fn draw_image_brush(canvas: &Canvas, cmd: &Rectangle, media_id: MediaId, media_store: &MediaStore, x: f32, y: f32, w: f32, h: f32) {
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    let media = media_store.get_image(media_id);
    let img = &media.image;
    let (iw, ih) = (img.width(), img.height());
    if iw == 0 || ih == 0 {
        log::warn!("Image {media_id:?} has zero dimensions, skipping image brush");
        return;
    }

    let info = ImageInfo::new(
        ISize::new(iw as i32, ih as i32),
        ColorType::RGBA8888,
        AlphaType::Unpremul,
        None,
    );
    let row_bytes = iw as usize * 4;
    let Some(image) = images::raster_from_data(&info, Data::new_copy(img.as_raw()), row_bytes) else {
        log::warn!("Failed to build Skia image for {media_id:?}");
        return;
    };

    let dest = Rect::from_xywh(x, y, w, h);
    let sampling = SamplingOptions::new(FilterMode::Linear, MipmapMode::None);
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_blend_mode(to_skia_blend_mode(cmd.blend_mode()));

    if cmd.is_rounded() {
        let (r_tl, ..) = cmd.radius_x();
        canvas.save();
        canvas.clip_rrect(RRect::new_rect_xy(dest, r_tl as f32, r_tl as f32), None, true);
        canvas.draw_image_rect_with_sampling_options(&image, None, dest, sampling, &paint);
        canvas.restore();
    } else {
        canvas.draw_image_rect_with_sampling_options(&image, None, dest, sampling, &paint);
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
    let colors: Vec<Color4f> = g
        .stops
        .iter()
        .map(|s| Color::from_argb(s.color.a8(), s.color.r8(), s.color.g8(), s.color.b8()).into())
        .collect();
    let positions: Vec<f32> = g.stops.iter().map(|s| s.offset).collect();

    let gradient = SkGradient::new(
        GradientColors::new(colors.as_slice(), Some(positions.as_slice()), TileMode::Clamp, None),
        Interpolation::default(),
    );
    let shader = shaders::linear_gradient(
        (Point::new(x + x0, y + y0), Point::new(x + x1, y + y1)),
        &gradient,
        None,
    );
    if let Some(shader) = shader {
        paint.set_shader(shader);
    }
}
