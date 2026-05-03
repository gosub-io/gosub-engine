use crate::common::geo::Rect;
use crate::common::get_media_store;
use crate::painter::commands::brush::Brush;
use vello::peniko::color::{AlphaColor, Rgba8};
use vello::peniko::Image as PenikoImage;
use vello::peniko::{Blob, Brush as VelloBrush};

pub fn set_brush(brush: &Brush, _rect: Rect) -> VelloBrush {
    match brush {
        Brush::Solid(color) => {
            let c = Rgba8::from_u8_array([color.r8(), color.g8(), color.b8(), color.a8()]);
            VelloBrush::Solid(AlphaColor::from(c))
        }
        Brush::Image(media_id) => {
            let binding = get_media_store();
            let Ok(media_store) = binding.read() else {
                log::warn!("Failed to acquire media store lock, returning transparent brush");
                return VelloBrush::Solid(AlphaColor::from(Rgba8::from_u8_array([0, 0, 0, 0])));
            };
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
