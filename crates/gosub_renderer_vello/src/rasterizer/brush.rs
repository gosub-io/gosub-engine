use gosub_render_pipeline::common::geo::Rect;
use gosub_render_pipeline::common::media::MediaStore;
use gosub_render_pipeline::painter::commands::brush::Brush;
use gosub_render_pipeline::painter::commands::gradient::Gradient as CssGradient;
use vello::kurbo::Affine;
use vello::peniko::color::{AlphaColor, DynamicColor, Rgba8};
use vello::peniko::{
    Blob, Brush as VelloBrush, ColorStop, Gradient as VelloGradient, ImageAlphaType, ImageBrush, ImageData,
    ImageFormat, ImageSampler,
};

/// Build the Vello brush for a paint command, plus the brush transform to pass to
/// `Scene::fill`/`stroke`. Image brushes need one: a Vello image brush paints the raw pixels
/// anchored at the canvas origin, so without a transform the image lands at (0,0) at its
/// natural size and the pad-extend sampler smears its edge pixels across the rest of the
/// shape. The returned transform maps the image onto `rect` (translate + stretch), matching
/// the Cairo/Skia backends' draw-into-dest-rect semantics. Solid/gradient brushes need none.
pub fn set_brush(brush: &Brush, rect: Rect, media_store: &MediaStore) -> (VelloBrush, Option<Affine>) {
    match brush {
        Brush::Solid(color) => {
            let c = Rgba8::from_u8_array([color.r8(), color.g8(), color.b8(), color.a8()]);
            (VelloBrush::Solid(AlphaColor::from(c)), None)
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
            (VelloBrush::Gradient(gradient), None)
        }
        Brush::Image(media_id) => {
            let media = media_store.get_image(*media_id);
            let (iw, ih) = (media.image.width(), media.image.height());
            let image_data = ImageData {
                data: Blob::<u8>::from(media.image.as_raw().to_vec()),
                format: ImageFormat::Rgba8,
                // Decoded `image::RgbaImage` pixels are straight (unpremultiplied) alpha —
                // the same interpretation the Skia backend uses. Declaring them premultiplied
                // renders semi-transparent pixels too bright.
                alpha_type: ImageAlphaType::Alpha,
                width: iw,
                height: ih,
            };
            let transform = (iw > 0 && ih > 0).then(|| {
                Affine::translate((rect.x, rect.y))
                    * Affine::scale_non_uniform(rect.width / iw as f64, rect.height / ih as f64)
            });
            (
                VelloBrush::Image(ImageBrush {
                    image: image_data,
                    sampler: ImageSampler::default(),
                }),
                transform,
            )
        }
    }
}
