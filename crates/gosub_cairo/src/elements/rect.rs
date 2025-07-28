use crate::elements::border::GsBorderRadius;
use crate::elements::brush::GsBrush;
use crate::CairoBackend;
use gosub_interface::render_backend::{Rect as TRect, RenderRect};
use gosub_shared::geo::{Point, Size, FP};

#[derive(Clone, Debug)]
pub struct GsRect {
    // X position
    pub(crate) x: f64,
    /// Y position
    pub(crate) y: f64,
    /// Width
    pub(crate) width: f64,
    /// Height
    pub(crate) height: f64,
    /// Rounding radius
    #[allow(unused)]
    pub(crate) radius: Option<GsBorderRadius>,
}

impl GsRect {
    pub fn new(x: f64, y: f64, width: f64, height: f64, radius: Option<GsBorderRadius>) -> Self {
        GsRect {
            x,
            y,
            width,
            height,
            radius,
        }
    }

    pub(crate) fn render(obj: &RenderRect<CairoBackend>, cr: &cairo::Context) {
        // info!(target: "cairo", "GsRect::render");

        let x = obj.rect.x;
        let y = obj.rect.y;
        let width = obj.rect.width;
        let height = obj.rect.height;

        GsBrush::render(&obj.brush, cr);

        cr.move_to(x, y);
        cr.rectangle(x, y, width, height);
        _ = cr.fill();
    }
}

impl TRect for GsRect {
    fn new(x: FP, y: FP, width: FP, height: FP) -> Self {
        GsRect::new(f64::from(x), f64::from(y), f64::from(width), f64::from(height), None)
    }

    fn from_point(point: Point, size: Size) -> Self {
        TRect::new(point.x, point.y, size.width, size.height)
    }
}
