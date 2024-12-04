use crate::elements::color::GsColor;
use crate::elements::gradient::GsGradient;
use crate::elements::image::GsImage;
use crate::CairoBackend;
use gosub_shared::render_backend::Brush as TBrush;

#[derive(Clone, Debug)]
pub enum GsBrush {
    Gradient(GsGradient),
    Solid(GsColor),
    Image(GsImage),
}

impl GsBrush {
    pub fn gradient(gradient: GsGradient) -> Self {
        GsBrush::Gradient(gradient)
    }

    pub fn solid(color: GsColor) -> Self {
        GsBrush::Solid(color)
    }

    pub fn image(image: GsImage) -> Self {
        GsBrush::Image(image)
    }

    pub fn render(obj: &GsBrush, cr: cairo::Context) {
        match &obj {
            GsBrush::Solid(c) => {
                cr.set_source_rgba(c.r, c.g, c.b, c.a);
            }
            GsBrush::Gradient(_g) => {
                // unimplemented!("Gradient brush not implemented");
                // let pat = g.create_pattern(cr);
                // cr.set_source(&pat);
            }
            GsBrush::Image(_i) => {
                // unimplemented!("Image brush not implemented");
                // let pat = i.create_pattern(cr);
                // cr.set_source(&pat);
            }
        }
    }
}

impl TBrush<CairoBackend> for GsBrush {
    fn gradient(gradient: GsGradient) -> Self {
        GsBrush::gradient(gradient)
    }
    fn color(color: GsColor) -> Self {
        GsBrush::solid(color)
    }
    fn image(image: GsImage) -> Self {
        GsBrush::image(image)
    }
}
