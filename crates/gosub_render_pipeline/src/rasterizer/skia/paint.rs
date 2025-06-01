use skia_safe::{image_filters, AlphaType, Color4f, ColorSpace, ColorType, Data, ISize, ImageInfo, Paint as SkiaPaint};
use crate::common::geo::Dimension;
use crate::common::get_media_store;
use crate::painter::commands::brush::Brush;

// Instead of sending a (skia) Paint object, we encapsulate this, as we might need to store additional information
// in case of an image paint.
pub struct ImagePaint {
    pub paint: SkiaPaint,
    pub dimension: Dimension,
}

pub enum Paint {
    Solid(SkiaPaint),
    Image(ImagePaint),
}

impl Paint {
    pub fn paint(&self) -> &SkiaPaint {
        match self {
            Paint::Solid(p) => p,
            Paint::Image(p) => &p.paint,
        }
    }

    pub fn paint_mut(&mut self) -> &mut SkiaPaint {
        match self {
            Paint::Solid(p) => p,
            Paint::Image(p) => &mut p.paint,
        }
    }
}

pub fn create_paint(brush: &Brush) -> Paint {
    match brush {
        Brush::Solid(color) => {
            // Note: bgra instead of rgba.. Although i'm not sure why, as this does not seem the documented order
            let paint = SkiaPaint::new(Color4f::new(color.b(), color.g(), color.r(), color.a()), &ColorSpace::new_srgb());
            Paint::Solid(paint)
        }
        Brush::Image(media_id) => {
            let binding = get_media_store();
            let media_store = binding.read().expect("Failed to get image store");
            let media = media_store.get_image(*media_id);

            let mut p = SkiaPaint::default();

            let img_info = ImageInfo::new(
                ISize::new(media.image.width() as i32, media.image.height() as i32),
                ColorType::RGBA8888,
                AlphaType::Premul,
                None,
            );

            let skia_img = unsafe {
                skia_safe::images::raster_from_data(
                    &img_info,
                    Data::new_bytes(media.image.to_vec().as_slice()),
                    (img_info.width() * 4) as usize,
                ).unwrap()
            };

            let image_filter = image_filters::image(skia_img, None, None, None);
            p.set_image_filter(image_filter);

            Paint::Image(ImagePaint{
                paint: p,
                dimension: Dimension::new(media.image.width() as f64, media.image.height() as f64),
            })
        }
    }
}