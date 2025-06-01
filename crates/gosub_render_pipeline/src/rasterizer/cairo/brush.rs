use gtk4::cairo::Context;
use gtk4::gdk_pixbuf::{Colorspace, Pixbuf};
use gtk4::glib::Bytes;
use gtk4::prelude::GdkCairoContextExt;
use crate::common::geo::Rect;
use crate::painter::commands::brush::Brush;

// Sets the given brush to the context. In case of an image brush, rect defines the scale size of the image.
pub fn set_brush(cr: &Context, brush: &Brush, rect: Rect) {
    match brush {
        Brush::Solid(color) => {
            cr.set_source_rgba(color.r() as f64, color.g() as f64, color.b() as f64, color.a() as f64);
        }
        Brush::Image(img) => {
            // If the rect has no width or height, we do not need to draw the image. So we can leave the brush as-is.
            if rect.width == 0.0 || rect.height == 0.0 {
                return;
            }

            let bytes = Bytes::from(img.data());
            let pixbuf = Pixbuf::from_bytes(&bytes, Colorspace::Rgb, true, 8, img.width() as i32, img.height() as i32, img.width() as i32 * 4);

            let scale_x = rect.width / img.width() as f64;
            let scale_y = rect.height / img.height() as f64;

            // Create a scaled version of the image. This does not really sound like a good idea, but i have to find better ways to deal
            // with scaled images.
            let scaled_pixbuf = Pixbuf::new(Colorspace::Rgb, true, 8, rect.width as i32, rect.height as i32).unwrap();
            pixbuf.scale(&scaled_pixbuf,
                0,0,
                rect.width as i32, rect.height as i32,
                0.0, 0.0,
                scale_x, scale_y,
                gtk4::gdk_pixbuf::InterpType::Bilinear);

            cr.set_source_pixbuf(&scaled_pixbuf, rect.x, rect.y);
        }
    }
}