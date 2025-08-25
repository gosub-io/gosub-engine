use crate::CairoBackend;
use gosub_interface::render_backend::{ColorStops, Gradient as TGradient};
use gosub_shared::geo::{Point as GsPoint, FP};
use peniko::color::{AlphaColor, DynamicColor, Srgb};
use peniko::{ColorStop, Gradient as ExtGradient};
use smallvec::SmallVec;

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
    fn new_linear(start: GsPoint, end: GsPoint, stops: ColorStops<CairoBackend>) -> Self {
        let vec = to_stop_vec(stops);
        let g = ExtGradient::new_linear(kurbo_point(start), kurbo_point(end)).with_stops(&*vec);

        GsGradient::new(g)
    }

    fn new_radial_two_point(
        start_center: GsPoint,
        start_radius: FP,
        end_center: GsPoint,
        end_radius: FP,
        stops: ColorStops<CairoBackend>,
    ) -> Self {
        let vec = to_stop_vec(stops);
        let g = ExtGradient::new_two_point_radial(
            kurbo_point(start_center),
            start_radius,
            kurbo_point(end_center),
            end_radius,
        )
        .with_stops(&*vec);

        GsGradient::new(g)
    }

    fn new_radial(center: GsPoint, radius: FP, stops: ColorStops<CairoBackend>) -> Self {
        let vec = to_stop_vec(stops);
        let g = ExtGradient::new_radial(kurbo_point(center), radius).with_stops(&*vec);

        GsGradient::new(g)
    }

    fn new_sweep(center: GsPoint, start_angle: FP, end_angle: FP, stops: ColorStops<CairoBackend>) -> Self {
        let vec = to_stop_vec(stops);
        let g = ExtGradient::new_sweep(kurbo_point(center), start_angle, end_angle).with_stops(&*vec);

        GsGradient::new(g)
    }
}

fn to_stop_vec(stops: ColorStops<CairoBackend>) -> SmallVec<[ColorStop; 4]> {
    let mut vec = SmallVec::<[ColorStop; 4]>::new();

    for stop in &stops {
        let alpha_color = AlphaColor::<Srgb>::new([
            stop.color.r as f32,
            stop.color.g as f32,
            stop.color.b as f32,
            stop.color.a as f32,
        ]);

        vec.push(ColorStop {
            offset: stop.offset,
            color: DynamicColor::from_alpha_color(alpha_color),
        });
    }

    vec
}

fn kurbo_point(point: GsPoint) -> kurbo::Point {
    kurbo::Point::new(point.x.into(), point.y.into())
}
