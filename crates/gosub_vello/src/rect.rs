use gosub_render_backend::geo::{Point, Size, FP};
use gosub_render_backend::Rect as TRect;
use vello::kurbo::Rect as VelloRect;

pub struct Rect(pub(crate) VelloRect);

impl From<VelloRect> for Rect {
    fn from(rect: VelloRect) -> Self {
        Rect(rect)
    }
}

impl TRect for Rect {
    fn new(x: FP, y: FP, width: FP, height: FP) -> Self {
        VelloRect::new(
            x as f64,
            y as f64,
            x as f64 + width as f64,
            y as f64 + height as f64,
        )
        .into()
    }

    fn from_point(point: Point, size: Size) -> Self {
        TRect::new(point.x, point.y, size.width, size.height)
    }
}
