use crate::common::geo::Rect;
use crate::common::media::{get_media_store, Media};
use crate::painter::commands::brush::Brush;
use gtk4::cairo::Context;
use gtk4::gdk_pixbuf::{Colorspace, Pixbuf};
use gtk4::glib::Bytes;
use gtk4::prelude::GdkCairoContextExt;

// Sets the given brush to the context. In case of an image brush, rect defines the scale size of the image.
pub fn set_brush(cr: &Context, brush: &Brush, rect: Rect) {
    match brush {
        Brush::Solid(color) => {
            cr.set_source_rgba(color.r() as f64, color.g() as f64, color.b() as f64, color.a() as f64);
        }
        Brush::Image(media_id) => {
            if rect.width == 0.0 || rect.height == 0.0 {
                return;
            }

            // Look up the image in the global media store
            let store_guard = get_media_store().read().expect("Failed to lock media store");
            let entries = store_guard.entries.read().expect("Failed to lock entries");
            let Some(media) = entries.get(media_id).cloned() else {
                return;
            };
            drop(entries);
            drop(store_guard);

            let Media::Image(media_image) = &*media else {
                return;
            };

            let img = &media_image.image;
            let img_w = img.width();
            let img_h = img.height();

            let bytes = Bytes::from(img.as_raw());
            let pixbuf = Pixbuf::from_bytes(
                &bytes,
                Colorspace::Rgb,
                true,
                8,
                img_w as i32,
                img_h as i32,
                img_w as i32 * 4,
            );

            let scale_x = rect.width / img_w as f64;
            let scale_y = rect.height / img_h as f64;

            let scaled_pixbuf = Pixbuf::new(Colorspace::Rgb, true, 8, rect.width as i32, rect.height as i32).unwrap();
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
