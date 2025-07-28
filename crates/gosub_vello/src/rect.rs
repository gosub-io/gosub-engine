use gosub_interface::render_backend::Rect as TRect;
use gosub_shared::geo::{Point, Size, FP};
use vello::kurbo::Rect as VelloRect;

#[derive(Clone)]
pub struct Rect(pub(crate) VelloRect);

impl From<VelloRect> for Rect {
    fn from(rect: VelloRect) -> Self {
        Rect(rect)
    }
}

impl TRect for Rect {
    fn new(x: FP, y: FP, width: FP, height: FP) -> Self {
        VelloRect::new(
            f64::from(x),
            f64::from(y),
            f64::from(x) + f64::from(width),
            f64::from(y) + f64::from(height),
        )
        .into()
    }

    fn from_point(point: Point, size: Size) -> Self {
        TRect::new(point.x, point.y, size.width, size.height)
    }
}
