use gosub_pipeline::common::geo::Rect;
use gosub_pipeline::common::media::MediaStore;
use gosub_pipeline::painter::commands::brush::Brush;
use gtk4::cairo::Context;
use gtk4::gdk_pixbuf::{Colorspace, Pixbuf};
use gtk4::glib::Bytes;
use gtk4::prelude::GdkCairoContextExt;

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

            let bytes = Bytes::from(img.as_raw().as_slice());
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

            let Some(scaled_pixbuf) =
                Pixbuf::new(Colorspace::Rgb, true, 8, rect.width as i32, rect.height as i32)
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
                gtk4::gdk_pixbuf::InterpType::Bilinear,
            );

            cr.set_source_pixbuf(&scaled_pixbuf, rect.x, rect.y);
        }
    }
}
