use gosub_shared::render_backend::geo::{Point, Size, FP};
use gosub_shared::render_backend::{Rect as TRect};
use crate::BorderRadius;

pub struct Rect {
    // X position
    pub(crate) x: f64,
    /// Y position
    pub(crate) y: f64,
    /// Width
    pub(crate) width: f64,
    /// Height
    pub(crate) height: f64,
    /// Rounding radius
    pub(crate) radius: Option<BorderRadius>,
}

impl Rect {
    pub fn new(x: f64, y: f64, width: f64, height: f64, radius: Option<BorderRadius>) -> Self {
        Rect {
            x,
            y,
            width,
            height,
            radius,
        }
    }
}

impl TRect for Rect {
    fn new(x: FP, y: FP, width: FP, height: FP) -> Self {
        Rect::new(
            x as f64,
            y as f64,
            width as f64,
            height as f64,
            None,
        )
    }

    fn from_point(point: Point, size: Size) -> Self {
        TRect::new(point.x, point.y, size.width, size.height)
    }
}
