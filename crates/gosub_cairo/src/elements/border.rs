use kurbo::{Arc, BezPath, Cap, Join, Point, Rect, RoundedRectRadii, Stroke, Vec2};
use log::warn;
use smallvec::SmallVec;

use crate::elements::brush::GsBrush;
use crate::elements::transform::GsTransform;
use crate::{CairoBackend, Scene};
use gosub_interface::render_backend::{
    Border as TBorder, BorderRadius as TBorderRadius, BorderSide as TBorderSide, BorderStyle, Radius, RenderBorder,
};
use gosub_shared::geo::FP;

#[derive(Clone, Debug)]
pub struct GsBorder {
    pub(crate) left: Option<GsBorderSide>,
    pub(crate) right: Option<GsBorderSide>,
    pub(crate) top: Option<GsBorderSide>,
    pub(crate) bottom: Option<GsBorderSide>,
}

enum GsSide {
    Left,
    Right,
    Top,
    Bottom,
}

pub struct GsBorderRenderOptions<'a> {
    pub border: &'a RenderBorder<CairoBackend>,
    pub rect: &'a Rect,
    pub transform: Option<&'a GsTransform>,
    pub radius: Option<&'a GsBorderRadius>,
}

#[allow(unused)]
struct GsBorderRenderSideOptions<'a> {
    side: GsSide,
    segment: &'a GsBorderSide,
    transform: Option<&'a GsTransform>,
    radius: Option<(Radius, Radius)>,
    rect: &'a Rect,
}

impl<'a> GsBorderRenderOptions<'a> {
    fn left(&self, transform: Option<&'a GsTransform>) -> Option<GsBorderRenderSideOptions<'_>> {
        let segment = self.border.border.left.as_ref()?;

        Some(GsBorderRenderSideOptions {
            side: GsSide::Left,
            segment,
            transform,
            radius: self.radius.map(|r| (r.top_left, r.bottom_left)),
            rect: self.rect,
        })
    }

    fn right(&self, transform: Option<&'a GsTransform>) -> Option<GsBorderRenderSideOptions<'_>> {
        let segment = self.border.border.right.as_ref()?;

        Some(GsBorderRenderSideOptions {
            side: GsSide::Right,
            segment,
            transform,
            radius: self.radius.map(|r| (r.top_right, r.bottom_right)),
            rect: self.rect,
        })
    }

    fn top(&self, transform: Option<&'a GsTransform>) -> Option<GsBorderRenderSideOptions<'_>> {
        let segment = self.border.border.top.as_ref()?;

        Some(GsBorderRenderSideOptions {
            side: GsSide::Top,
            segment,
            transform,
            radius: self.radius.map(|r| (r.top_left, r.top_right)),
            rect: self.rect,
        })
    }

    fn bottom(&self, transform: Option<&'a GsTransform>) -> Option<GsBorderRenderSideOptions<'_>> {
        let segment = self.border.border.bottom.as_ref()?;

        Some(GsBorderRenderSideOptions {
            side: GsSide::Bottom,
            segment,
            transform,
            radius: self.radius.map(|r| (r.bottom_left, r.bottom_right)),
            rect: self.rect,
        })
    }
}

impl GsBorder {
    pub fn draw(scene: &mut Scene, opts: GsBorderRenderOptions) {
        let transform = match (opts.transform, opts.border.transform.as_ref()) {
            (Some(t1), Some(t2)) => Some(*t1 * *t2),
            (Some(t1), None) => Some(*t1),
            (None, Some(t2)) => Some(*t2),
            (None, None) => None,
        };

        let transform = transform.as_ref();

        if let Some(segment) = opts.left(transform) {
            Self::draw_side(scene, segment);
        }
        if let Some(segment) = opts.right(transform) {
            Self::draw_side(scene, segment);
        }
        if let Some(segment) = opts.top(transform) {
            Self::draw_side(scene, segment);
        }
        if let Some(segment) = opts.bottom(transform) {
            Self::draw_side(scene, segment);
        }
    }

    fn draw_side(_scene: &mut Scene, opts: GsBorderRenderSideOptions) {
        let border_width = opts.segment.width as f64;
        let _brush = &opts.segment.brush;
        let style = opts.segment.style;
        let radius = opts.radius;

        let width = opts.rect.width();
        let height = opts.rect.height();

        let pos = opts.rect.origin();

        let mut path = BezPath::new();

        match opts.side {
            GsSide::Top => {
                match radius {
                    Some((left, right)) => {
                        let offset_left = left.offset();
                        let offset_right = right.offset();

                        path.move_to((pos.x - offset_left.width as f64, pos.y - offset_left.height as f64));

                        let arc = Arc {
                            center: Point::new(pos.x + offset_left.width as f64, pos.y - offset_left.height as f64),
                            radii: Vec2::new(left.radii_f64().0, left.radii_f64().1),
                            start_angle: -std::f64::consts::PI * 3.0 / 4.0,
                            sweep_angle: std::f64::consts::PI / 4.0,
                            x_rotation: 0.0,
                        };

                        arc.to_cubic_beziers(0.1, |p1, p2, p3| {
                            path.curve_to(p1, p2, p3);
                        });

                        path.line_to((
                            pos.x + width - right.radi_x() as f64,
                            pos.y - offset_right.height as f64,
                        ));

                        let arc = Arc {
                            center: Point::new(pos.x + width - right.radi_x() as f64, pos.y + right.radi_y() as f64),
                            radii: Vec2::new(right.radii_f64().0, right.radii_f64().1),
                            start_angle: 0.0,
                            sweep_angle: std::f64::consts::PI / 4.0,
                            x_rotation: 0.0,
                        };

                        arc.to_cubic_beziers(0.1, |p1, p2, p3| {
                            path.curve_to(p1, p2, p3);
                        });
                    }
                    None => {
                        path.move_to((pos.x, pos.y));
                        path.line_to((pos.x + width, pos.y));
                    }
                };
            }
            GsSide::Right => match radius {
                Some((top, bottom)) => {
                    let offset_top = top.offset();
                    let offset_bottom = bottom.offset();

                    path.move_to((
                        pos.x + width + offset_top.width as f64,
                        pos.y - offset_top.height as f64,
                    ));

                    let arc = Arc {
                        center: Point::new(
                            pos.x + width - offset_top.width as f64,
                            pos.y + offset_top.height as f64,
                        ),
                        radii: Vec2::new(top.radii_f64().0, top.radii_f64().1),
                        start_angle: -std::f64::consts::PI / 4.0,
                        sweep_angle: std::f64::consts::PI / 4.0,
                        x_rotation: 0.0,
                    };

                    arc.to_cubic_beziers(0.1, |p1, p2, p3| {
                        path.curve_to(p1, p2, p3);
                    });

                    path.line_to((
                        pos.x + width - offset_bottom.width as f64,
                        pos.y + height - bottom.radi_y() as f64,
                    ));

                    let arc = Arc {
                        center: Point::new(
                            pos.x + width - offset_bottom.width as f64,
                            pos.y + height - offset_bottom.height as f64,
                        ),
                        radii: Vec2::new(bottom.radii_f64().0, bottom.radii_f64().1),
                        start_angle: 0.0,
                        sweep_angle: std::f64::consts::PI / 4.0,
                        x_rotation: 0.0,
                    };

                    arc.to_cubic_beziers(0.1, |p1, p2, p3| {
                        path.curve_to(p1, p2, p3);
                    });
                }
                None => {
                    path.move_to((pos.x + width, pos.y));
                    path.line_to((pos.x + width, pos.y + height));
                }
            },
            GsSide::Bottom => match radius {
                Some((left, right)) => {
                    let offset_left = left.offset();
                    let offset_right = right.offset();

                    path.move_to((
                        pos.x + width + offset_right.width as f64,
                        pos.y + height + offset_right.height as f64,
                    ));

                    let arc = Arc {
                        center: Point::new(
                            pos.x + width - offset_right.width as f64,
                            pos.y + height - offset_right.height as f64,
                        ),
                        radii: Vec2::new(right.radii_f64().0, right.radii_f64().1),
                        start_angle: -std::f64::consts::PI * 7.0 / 4.0,
                        sweep_angle: std::f64::consts::PI / 4.0,
                        x_rotation: 0.0,
                    };

                    arc.to_cubic_beziers(0.1, |p1, p2, p3| {
                        path.curve_to(p1, p2, p3);
                    });

                    path.line_to((pos.x + left.radi_x() as f64, pos.y + height - offset_left.height as f64));

                    let arc = Arc {
                        center: Point::new(pos.x + left.radi_x() as f64, pos.y + height - offset_left.height as f64),
                        radii: Vec2::new(left.radii_f64().0, left.radii_f64().1),
                        start_angle: -std::f64::consts::PI * 3.0 / 2.0,
                        sweep_angle: std::f64::consts::PI / 4.0,
                        x_rotation: 0.0,
                    };

                    arc.to_cubic_beziers(0.1, |p1, p2, p3| {
                        path.curve_to(p1, p2, p3);
                    });
                }
                None => {
                    path.move_to((pos.x, pos.y + height));
                    path.line_to((pos.x + width, pos.y + height));
                }
            },
            GsSide::Left => match radius {
                Some((top, bottom)) => {
                    let offset_top = top.offset();
                    let offset_bottom = bottom.offset();

                    path.move_to((
                        pos.x - offset_top.width as f64,
                        pos.y + height + offset_top.height as f64,
                    ));

                    let arc = Arc {
                        center: Point::new(
                            pos.x + offset_top.width as f64,
                            pos.y + height - offset_top.height as f64,
                        ),
                        radii: Vec2::new(top.radii_f64().0, top.radii_f64().1),
                        start_angle: -std::f64::consts::PI * 5.0 / 4.0,
                        sweep_angle: std::f64::consts::PI / 4.0,
                        x_rotation: 0.0,
                    };

                    arc.to_cubic_beziers(0.1, |p1, p2, p3| {
                        path.curve_to(p1, p2, p3);
                    });

                    path.line_to((pos.x + offset_bottom.width as f64, pos.y + bottom.radi_y() as f64));

                    let arc = Arc {
                        center: Point::new(pos.x + offset_bottom.width as f64, pos.y + bottom.radi_y() as f64),
                        radii: Vec2::new(bottom.radii_f64().0, bottom.radii_f64().1),
                        start_angle: -std::f64::consts::PI,
                        sweep_angle: std::f64::consts::PI / 4.0,
                        x_rotation: 0.0,
                    };

                    arc.to_cubic_beziers(0.1, |p1, p2, p3| {
                        path.curve_to(p1, p2, p3);
                    });
                }
                None => {
                    path.move_to((pos.x, pos.y + height));
                    path.line_to((pos.x, pos.y));
                }
            },
        }

        let cap = match style {
            BorderStyle::Dashed => Cap::Square,
            BorderStyle::Dotted => Cap::Round,
            _ => Cap::Butt,
        };

        let dash_pattern = match style {
            BorderStyle::Dashed => SmallVec::from([
                border_width * 3.0,
                border_width * 3.0,
                border_width * 3.0,
                border_width * 3.0,
            ]),
            BorderStyle::Dotted => {
                SmallVec::from([border_width, border_width, border_width, border_width])
                //TODO: somehow this doesn't result in circles. It is more like a rounded rectangle
            }
            _ => SmallVec::default(),
        };

        let _stroke = Stroke {
            width: border_width,
            join: Join::Bevel,
            miter_limit: 0.0,
            start_cap: cap,
            end_cap: cap,
            dash_pattern,
            dash_offset: 0.0,
        };

        warn!("Border drawing is not implemented yet");
        // scene.stroke(
        //     &stroke,
        //     opts.GsTransform.unwrap_or(&transform::identity()),
        //     GsBrush,
        //     None,
        //     &path,
        // );
    }
}

impl TBorder<CairoBackend> for GsBorder {
    fn new(all: GsBorderSide) -> Self {
        Self {
            left: Some(all.clone()),
            right: Some(all.clone()),
            top: Some(all.clone()),
            bottom: Some(all),
        }
    }

    fn empty() -> Self {
        Self {
            left: None,
            right: None,
            top: None,
            bottom: None,
        }
    }

    fn all(left: GsBorderSide, right: GsBorderSide, top: GsBorderSide, bottom: GsBorderSide) -> Self {
        Self {
            left: Some(left),
            right: Some(right),
            top: Some(top),
            bottom: Some(bottom),
        }
    }

    fn left(&mut self, side: GsBorderSide) {
        self.left = Some(side);
    }

    fn right(&mut self, side: GsBorderSide) {
        self.right = Some(side);
    }

    fn top(&mut self, side: GsBorderSide) {
        self.top = Some(side);
    }

    fn bottom(&mut self, side: GsBorderSide) {
        self.bottom = Some(side);
    }
}

#[derive(Clone, Debug)]
pub struct GsBorderSide {
    pub(crate) width: FP,
    pub(crate) style: BorderStyle,
    pub(crate) brush: GsBrush,
}

impl TBorderSide<CairoBackend> for GsBorderSide {
    fn new(width: FP, style: BorderStyle, brush: GsBrush) -> Self {
        Self { width, style, brush }
    }
}

#[derive(Clone, Debug)]
pub struct GsBorderRadius {
    pub(crate) top_left: Radius,
    pub(crate) top_right: Radius,
    pub(crate) bottom_left: Radius,
    pub(crate) bottom_right: Radius,
}

impl From<[FP; 4]> for GsBorderRadius {
    fn from(value: [FP; 4]) -> Self {
        Self {
            top_left: value[0].into(),
            top_right: value[1].into(),
            bottom_left: value[2].into(),
            bottom_right: value[3].into(),
        }
    }
}

impl From<[FP; 8]> for GsBorderRadius {
    fn from(value: [FP; 8]) -> Self {
        Self {
            top_left: (value[0], value[1]).into(),
            top_right: (value[2], value[3]).into(),
            bottom_left: (value[4], value[5]).into(),
            bottom_right: (value[6], value[7]).into(),
        }
    }
}

impl From<(FP, FP, FP, FP)> for GsBorderRadius {
    fn from(value: (FP, FP, FP, FP)) -> Self {
        Self {
            top_left: value.0.into(),
            top_right: value.1.into(),
            bottom_left: value.2.into(),
            bottom_right: value.3.into(),
        }
    }
}

impl From<(FP, FP, FP, FP, FP, FP, FP, FP)> for GsBorderRadius {
    fn from(value: (FP, FP, FP, FP, FP, FP, FP, FP)) -> Self {
        Self {
            top_left: (value.0, value.1).into(),
            top_right: (value.2, value.3).into(),
            bottom_left: (value.4, value.5).into(),
            bottom_right: (value.6, value.7).into(),
        }
    }
}

impl From<FP> for GsBorderRadius {
    fn from(value: FP) -> Self {
        Self {
            top_left: value.into(),
            top_right: value.into(),
            bottom_left: value.into(),
            bottom_right: value.into(),
        }
    }
}

impl From<Radius> for GsBorderRadius {
    fn from(value: Radius) -> Self {
        Self {
            top_left: value,
            top_right: value,
            bottom_left: value,
            bottom_right: value,
        }
    }
}

impl From<[Radius; 4]> for GsBorderRadius {
    fn from(value: [Radius; 4]) -> Self {
        Self {
            top_left: value[0],
            top_right: value[1],
            bottom_left: value[2],
            bottom_right: value[3],
        }
    }
}

impl From<(Radius, Radius, Radius, Radius)> for GsBorderRadius {
    fn from(value: (Radius, Radius, Radius, Radius)) -> Self {
        Self {
            top_left: value.0,
            top_right: value.1,
            bottom_left: value.2,
            bottom_right: value.3,
        }
    }
}

impl TBorderRadius for GsBorderRadius {
    fn uniform_radius(radius: Radius) -> Self {
        Self {
            top_left: radius,
            top_right: radius,
            bottom_left: radius,
            bottom_right: radius,
        }
    }

    fn all_radius(tl: Radius, tr: Radius, dl: Radius, dr: Radius) -> Self {
        Self {
            top_left: tl,
            top_right: tr,
            bottom_left: dl,
            bottom_right: dr,
        }
    }

    fn top_left_radius(&mut self, radius: Radius) {
        self.top_left = radius;
    }

    fn top_right_radius(&mut self, radius: Radius) {
        self.top_right = radius;
    }

    fn bottom_left_radius(&mut self, radius: Radius) {
        self.bottom_left = radius;
    }

    fn bottom_right_radius(&mut self, radius: Radius) {
        self.bottom_right = radius;
    }
}

impl From<GsBorderRadius> for RoundedRectRadii {
    fn from(value: GsBorderRadius) -> Self {
        RoundedRectRadii::new(
            value.top_left.into(),
            value.top_right.into(),
            value.bottom_right.into(),
            value.bottom_left.into(),
        )
    }
}
