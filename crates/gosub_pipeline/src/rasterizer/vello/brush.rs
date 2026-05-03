use vello::peniko::{Blob, Brush as VelloBrush};
use vello::peniko::color::{AlphaColor, Rgba8};
use crate::common::geo::Rect;
use crate::painter::commands::brush::Brush;
use vello::peniko::Image as PenikoImage;
use crate::common::get_media_store;

pub fn set_brush(brush: &Brush, _rect: Rect) -> VelloBrush {
    match brush {
        Brush::Solid(color) => {
            let c = Rgba8::from_u8_array([color.r8(), color.g8(), color.b8(), color.a8()]);
            VelloBrush::Solid(AlphaColor::from(c))
        }
        Brush::Image(media_id) => {
            let binding = get_media_store();
            let media_store = binding.read().expect("Failed to get image store");
            let media = media_store.get_image(*media_id);

            VelloBrush::Image(PenikoImage::new(
                Blob::<u8>::from(media.image.as_raw().clone()),
                vello::peniko::ImageFormat::Rgba8,
                media.image.width(),
                media.image.height(),
            ))
        }
    }
}