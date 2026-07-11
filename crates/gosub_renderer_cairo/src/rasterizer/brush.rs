use cairo::Context;
use gosub_render_pipeline::common::geo::Rect;
use gosub_render_pipeline::common::media::MediaStore;
use gosub_render_pipeline::painter::commands::brush::Brush;
use gosub_render_pipeline::painter::commands::gradient::Gradient;

pub fn set_brush(cr: &Context, brush: &Brush, rect: Rect, media_store: &MediaStore) {
    match brush {
        Brush::Solid(color) => {
            cr.set_source_rgba(color.r() as f64, color.g() as f64, color.b() as f64, color.a() as f64);
        }
        Brush::Gradient(Gradient::Linear(g)) => {
            if rect.width == 0.0 || rect.height == 0.0 {
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
        Brush::Image(media_id) => {
            if rect.width == 0.0 || rect.height == 0.0 {
                return;
            }

            let media = media_store.get_image(*media_id);
            let img = &media.image;

            if img.width() == 0 || img.height() == 0 {
                log::warn!("Image has zero dimensions, skipping image brush");
                return;
            }

            // Convert RGBA → premultiplied ARGB32 and paint via a bilinear-filtered
            // SurfacePattern; cairo scales during compositing, no intermediate copy.
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
                    pattern.set_filter(cairo::Filter::Bilinear);
                    pattern.set_extend(cairo::Extend::Pad);
                    // The pattern matrix maps user space → pattern space, so the translation
                    // to the rect origin must be expressed in pattern units (pre-scaled).
                    let sx = img.width() as f64 / rect.width;
                    let sy = img.height() as f64 / rect.height;
                    pattern.set_matrix(cairo::Matrix::new(sx, 0.0, 0.0, sy, -rect.x * sx, -rect.y * sy));
                    let _ = cr.set_source(&pattern);
                }
                Err(e) => log::warn!("Failed to create Cairo image surface: {e:?}"),
            }
        }
    }
}
