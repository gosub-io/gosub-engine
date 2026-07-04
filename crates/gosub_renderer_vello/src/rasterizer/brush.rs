use gosub_render_pipeline::common::geo::Rect;
use gosub_render_pipeline::common::media::MediaStore;
use gosub_render_pipeline::painter::commands::brush::Brush;
use gosub_render_pipeline::painter::commands::gradient::Gradient as CssGradient;
use vello::peniko::color::{AlphaColor, DynamicColor, Rgba8};
use vello::peniko::{
    Blob, Brush as VelloBrush, ColorStop, Gradient as VelloGradient, ImageAlphaType, ImageBrush, ImageData,
    ImageFormat, ImageSampler,
};

pub fn set_brush(brush: &Brush, rect: Rect, media_store: &MediaStore) -> VelloBrush {
    match brush {
        Brush::Solid(color) => {
            let c = Rgba8::from_u8_array([color.r8(), color.g8(), color.b8(), color.a8()]);
            VelloBrush::Solid(AlphaColor::from(c))
        }
        Brush::Gradient(CssGradient::Linear(g)) => {
            let ((x0, y0), (x1, y1)) = g.line(rect.width as f32, rect.height as f32);
            let stops: Vec<ColorStop> = g
                .stops
                .iter()
                .map(|s| ColorStop {
                    offset: s.offset,
                    color: DynamicColor::from_alpha_color(AlphaColor::from_rgba8(
                        s.color.r8(),
                        s.color.g8(),
                        s.color.b8(),
                        s.color.a8(),
                    )),
                })
                .collect();
            let gradient = VelloGradient::new_linear(
                (rect.x + x0 as f64, rect.y + y0 as f64),
                (rect.x + x1 as f64, rect.y + y1 as f64),
            )
            .with_stops(stops.as_slice());
            VelloBrush::Gradient(gradient)
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
