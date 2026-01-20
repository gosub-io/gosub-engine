use crate::elements::color::GsColor;
use crate::elements::gradient::GsGradient;
use crate::elements::image::GsImage;
use crate::CairoBackend;
use gosub_interface::render_backend::Brush as TBrush;

#[derive(Clone, Debug)]
pub enum GsBrush {
    Gradient(GsGradient),
    Solid(GsColor),
    Image(GsImage),
}

impl GsBrush {
    pub const fn gradient(gradient: GsGradient) -> Self {
        Self::Gradient(gradient)
    }

    pub const fn solid(color: GsColor) -> Self {
        Self::Solid(color)
    }

    pub const fn image(image: GsImage) -> Self {
        Self::Image(image)
    }

    pub fn render(obj: &Self, cr: &cairo::Context) {
        match &obj {
            Self::Solid(c) => {
                cr.set_source_rgba(c.r, c.g, c.b, c.a);
            }
            Self::Gradient(_g) => {
                // unimplemented!("Gradient brush not implemented");
                // let pat = g.create_pattern(cr);
                // cr.set_source(&pat);
            }
            Self::Image(_i) => {
                // unimplemented!("Image brush not implemented");
                // let pat = i.create_pattern(cr);
                // cr.set_source(&pat);
            }
        }
    }
}

impl TBrush<CairoBackend> for GsBrush {
    fn gradient(gradient: GsGradient) -> Self {
        Self::gradient(gradient)
    }
    fn color(color: GsColor) -> Self {
        Self::solid(color)
    }
    fn image(image: GsImage) -> Self {
        Self::image(image)
    }
}
