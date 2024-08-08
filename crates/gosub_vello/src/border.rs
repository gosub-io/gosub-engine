use smallvec::SmallVec;
use vello::kurbo::{Arc, BezPath, Cap, Join, RoundedRectRadii, Stroke};
use vello::Scene;

use crate::{Brush, Rect, Transform, VelloBackend};
use gosub_render_backend::geo::FP;
use gosub_render_backend::{
    Border as TBorder, BorderRadius as TBorderRadius, BorderSide as TBorderSide, BorderStyle,
    Radius, RenderBorder,
};

pub struct Border {
    pub(crate) left: Option<BorderSide>,
    pub(crate) right: Option<BorderSide>,
    pub(crate) top: Option<BorderSide>,
    pub(crate) bottom: Option<BorderSide>,
}

enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

pub struct BorderRenderOptions<'a> {
    pub border: &'a RenderBorder<VelloBackend>,
    pub rect: &'a Rect,
    pub transform: Option<&'a Transform>,
    pub radius: Option<&'a BorderRadius>,
}

struct BorderRenderSideOptions<'a> {
    side: Side,
    segment: &'a BorderSide,
    transform: Option<&'a Transform>,
    radius: Option<(Radius, Radius)>,
    rect: &'a Rect,
}

impl<'a> BorderRenderOptions<'a> {
    fn left(&self, transform: Option<&'a Transform>) -> Option<BorderRenderSideOptions> {
        let segment = self.border.border.left.as_ref()?;

        Some(BorderRenderSideOptions {
            side: Side::Left,
            segment,
            transform,
            radius: self.radius.map(|r| (r.top_left, r.bottom_left)),
            rect: self.rect,
        })
    }

    fn right(&self, transform: Option<&'a Transform>) -> Option<BorderRenderSideOptions> {
        let segment = self.border.border.right.as_ref()?;

        Some(BorderRenderSideOptions {
            side: Side::Right,
            segment,
            transform,
            radius: self.radius.map(|r| (r.top_right, r.bottom_right)),
            rect: self.rect,
        })
    }

    fn top(&self, transform: Option<&'a Transform>) -> Option<BorderRenderSideOptions> {
        let segment = self.border.border.top.as_ref()?;

        Some(BorderRenderSideOptions {
            side: Side::Top,
            segment,
            transform,
            radius: self.radius.map(|r| (r.top_left, r.top_right)),
            rect: self.rect,
        })
    }

    fn bottom(&self, transform: Option<&'a Transform>) -> Option<BorderRenderSideOptions> {
        let segment = self.border.border.bottom.as_ref()?;

        Some(BorderRenderSideOptions {
            side: Side::Bottom,
            segment,
            transform,
            radius: self.radius.map(|r| (r.bottom_left, r.bottom_right)),
            rect: self.rect,
        })
    }
}

impl Border {
    pub fn draw(scene: &mut Scene, opts: BorderRenderOptions) {
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

    fn draw_side(scene: &mut Scene, opts: BorderRenderSideOptions) {
        let border_width = opts.segment.width as f64;
        let brush = &opts.segment.brush.0;
        let style = opts.segment.style;
        let radius = opts.radius;

        let width = opts.rect.0.width();
        let height = opts.rect.0.height();

        let pos = opts.rect.0.origin();

        let mut path = BezPath::new();

        match opts.side {
            Side::Top => {
                match radius {
                    Some((left, right)) => {
                        let offset_left = left.offset();
                        let offset_right = right.offset();

                        path.move_to((
                            pos.x - offset_left.width as f64,
                            pos.y - offset_left.height as f64,
                        ));

                        let arc = Arc::new(
                            (
                                pos.x + offset_left.width as f64,
                                pos.y - offset_left.height as f64,
                            ),
                            left.radii_f64(),
                            -std::f64::consts::PI * 3.0 / 4.0,
                            std::f64::consts::PI / 4.0,
                            0.0,
                        );

                        arc.to_cubic_beziers(0.1, |p1, p2, p3| {
                            path.curve_to(p1, p2, p3);
                        });

                        path.line_to((
                            pos.x + width - right.radi_x() as f64,
                            pos.y - offset_right.height as f64,
                        ));

                        let arc = Arc::new(
                            (
                                pos.x + width - right.radi_x() as f64,
                                pos.y + right.radi_y() as f64,
                            ),
                            right.radii_f64(),
                            0.0,
                            std::f64::consts::PI / 4.0,
                            0.0,
                        );

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
            Side::Right => match radius {
                Some((top, bottom)) => {
                    let offset_top = top.offset();
                    let offset_bottom = bottom.offset();

                    path.move_to((
                        pos.x + width + offset_top.width as f64,
                        pos.y - offset_top.height as f64,
                    ));

                    let arc = Arc::new(
                        (
                            pos.x + width - offset_top.width as f64,
                            pos.y + offset_top.height as f64,
                        ),
                        top.radii_f64(),
                        -std::f64::consts::PI / 4.0,
                        std::f64::consts::PI / 4.0,
                        0.0,
                    );

                    arc.to_cubic_beziers(0.1, |p1, p2, p3| {
                        path.curve_to(p1, p2, p3);
                    });

                    path.line_to((
                        pos.x + width - offset_bottom.width as f64,
                        pos.y + height - bottom.radi_y() as f64,
                    ));

                    let arc = Arc::new(
                        (
                            pos.x + width - offset_bottom.width as f64,
                            pos.y + height - offset_bottom.height as f64,
                        ),
                        bottom.radii_f64(),
                        0.0,
                        std::f64::consts::PI / 4.0,
                        0.0,
                    );

                    arc.to_cubic_beziers(0.1, |p1, p2, p3| {
                        path.curve_to(p1, p2, p3);
                    });
                }
                None => {
                    path.move_to((pos.x + width, pos.y));
                    path.line_to((pos.x + width, pos.y + height));
                }
            },
            Side::Bottom => match radius {
                Some((left, right)) => {
                    let offset_left = left.offset();
                    let offset_right = right.offset();

                    path.move_to((
                        pos.x + width + offset_right.width as f64,
                        pos.y + height + offset_right.height as f64,
                    ));

                    let arc = Arc::new(
                        (
                            pos.x + width - offset_right.width as f64,
                            pos.y + height - offset_right.height as f64,
                        ),
                        right.radii_f64(),
                        -std::f64::consts::PI * 7.0 / 4.0,
                        std::f64::consts::PI / 4.0,
                        0.0,
                    );

                    arc.to_cubic_beziers(0.1, |p1, p2, p3| {
                        path.curve_to(p1, p2, p3);
                    });

                    path.line_to((
                        pos.x + left.radi_x() as f64,
                        pos.y + height - offset_left.height as f64,
                    ));

                    let arc = Arc::new(
                        (
                            pos.x + left.radi_x() as f64,
                            pos.y + height - offset_left.height as f64,
                        ),
                        left.radii_f64(),
                        -std::f64::consts::PI * 3.0 / 2.0,
                        std::f64::consts::PI / 4.0,
                        0.0,
                    );

                    arc.to_cubic_beziers(0.1, |p1, p2, p3| {
                        path.curve_to(p1, p2, p3);
                    });
                }
                None => {
                    path.move_to((pos.x, pos.y + height));
                    path.line_to((pos.x + width, pos.y + height));
                }
            },
            Side::Left => match radius {
                Some((top, bottom)) => {
                    let offset_top = top.offset();
                    let offset_bottom = bottom.offset();

                    path.move_to((
                        pos.x - offset_top.width as f64,
                        pos.y + height + offset_top.height as f64,
                    ));

                    let arc = Arc::new(
                        (
                            pos.x + offset_top.width as f64,
                            pos.y + height - offset_top.height as f64,
                        ),
                        top.radii_f64(),
                        -std::f64::consts::PI * 5.0 / 4.0,
                        std::f64::consts::PI / 4.0,
                        0.0,
                    );

                    arc.to_cubic_beziers(0.1, |p1, p2, p3| {
                        path.curve_to(p1, p2, p3);
                    });

                    path.line_to((
                        pos.x + offset_bottom.width as f64,
                        pos.y + bottom.radi_y() as f64,
                    ));

                    let arc = Arc::new(
                        (
                            pos.x + offset_bottom.width as f64,
                            pos.y + bottom.radi_y() as f64,
                        ),
                        bottom.radii_f64(),
                        -std::f64::consts::PI,
                        std::f64::consts::PI / 4.0,
                        0.0,
                    );

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

        let stroke = Stroke {
            width: border_width,
            join: Join::Bevel,
            miter_limit: 0.0,
            start_cap: cap,
            end_cap: cap,
            dash_pattern,
            dash_offset: 0.0,
        };

        scene.stroke(
            &stroke,
            opts.transform.map(|t| t.0).unwrap_or_default(),
            brush,
            None,
            &path,
        );
    }
}

impl TBorder<VelloBackend> for Border {
    fn new(all: BorderSide) -> Self {
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

    fn all(left: BorderSide, right: BorderSide, top: BorderSide, bottom: BorderSide) -> Self {
        Self {
            left: Some(left),
            right: Some(right),
            top: Some(top),
            bottom: Some(bottom),
        }
    }

    fn left(&mut self, side: BorderSide) {
        self.left = Some(side);
    }

    fn right(&mut self, side: BorderSide) {
        self.right = Some(side);
    }

    fn top(&mut self, side: BorderSide) {
        self.top = Some(side);
    }

    fn bottom(&mut self, side: BorderSide) {
        self.bottom = Some(side);
    }
}

#[derive(Clone)]
pub struct BorderSide {
    pub(crate) width: FP,
    pub(crate) style: BorderStyle,
    pub(crate) brush: Brush,
}

impl TBorderSide<VelloBackend> for BorderSide {
    fn new(width: FP, style: BorderStyle, brush: Brush) -> Self {
        Self {
            width,
            style,
            brush,
        }
    }
}

#[derive(Clone)]
pub struct BorderRadius {
    pub(crate) top_left: Radius,
    pub(crate) top_right: Radius,
    pub(crate) bottom_left: Radius,
    pub(crate) bottom_right: Radius,
}

impl From<[FP; 4]> for BorderRadius {
    fn from(value: [FP; 4]) -> Self {
        Self {
            top_left: value[0].into(),
            top_right: value[1].into(),
            bottom_left: value[2].into(),
            bottom_right: value[3].into(),
        }
    }
}

impl From<[FP; 8]> for BorderRadius {
    fn from(value: [FP; 8]) -> Self {
        Self {
            top_left: (value[0], value[1]).into(),
            top_right: (value[2], value[3]).into(),
            bottom_left: (value[4], value[5]).into(),
            bottom_right: (value[6], value[7]).into(),
        }
    }
}

impl From<(FP, FP, FP, FP)> for BorderRadius {
    fn from(value: (FP, FP, FP, FP)) -> Self {
        Self {
            top_left: value.0.into(),
            top_right: value.1.into(),
            bottom_left: value.2.into(),
            bottom_right: value.3.into(),
        }
    }
}

impl From<(FP, FP, FP, FP, FP, FP, FP, FP)> for BorderRadius {
    fn from(value: (FP, FP, FP, FP, FP, FP, FP, FP)) -> Self {
        Self {
            top_left: (value.0, value.1).into(),
            top_right: (value.2, value.3).into(),
            bottom_left: (value.4, value.5).into(),
            bottom_right: (value.6, value.7).into(),
        }
    }
}

impl From<FP> for BorderRadius {
    fn from(value: FP) -> Self {
        Self {
            top_left: value.into(),
            top_right: value.into(),
            bottom_left: value.into(),
            bottom_right: value.into(),
        }
    }
}

impl From<Radius> for BorderRadius {
    fn from(value: Radius) -> Self {
        Self {
            top_left: value,
            top_right: value,
            bottom_left: value,
            bottom_right: value,
        }
    }
}

impl From<[Radius; 4]> for BorderRadius {
    fn from(value: [Radius; 4]) -> Self {
        Self {
            top_left: value[0],
            top_right: value[1],
            bottom_left: value[2],
            bottom_right: value[3],
        }
    }
}

impl From<(Radius, Radius, Radius, Radius)> for BorderRadius {
    fn from(value: (Radius, Radius, Radius, Radius)) -> Self {
        Self {
            top_left: value.0,
            top_right: value.1,
            bottom_left: value.2,
            bottom_right: value.3,
        }
    }
}

impl TBorderRadius for BorderRadius {
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

impl From<BorderRadius> for RoundedRectRadii {
    fn from(value: BorderRadius) -> Self {
        RoundedRectRadii::new(
            value.top_left.into(),
            value.top_right.into(),
            value.bottom_right.into(),
            value.bottom_left.into(),
        )
    }
}
