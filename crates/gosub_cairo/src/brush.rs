use crate::{Color, CairoGradient, Image, CairoBackend};
use gosub_shared::render_backend::Brush as TBrush;

#[derive(Clone)]
enum CairoBrush {
    Gradient(CairoGradient),
    Solid(Color),
    Image(Image),
}

#[derive(Clone)]
pub struct Brush {
    brush: CairoBrush,
}

impl Brush {
    pub fn gradient(gradient: CairoGradient) -> Self {
        Brush{
            brush: CairoBrush::Gradient(gradient)
        }
    }

    pub fn solid(color: Color) -> Self {
        Brush{
            brush: CairoBrush::Solid(color)
        }
    }

    pub fn image(image: Image) -> Self {
        Brush {
            brush: CairoBrush::Image(image)
        }
    }
}

impl TBrush<CairoBackend> for Brush {
    fn gradient(gradient: CairoGradient) -> Self {
        Brush::gradient(gradient)
    }

    fn color(color: Color) -> Self {
        Brush::solid(color)
    }

    fn image(image: Image) -> Self {
        Brush::image(image)
    }
}
