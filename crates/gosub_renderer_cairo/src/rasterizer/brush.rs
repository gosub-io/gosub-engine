use cairo::Context;
use gosub_render_pipeline::common::geo::Rect;
use gosub_render_pipeline::common::media::MediaStore;
use gosub_render_pipeline::painter::commands::brush::Brush;
use gosub_render_pipeline::painter::commands::gradient::{Gradient, LinearGradient, Tiling};
use std::sync::Arc;

#[allow(unsafe_code)] // an image brush paints from the media store's shared pixels without copying them
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

            let Some(media) = media_store.get_image(*media_id) else {
                return;
            };
            let img = &media.image;

            if img.width() == 0 || img.height() == 0 {
                log::warn!("Image has zero dimensions, skipping image brush");
                return;
            }

            // Premultiplied ARGB32, converted once per image and shared; the surface
            // borrows those bytes and keeps them alive through its user data for as long
            // as the pattern (and the context holding it) refers to them.
            let width = img.width() as i32;
            let height = img.height() as i32;
            let stride = width * 4;
            let data = media.premultiplied_bgra();
            static PIXELS: cairo::UserDataKey<Arc<Vec<u8>>> = cairo::UserDataKey::new();
            // SAFETY: `data` is read-only, lives in the media store, and the surface holds
            // its own reference below; Cairo never writes to a source surface.
            let surface = unsafe {
                cairo::ImageSurface::create_for_data_unsafe(
                    data.as_ptr() as *mut u8,
                    cairo::Format::ARgb32,
                    width,
                    height,
                    stride,
                )
            }
            .and_then(|surface| {
                surface.set_user_data(&PIXELS, std::rc::Rc::new(Arc::clone(&data)))?;
                Ok(surface)
            });

            match surface {
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
    let tile = g.rasterize_tile(tiling);
    let (Ok(tw), Ok(th)) = (i32::try_from(tile.width), i32::try_from(tile.height)) else {
        log::warn!("Gradient tile {}x{} too large for Cairo", tile.width, tile.height);
        return;
    };

    // Straight-alpha RGBA tile → premultiplied ARGB32 (host byte order: BGRA on little-endian).
    let rgba = &tile.rgba;
    let stride = match cairo::Format::ARgb32.stride_for_width(tile.width) {
        Ok(stride) => stride,
        Err(e) => {
            log::warn!("No Cairo stride for a {tw}px gradient tile: {e:?}");
            return;
        }
    };
    let Some(len) = tile_buffer_len(stride, th) else {
        log::warn!("Gradient tile buffer {stride}x{th} overflows; skipping");
        return;
    };
    let mut data = vec![0u8; len];
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
            // A clamped tile is stretched back to `background-size` by `tile.scale`.
            let ox = rect.x + tiling.position.0 as f64;
            let oy = rect.y + tiling.position.1 as f64;
            let (sx, sy) = (1.0 / tile.scale.0, 1.0 / tile.scale.1);
            pattern.set_matrix(cairo::Matrix::new(sx, 0.0, 0.0, sy, -ox * sx, -oy * sy));
            if let Err(e) = cr.set_source(&pattern) {
                log::warn!("Failed to set Cairo tiled-gradient source: {e:?}");
            }
        }
        Err(e) => log::warn!("Failed to create Cairo gradient tile surface: {e:?}"),
    }
}

/// Bytes in a `stride`-wide, `rows`-tall ARGB32 buffer, or `None` if either is negative or the
/// product does not fit (an `i32` multiply would wrap to a small or negative length).
fn tile_buffer_len(stride: i32, rows: i32) -> Option<usize> {
    usize::try_from(stride).ok()?.checked_mul(usize::try_from(rows).ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_buffer_len_multiplies_normal_sizes() {
        assert_eq!(tile_buffer_len(400, 30), Some(12_000));
        assert_eq!(tile_buffer_len(4, 0), Some(0));
    }

    #[test]
    fn tile_buffer_len_rejects_negative_sizes() {
        assert_eq!(tile_buffer_len(-4, 10), None);
        assert_eq!(tile_buffer_len(4, -10), None);
    }

    #[test]
    fn tile_buffer_len_does_not_wrap_like_i32() {
        // 2^20 * 2^12 = 2^32: an i32 product wraps to 0, the widened one is exact.
        assert_eq!((1i32 << 20).wrapping_mul(1 << 12), 0);
        assert_eq!(tile_buffer_len(1 << 20, 1 << 12), usize::try_from(1u64 << 32).ok());
    }
}
