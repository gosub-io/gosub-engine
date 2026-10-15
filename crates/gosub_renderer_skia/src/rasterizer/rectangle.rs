use gosub_render_pipeline::common::geo::Rect as GeoRect;
use gosub_render_pipeline::common::media::{MediaId, MediaStore};
use gosub_render_pipeline::painter::commands::border::{per_side_strips, BorderStyle};
use gosub_render_pipeline::painter::commands::brush::Brush;
use gosub_render_pipeline::painter::commands::gradient::{Gradient, LinearGradient, Tiling};
use gosub_render_pipeline::painter::commands::rectangle::{BlendMode as CssBlendMode, Rectangle};
use gosub_render_pipeline::painter::commands::shadow::{blur_reach, RoundedRect, Shadow};
use gosub_render_pipeline::tiler::Tile;
use skia_safe::gradient::{shaders, Colors as GradientColors, Gradient as SkGradient, Interpolation};
use skia_safe::{
    images, AlphaType, BlendMode as SkBlendMode, BlurStyle, Canvas, ClipOp, Color, Color4f, ColorType, Data,
    FilterMode, ISize, ImageInfo, MaskFilter, Matrix, MipmapMode, Paint, PathBuilder, PathFillType, Point, RRect, Rect,
    SamplingOptions, TileMode,
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
    paint_shadows(canvas, cmd.shadows(), false);

    if let Some(brush) = cmd.background() {
        if let Brush::Image(media_id, tiling) = brush {
            draw_image_brush(
                canvas,
                cmd,
                *media_id,
                media_store,
                tiling.as_ref(),
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
                match &g.tiling {
                    Some(tiling) => apply_tiled_gradient(&mut paint, g, tiling, r.x as f32, r.y as f32),
                    None => {
                        apply_linear_gradient(&mut paint, g, r.x as f32, r.y as f32, r.width as f32, r.height as f32)
                    }
                }
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

    paint_shadows(canvas, cmd.shadows(), true);

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
        // Skia centers a stroke on the path; inset by half the border width so
        // the whole border lies inside the border box (a centered stroke bleeds
        // into the neighbouring element and gets painted over - visibly halving
        // collapsed table borders).
        let half = border.width() / 2.0;
        draw_rect_or_rounded(
            canvas,
            cmd,
            r.x as f32 + half,
            r.y as f32 + half,
            (r.width as f32 - border.width()).max(0.0),
            (r.height as f32 - border.width()).max(0.0),
            &paint,
        );
    }
}

/// Paints the outer or the inset ones of `shadows`, last to first so the first listed ends up on
/// top. Skia blurs in CSS pixels: the blur's sigma follows the canvas' dpr scale.
fn paint_shadows(canvas: &Canvas, shadows: &[Shadow], inset: bool) {
    for shadow in shadows.iter().rev().filter(|shadow| shadow.inset == inset) {
        let c = &shadow.color;
        let mut paint = Paint::new(Color4f::new(c.r(), c.g(), c.b(), c.a()), None);
        paint.set_anti_alias(true);
        if shadow.sigma > 0.0 {
            paint.set_mask_filter(MaskFilter::blur(BlurStyle::Normal, shadow.sigma as f32, None));
        }
        canvas.save();
        if inset {
            // Fill everything around the hole, from far enough out that the clip's own edge is
            // past where the blur fades.
            canvas.clip_rrect(sk_rrect(&shadow.clip), ClipOp::Intersect, true);
            let (clip, shape) = (shadow.clip.rect, shadow.shape.rect);
            let reach = blur_reach(shadow.sigma) + 1.0;
            let x = clip.x.min(shape.x) - reach;
            let y = clip.y.min(shape.y) - reach;
            let x2 = (clip.x + clip.width).max(shape.x + shape.width) + reach;
            let y2 = (clip.y + clip.height).max(shape.y + shape.height) + reach;
            let mut path = PathBuilder::new();
            path.set_fill_type(PathFillType::EvenOdd);
            path.add_rect(Rect::new(x as f32, y as f32, x2 as f32, y2 as f32), None, None);
            path.add_rrect(sk_rrect(&shadow.shape), None, None);
            canvas.draw_path(&path.detach(), &paint);
        } else {
            canvas.clip_rrect(sk_rrect(&shadow.clip), ClipOp::Difference, true);
            canvas.draw_rrect(sk_rrect(&shadow.shape), &paint);
        }
        canvas.restore();
    }
}

fn sk_rrect(rounded: &RoundedRect) -> RRect {
    let r = rounded.rect;
    let rect = Rect::from_xywh(r.x as f32, r.y as f32, r.width as f32, r.height as f32);
    RRect::new_rect_radii(
        rect,
        &rounded.radii.map(|radius| Point::new(radius.x as f32, radius.y as f32)),
    )
}

/// Paints a non-uniform border (e.g. `border-bottom` only) by filling each visible side as a
/// solid edge rectangle. Side order is `[top, right, bottom, left]`.
fn paint_per_side_border(canvas: &Canvas, cmd: &Rectangle) {
    let brushes = cmd.border().brushes();
    let strips = per_side_strips(cmd.rect(), cmd.border().widths(), &cmd.border().styles());

    for (i, side) in strips.iter().enumerate() {
        if side.is_empty() {
            continue;
        }
        let mut paint = Paint::new(brush_to_color4f(&brushes[i]), None);
        paint.set_anti_alias(true);
        paint.set_blend_mode(to_skia_blend_mode(cmd.blend_mode()));
        paint.set_style(skia_safe::paint::Style::Fill);
        for strip in side {
            canvas.draw_rect(
                Rect::from_xywh(strip.x as f32, strip.y as f32, strip.width as f32, strip.height as f32),
                &paint,
            );
        }
    }
}

/// `radius_x`/`radius_y` yield corners in CSS order (top-left, top-right, bottom-right,
/// bottom-left), which is also the order Skia's radii array expects.
/// `inset` is how far `rect` sits inside the CSS border box; the radii shrink by the same
/// amount so the outer edge of an inset stroke keeps the box's corner curvature.
fn rounded_rect(cmd: &Rectangle, rect: Rect, inset: f32) -> RRect {
    let (x_tl, x_tr, x_br, x_bl) = cmd.radius_x();
    let (y_tl, y_tr, y_br, y_bl) = cmd.radius_y();
    let r = |v: f64| (v as f32 - inset).max(0.0);
    RRect::new_rect_radii(
        rect,
        &[
            Point::new(r(x_tl), r(y_tl)),
            Point::new(r(x_tr), r(y_tr)),
            Point::new(r(x_br), r(y_br)),
            Point::new(r(x_bl), r(y_bl)),
        ],
    )
}

fn draw_rect_or_rounded(canvas: &Canvas, cmd: &Rectangle, x: f32, y: f32, w: f32, h: f32, paint: &Paint) {
    let skia_rect = Rect::from_xywh(x, y, w, h);
    if cmd.is_rounded() {
        let inset = x - cmd.rect().x as f32;
        canvas.draw_rrect(rounded_rect(cmd, skia_rect, inset), paint);
    } else {
        canvas.draw_rect(skia_rect, paint);
    }
}

/// Draw a raster image brush into the box at `(x, y)`×`w`×`h`. The decoded buffer is
/// unpremultiplied RGBA; a rounded box clips the draw to its corner radius.
#[allow(clippy::too_many_arguments)]
fn draw_image_brush(
    canvas: &Canvas,
    cmd: &Rectangle,
    media_id: MediaId,
    media_store: &MediaStore,
    tiling: Option<&Tiling>,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
) {
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    let Some(media) = media_store.get_image(media_id) else {
        return;
    };
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
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_blend_mode(to_skia_blend_mode(cmd.blend_mode()));

    // Tiled `background-image`: fill the box with a repeating image shader instead of scaling one
    // copy to the box. The image (iw×ih px) is scaled to `tile_size` (CSS px) and repeated, offset
    // by `background-position`.
    if let Some(t) = tiling {
        let sx = t.tile_size.0 / iw as f32;
        let sy = t.tile_size.1 / ih as f32;
        // A tile that does not repeat clamps to its own edge pixels, so linear sampling at the
        // border blends with the image rather than with transparency; the clip to its footprint
        // below keeps the clamped edge from painting past the tile.
        let mode = |repeat: bool| if repeat { TileMode::Repeat } else { TileMode::Clamp };
        let tile_modes = (mode(t.repeat.0), mode(t.repeat.1));
        let mut local = Matrix::translate((x + t.position.0, y + t.position.1));
        local.pre_scale((sx, sy), None);
        // Nearest keeps tile edges crisp and avoids bleeding across the repeat seam. A single
        // tile scaled to its box (`object-fit`, `background-size: cover`) has no seam, and
        // Nearest renders the photo jagged; one drawn at its own size (an icon sprite) keeps
        // Nearest and stays pixel-exact.
        let rescaled = (sx - 1.0).abs() > 1e-3 || (sy - 1.0).abs() > 1e-3;
        let filter = if !t.repeat.0 && !t.repeat.1 && rescaled {
            FilterMode::Linear
        } else {
            FilterMode::Nearest
        };
        let sampling = SamplingOptions::new(filter, MipmapMode::None);
        if let Some(shader) = image.to_shader(tile_modes, sampling, Some(&local)) {
            paint.set_shader(shader);
        }
        let footprint = t.footprint(GeoRect::new(x as f64, y as f64, w as f64, h as f64));
        if let Some(f) = footprint {
            if f.width <= 0.0 || f.height <= 0.0 {
                return;
            }
            canvas.save();
            let clip = Rect::from_xywh(f.x as f32, f.y as f32, f.width as f32, f.height as f32);
            canvas.clip_rect(clip, ClipOp::Intersect, true);
        }
        if cmd.is_rounded() {
            canvas.draw_rrect(rounded_rect(cmd, dest, 0.0), &paint);
        } else {
            canvas.draw_rect(dest, &paint);
        }
        if footprint.is_some() {
            canvas.restore();
        }
        return;
    }

    let sampling = SamplingOptions::new(FilterMode::Linear, MipmapMode::None);
    if cmd.is_rounded() {
        canvas.save();
        canvas.clip_rrect(rounded_rect(cmd, dest, 0.0), None, true);
        canvas.draw_image_rect_with_sampling_options(&image, None, dest, sampling, &paint);
        canvas.restore();
    } else {
        canvas.draw_image_rect_with_sampling_options(&image, None, dest, sampling, &paint);
    }
}

fn brush_to_color4f(brush: &Brush) -> Color4f {
    match brush {
        Brush::Solid(color) => Color4f::new(color.r(), color.g(), color.b(), color.a()),
        Brush::Image(..) | Brush::Gradient(_) => Color4f::new(1.0, 0.0, 1.0, 1.0),
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

/// Install a repeating image shader for a tiled `background-image` gradient layer: rasterize
/// one `background-size` tile and tile it in 2D, offset by `background-position`. The box at
/// `(x, y)` is filled by the shader (the caller draws the rect).
fn apply_tiled_gradient(paint: &mut Paint, g: &LinearGradient, tiling: &Tiling, x: f32, y: f32) {
    let tile = g.rasterize_tile(tiling);
    let (Ok(tw), Ok(th)) = (i32::try_from(tile.width), i32::try_from(tile.height)) else {
        log::warn!("Gradient tile {}x{} too large for Skia", tile.width, tile.height);
        return;
    };
    let info = ImageInfo::new(ISize::new(tw, th), ColorType::RGBA8888, AlphaType::Unpremul, None);
    let row_bytes = tile.width as usize * 4;
    let Some(image) = images::raster_from_data(&info, Data::new_copy(&tile.rgba), row_bytes) else {
        log::warn!("Failed to build Skia gradient tile image");
        return;
    };

    // Full-repeat (the default) tiles both axes; no-repeat clamps to the single tile. Skia
    // tile modes are per-axis, so honour each independently.
    let mode = |repeat: bool| if repeat { TileMode::Repeat } else { TileMode::Decal };
    let tile_modes = (mode(tiling.repeat.0), mode(tiling.repeat.1));
    // A clamped tile is stretched back to `background-size` by `tile.scale`.
    let mut local = Matrix::translate((x + tiling.position.0, y + tiling.position.1));
    local.pre_scale((tile.scale.0 as f32, tile.scale.1 as f32), None);
    // Nearest keeps the tile edges crisp and avoids bleeding across the repeat seam.
    let sampling = SamplingOptions::new(FilterMode::Nearest, MipmapMode::None);
    if let Some(shader) = image.to_shader(tile_modes, sampling, Some(&local)) {
        paint.set_shader(shader);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gosub_render_pipeline::common::media::MediaType;
    use skia_safe::{surfaces, EncodedImageFormat};

    /// A 2x2 opaque red PNG, stored and returned by id.
    fn red_image(store: &MediaStore) -> MediaId {
        let mut surface = surfaces::raster_n32_premul((2, 2)).expect("surface");
        surface.canvas().clear(Color::RED);
        let png = surface
            .image_snapshot()
            .encode(None, EncodedImageFormat::PNG, None)
            .expect("png");
        store
            .load_media_from_data(MediaType::Image, png.as_bytes())
            .expect("decodes")
    }

    #[test]
    fn scaled_single_tile_keeps_its_edge_and_paints_nothing_past_it() {
        let store = MediaStore::new();
        let media_id = red_image(&store);
        let mut surface = surfaces::raster_n32_premul((40, 40)).expect("surface");
        surface.canvas().clear(Color::TRANSPARENT);
        let cmd = Rectangle::new(GeoRect::new(0.0, 0.0, 40.0, 40.0));
        // The 2x2 image scaled to 20x20 (so linear sampling) and placed once at the top-left.
        let tiling = Tiling {
            tile_size: (20.0, 20.0),
            position: (0.0, 0.0),
            repeat: (false, false),
        };
        draw_image_brush(
            surface.canvas(),
            &cmd,
            media_id,
            &store,
            Some(&tiling),
            0.0,
            0.0,
            40.0,
            40.0,
        );
        let snapshot = surface.image_snapshot();
        let pixmap = snapshot.peek_pixels().expect("raster pixels");
        let alpha = |x: i32, y: i32| pixmap.get_color((x, y)).a();
        assert_eq!(alpha(19, 10), 255, "the tile's last column is opaque, not faded");
        assert_eq!(alpha(10, 19), 255, "the tile's last row is opaque, not faded");
        assert_eq!(alpha(20, 10), 0, "nothing right of the tile");
        assert_eq!(alpha(10, 20), 0, "nothing below the tile");
    }
}
