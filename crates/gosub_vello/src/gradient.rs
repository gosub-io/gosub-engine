use vello::peniko::{
    ColorStop as VelloColorStop, ColorStops as VelloColorStops, Gradient as VelloGradient,
};

use crate::{Convert, VelloBackend};
use gosub_render_backend::geo::{Point, FP};
use gosub_render_backend::{ColorStop, ColorStops, Gradient as TGradient};

pub struct Gradient(pub(crate) VelloGradient);

impl From<VelloGradient> for Gradient {
    fn from(gradient: VelloGradient) -> Self {
        Gradient(gradient)
    }
}

impl TGradient<VelloBackend> for Gradient {
    fn new_linear(start: Point, end: Point, stops: ColorStops<VelloBackend>) -> Self {
        let mut gradient = VelloGradient::new_linear(start.convert(), end.convert());
        gradient.stops = stops.convert();

        Gradient(gradient)
    }

    fn new_radial_two_point(
        start_center: Point,
        start_radius: FP,
        end_center: Point,
        end_radius: FP,
        stops: ColorStops<VelloBackend>,
    ) -> Self {
        let mut gradient = VelloGradient::new_two_point_radial(
            start_center.convert(),
            start_radius,
            end_center.convert(),
            end_radius,
        );

        gradient.stops = stops.convert();

        Gradient(gradient)
    }

    fn new_radial(center: Point, radius: FP, stops: ColorStops<VelloBackend>) -> Self
    where
        Self: Sized,
    {
        let mut gradient = VelloGradient::new_radial(center.convert(), radius);
        gradient.stops = stops.convert();

        Gradient(gradient)
    }

    fn new_sweep(
        center: Point,
        start_angle: FP,
        end_angle: FP,
        stops: ColorStops<VelloBackend>,
    ) -> Self {
        let mut gradient = VelloGradient::new_sweep(center.convert(), start_angle, end_angle);
        gradient.stops = stops.convert();

        Gradient(gradient)
    }
}

impl Convert<VelloColorStops> for ColorStops<VelloBackend> {
    fn convert(self) -> VelloColorStops {
        let mut stops = VelloColorStops::new();
        for stop in self {
            stops.push(stop.convert());
        }
        stops
    }
}

impl Convert<VelloColorStop> for ColorStop<VelloBackend> {
    fn convert(self) -> VelloColorStop {
        VelloColorStop {
            offset: self.offset,
            color: self.color.0,
        }
    }
}
