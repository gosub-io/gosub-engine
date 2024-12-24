use crate::CairoBackend;
use gosub_interface::render_backend::{ColorStops, Gradient as TGradient};
use gosub_shared::geo::{Point, FP};
use peniko::{Color as ExtColor, ColorStop as ExtColorStop, ColorStops as ExtColorStops, Gradient as ExtGradient};

#[allow(unused)]
#[derive(Clone, Debug)]
pub struct GsGradient {
    gradient: ExtGradient,
}

impl GsGradient {
    pub fn new(gradient: ExtGradient) -> Self {
        Self { gradient }
    }
}

impl TGradient<CairoBackend> for GsGradient {
    fn new_linear(start: Point, end: Point, stops: ColorStops<CairoBackend>) -> Self {
        let peniko_stops = to_stops(stops);
        let g = ExtGradient::new_linear(to_kurbo(start), to_kurbo(end)).with_stops(peniko_stops.as_slice());
        GsGradient::new(g)
    }

    fn new_radial_two_point(
        start_center: Point,
        start_radius: FP,
        end_center: Point,
        end_radius: FP,
        stops: ColorStops<CairoBackend>,
    ) -> Self {
        let peniko_stops = to_stops(stops);
        let g =
            ExtGradient::new_two_point_radial(to_kurbo(start_center), start_radius, to_kurbo(end_center), end_radius)
                .with_stops(peniko_stops.as_slice());
        GsGradient::new(g)
    }

    fn new_radial(center: Point, radius: FP, stops: ColorStops<CairoBackend>) -> Self {
        let peniko_stops = to_stops(stops);
        let g = ExtGradient::new_radial(to_kurbo(center), radius).with_stops(peniko_stops.as_slice());
        GsGradient::new(g)
    }

    fn new_sweep(center: Point, start_angle: FP, end_angle: FP, stops: ColorStops<CairoBackend>) -> Self {
        let peniko_stops = to_stops(stops);
        let g = ExtGradient::new_sweep(to_kurbo(center), start_angle, end_angle).with_stops(peniko_stops.as_slice());
        GsGradient::new(g)
    }
}

fn to_kurbo(point: Point) -> kurbo::Point {
    kurbo::Point::new(point.x.into(), point.y.into())
}

fn to_stops(stops: ColorStops<CairoBackend>) -> ExtColorStops {
    let mut css = ExtColorStops::new();

    for stop in stops.iter() {
        css.push(ExtColorStop::from((
            stop.offset,
            ExtColor::from_rgba8(stop.color.r8(), stop.color.g8(), stop.color.b8(), stop.color.a8()),
        )));
    }

    css
}
