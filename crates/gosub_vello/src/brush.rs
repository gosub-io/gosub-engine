use crate::{Color, Gradient, Image, VelloBackend};
use gosub_render_backend::Brush as TBrush;
use vello::peniko::Brush as VelloBrush;

#[derive(Clone)]
pub struct Brush(pub(crate) VelloBrush);

impl From<VelloBrush> for Brush {
    fn from(brush: VelloBrush) -> Self {
        Brush(brush)
    }
}

impl TBrush<VelloBackend> for Brush {
    fn gradient(gradient: Gradient) -> Self {
        Brush(VelloBrush::Gradient(gradient.0))
    }

    fn color(color: Color) -> Self {
        Brush(VelloBrush::Solid(color.0))
    }

    fn image(image: Image) -> Self {
        Brush(VelloBrush::Image(image.0))
    }
}
