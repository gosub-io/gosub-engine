use crate::common::geo::Rect;
use crate::common::get_media_store;
use crate::common::media::{Media, MediaType};
use crate::painter::commands::brush::Brush;
use gtk4::cairo::Context;
use gtk4::gdk_pixbuf::{Colorspace, Pixbuf};
use gtk4::glib::Bytes;
use gtk4::prelude::GdkCairoContextExt;

pub fn set_brush(cr: &Context, brush: &Brush, rect: Rect) {
    match brush {
        Brush::Solid(color) => {
            cr.set_source_rgba(color.r() as f64, color.g() as f64, color.b() as f64, color.a() as f64);
        }
        Brush::Image(media_id) => {
            if rect.width == 0.0 || rect.height == 0.0 {
                return;
            }

            let store = get_media_store();
            let binding = store.read().unwrap();
            let media = binding.get(*media_id, MediaType::Image);
            let Media::Image(img) = &*media else {
                return;
            };

            let bytes = Bytes::from(img.image.as_raw().as_slice());
            let pixbuf = Pixbuf::from_bytes(
                &bytes,
                Colorspace::Rgb,
                true,
                8,
                img.image.width() as i32,
                img.image.height() as i32,
                img.image.width() as i32 * 4,
            );

            let scale_x = rect.width / img.image.width() as f64;
            let scale_y = rect.height / img.image.height() as f64;

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
