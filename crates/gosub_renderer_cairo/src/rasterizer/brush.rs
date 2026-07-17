use cairo::Context;
use gosub_render_pipeline::common::geo::Rect;
use gosub_render_pipeline::common::media::MediaStore;
use gosub_render_pipeline::painter::commands::brush::Brush;
use gosub_render_pipeline::painter::commands::gradient::{Gradient, LinearGradient, Tiling};

pub fn set_brush(cr: &Context, brush: &Brush, rect: Rect, media_store: &MediaStore) {
    match brush {
        Brush::Solid(color) => {
            cr.set_source_rgba(color.r() as f64, color.g() as f64, color.b() as f64, color.a() as f64);
        }
        Brush::Gradient(Gradient::Linear(g)) => {
            if rect.width == 0.0 || rect.height == 0.0 {
                return;
            }
            // Tiled `background-image` layer (repeated `background-size` cell): rasterize one
            // tile and paint it as a repeating pattern rather than filling the whole box.
            if let Some(tiling) = &g.tiling {
                set_tiled_gradient(cr, g, tiling, rect);
                return;
            }
            let ((x0, y0), (x1, y1)) = g.line(rect.width as f32, rect.height as f32);
            let pattern = cairo::LinearGradient::new(
                rect.x + x0 as f64,
                rect.y + y0 as f64,
                rect.x + x1 as f64,
                rect.y + y1 as f64,
            );
            for stop in &g.stops {
                pattern.add_color_stop_rgba(
                    stop.offset as f64,
                    stop.color.r() as f64,
                    stop.color.g() as f64,
                    stop.color.b() as f64,
                    stop.color.a() as f64,
                );
            }
            if let Err(e) = cr.set_source(&pattern) {
                log::warn!("Failed to set Cairo gradient source: {e:?}");
            }
        }
        Brush::Image(media_id, tiling) => {
            if rect.width == 0.0 || rect.height == 0.0 {
                return;
            }

            let media = media_store.get_image(*media_id);
            let img = &media.image;

            if img.width() == 0 || img.height() == 0 {
                log::warn!("Image has zero dimensions, skipping image brush");
                return;
            }

            // Convert RGBA → premultiplied ARGB32 and paint via a SurfacePattern; cairo scales
            // during compositing, no intermediate copy.
            let width = img.width() as i32;
            let height = img.height() as i32;
            let stride = cairo::Format::ARgb32.stride_for_width(img.width()).unwrap_or(width * 4);

            let mut data = vec![0u8; (stride * height) as usize];
            let src = img.as_raw();
            for row in 0..height as usize {
                for col in 0..width as usize {
                    let si = (row * width as usize + col) * 4;
                    let di = row * stride as usize + col * 4;
                    let r = src[si] as u32;
                    let g = src[si + 1] as u32;
                    let b = src[si + 2] as u32;
                    let a = src[si + 3] as u32;
                    // Premultiplied ARGB32 (host byte order: BGRA on little-endian)
                    data[di] = (b * a / 255) as u8;
                    data[di + 1] = (g * a / 255) as u8;
                    data[di + 2] = (r * a / 255) as u8;
                    data[di + 3] = a as u8;
                }
            }

            match cairo::ImageSurface::create_for_data(data, cairo::Format::ARgb32, width, height, stride) {
                Ok(surface) => {
                    let pattern = cairo::SurfacePattern::create(&surface);
                    // The pattern matrix maps user space → pattern (image pixel) space, so the
                    // translation is expressed in pattern units (pre-scaled by sx/sy).
                    let (sx, sy, ox, oy) = match tiling {
                        // Tiled `background-image`: repeat one `tile_size` (CSS px) cell across the
                        // box, anchored at `background-position`. Nearest keeps tile edges crisp.
                        Some(t) => {
                            pattern.set_filter(cairo::Filter::Nearest);
                            // Cairo's surface extend is 2D; honour full-repeat (default) and no-repeat.
                            let extend = if t.repeat.0 || t.repeat.1 {
                                cairo::Extend::Repeat
                            } else {
                                cairo::Extend::None
                            };
                            pattern.set_extend(extend);
                            (
                                img.width() as f64 / t.tile_size.0 as f64,
                                img.height() as f64 / t.tile_size.1 as f64,
                                rect.x + t.position.0 as f64,
                                rect.y + t.position.1 as f64,
                            )
                        }
                        // Non-tiled: scale the image to fill the whole rect.
                        None => {
                            pattern.set_filter(cairo::Filter::Bilinear);
                            pattern.set_extend(cairo::Extend::Pad);
                            (
                                img.width() as f64 / rect.width,
                                img.height() as f64 / rect.height,
                                rect.x,
                                rect.y,
                            )
                        }
                    };
                    pattern.set_matrix(cairo::Matrix::new(sx, 0.0, 0.0, sy, -ox * sx, -oy * sy));
                    let _ = cr.set_source(&pattern);
                }
                Err(e) => log::warn!("Failed to create Cairo image surface: {e:?}"),
            }
        }
    }
}

/// Rasterize one `background-size` tile and install it as a repeating pattern offset by
/// `background-position`. The caller's already-built fill path clips it to the element box.
fn set_tiled_gradient(cr: &Context, g: &LinearGradient, tiling: &Tiling, rect: Rect) {
    let tw = (tiling.tile_size.0.round() as i32).max(1);
    let th = (tiling.tile_size.1.round() as i32).max(1);

    // Straight-alpha RGBA tile → premultiplied ARGB32 (host byte order: BGRA on little-endian).
    let rgba = g.rasterize_tile(tw as u32, th as u32);
    let stride = cairo::Format::ARgb32.stride_for_width(tw as u32).unwrap_or(tw * 4);
    let mut data = vec![0u8; (stride * th) as usize];
    for row in 0..th as usize {
        for col in 0..tw as usize {
            let si = (row * tw as usize + col) * 4;
            let di = row * stride as usize + col * 4;
            let (r, gg, b, a) = (
                rgba[si] as u32,
                rgba[si + 1] as u32,
                rgba[si + 2] as u32,
                rgba[si + 3] as u32,
            );
            data[di] = (b * a / 255) as u8;
            data[di + 1] = (gg * a / 255) as u8;
            data[di + 2] = (r * a / 255) as u8;
            data[di + 3] = a as u8;
        }
    }

    match cairo::ImageSurface::create_for_data(data, cairo::Format::ARgb32, tw, th, stride) {
        Ok(surface) => {
            let pattern = cairo::SurfacePattern::create(&surface);
            // Nearest keeps the hard tile edges crisp and avoids bleeding across the wrap seam.
            pattern.set_filter(cairo::Filter::Nearest);
            // Cairo's surface extend is 2D; honour full-repeat (the default) and no-repeat.
            // Single-axis repeat (rare) approximates to full repeat.
            let extend = if tiling.repeat.0 || tiling.repeat.1 {
                cairo::Extend::Repeat
            } else {
                cairo::Extend::None
            };
            pattern.set_extend(extend);
            // The pattern matrix maps user space → pattern (tile) space, so anchoring the tile
            // origin at (rect + position) is expressed as the inverse translation.
            let ox = rect.x + tiling.position.0 as f64;
            let oy = rect.y + tiling.position.1 as f64;
            pattern.set_matrix(cairo::Matrix::new(1.0, 0.0, 0.0, 1.0, -ox, -oy));
            if let Err(e) = cr.set_source(&pattern) {
                log::warn!("Failed to set Cairo tiled-gradient source: {e:?}");
            }
        }
        Err(e) => log::warn!("Failed to create Cairo gradient tile surface: {e:?}"),
    }
}
