use gosub_render_pipeline::common::geo::Rect;
use gosub_render_pipeline::common::media::MediaStore;
use gosub_render_pipeline::painter::commands::brush::Brush;
use vello::peniko::color::{AlphaColor, Rgba8};
use vello::peniko::{Blob, Brush as VelloBrush, ImageAlphaType, ImageBrush, ImageData, ImageFormat, ImageSampler};

pub fn set_brush(brush: &Brush, _rect: Rect, media_store: &MediaStore) -> VelloBrush {
    match brush {
        Brush::Solid(color) => {
            let c = Rgba8::from_u8_array([color.r8(), color.g8(), color.b8(), color.a8()]);
            VelloBrush::Solid(AlphaColor::from(c))
        }
        Brush::Image(media_id) => {
            let media = media_store.get_image(*media_id);
            let image_data = ImageData {
                data: Blob::<u8>::from(media.image.as_raw().to_vec()),
                format: ImageFormat::Rgba8,
                alpha_type: ImageAlphaType::AlphaPremultiplied,
                width: media.image.width(),
                height: media.image.height(),
            };
            VelloBrush::Image(ImageBrush {
                image: image_data,
                sampler: ImageSampler::default(),
            })
        }
    }
}
