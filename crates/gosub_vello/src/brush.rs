use crate::{Color, Gradient, Image, VelloBackend};
use gosub_interface::render_backend::Brush as TBrush;
use vello::peniko::Brush as VelloBrush;

#[derive(Clone, Debug)]
pub struct Brush(pub(crate) VelloBrush);

impl From<VelloBrush> for Brush {
    fn from(brush: VelloBrush) -> Self {
        Self(brush)
    }
}

impl TBrush<VelloBackend> for Brush {
    fn gradient(gradient: Gradient) -> Self {
        Self(VelloBrush::Gradient(gradient.0))
    }

    fn color(color: Color) -> Self {
        Self(VelloBrush::Solid(color.0))
    }

    fn image(image: Image) -> Self {
        Self(VelloBrush::Image(image.0))
    }
}
