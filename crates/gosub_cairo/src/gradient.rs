use peniko::{Gradient, ColorStops, ColorStop};
use crate::CairoBackend;
use gosub_shared::render_backend::geo::{Point, FP};
use gosub_shared::render_backend::{ColorStops as TColorStops, Gradient as TGradient};

#[derive(Clone)]
pub struct CairoGradient {
    gradient: Gradient,
}

impl CairoGradient {
    pub fn new(gradient: Gradient) -> Self {
        Self { gradient }
    }
}

impl TGradient<CairoBackend> for CairoGradient {
    fn new_linear(start: Point, end: Point, stops: TColorStops<CairoBackend>) -> Self {
        let g = Gradient::new_linear(to_kurbo(start), to_kurbo(end)).with_stops(&*to_stops(stops));
        CairoGradient::new(g)
    }

    fn new_radial_two_point(
        start_center: Point,
        start_radius: FP,
        end_center: Point,
        end_radius: FP,
        stops: TColorStops<CairoBackend>,
    ) -> Self {
        let g = Gradient::new_two_point_radial(
            to_kurbo(start_center),
            start_radius,
            to_kurbo(end_center),
            end_radius
        ).with_stops(&*to_stops(stops.into()));
        CairoGradient::new(g)
    }

    fn new_radial(center: Point, radius: FP, stops: TColorStops<CairoBackend>) -> Self {
        let g = Gradient::new_radial(to_kurbo(center), radius).with_stops(&*to_stops(stops.into()));
        CairoGradient::new(g)
    }

    fn new_sweep(center: Point, start_angle: FP, end_angle: FP, stops: TColorStops<CairoBackend>) -> Self {
        let g = Gradient::new_sweep(to_kurbo(center), start_angle, end_angle).with_stops(&*to_stops(stops.into()));
        CairoGradient::new(g)
    }
}

fn to_kurbo(point: Point) -> kurbo::Point {
    kurbo::Point::new(point.x.into(), point.y.into())
}

fn to_stops(stops: TColorStops<CairoBackend>) -> ColorStops {
    let mut css = ColorStops::new();
    for stop in stops.iter() {
        css.push(
            ColorStop::from((stop.offset.into(), stop.color.color))
        );
    }

    css
}