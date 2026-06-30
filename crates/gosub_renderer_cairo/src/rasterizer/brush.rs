use cairo::Context;
use gosub_render_pipeline::common::geo::Rect;
use gosub_render_pipeline::common::media::MediaStore;
use gosub_render_pipeline::painter::commands::brush::Brush;

pub fn set_brush(cr: &Context, brush: &Brush, rect: Rect, media_store: &MediaStore) {
    match brush {
        Brush::Solid(color) => {
            cr.set_source_rgba(color.r() as f64, color.g() as f64, color.b() as f64, color.a() as f64);
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

            #[cfg(feature = "text_pango")]
            {
                use gtk4::gdk_pixbuf::{Colorspace, InterpType, Pixbuf};
                use gtk4::glib::Bytes;
                use gtk4::prelude::GdkCairoContextExt;

                let bytes = Bytes::from(img.as_raw());
                let pixbuf = Pixbuf::from_bytes(
                    &bytes,
                    Colorspace::Rgb,
                    true,
                    8,
                    img.width() as i32,
                    img.height() as i32,
                    img.width() as i32 * 4,
                );

                let scale_x = rect.width / img.width() as f64;
                let scale_y = rect.height / img.height() as f64;

                let Some(scaled_pixbuf) = Pixbuf::new(Colorspace::Rgb, true, 8, rect.width as i32, rect.height as i32)
                else {
                    log::warn!(
                        "Failed to create scaled pixbuf for dimensions {}x{}",
                        rect.width,
                        rect.height
                    );
                    return;
                };
                pixbuf.scale(
                    &scaled_pixbuf,
                    0,
                    0,
                    rect.width as i32,
                    rect.height as i32,
                    0.0,
                    0.0,
                    scale_x,
                    scale_y,
                    InterpType::Bilinear,
                );
                cr.set_source_pixbuf(&scaled_pixbuf, rect.x, rect.y);
            }

            #[cfg(not(feature = "text_pango"))]
            {
                // Cairo-native path: convert RGBA → premultiplied ARGB32 and paint via SurfacePattern.
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
                        let matrix = cairo::Matrix::new(
                            img.width() as f64 / rect.width,
                            0.0,
                            0.0,
                            img.height() as f64 / rect.height,
                            -rect.x,
                            -rect.y,
                        );
                        pattern.set_matrix(matrix);
                        let _ = cr.set_source(&pattern);
                    }
                    Err(e) => log::warn!("Failed to create Cairo image surface: {e:?}"),
                }
            }
        }
    }
}
