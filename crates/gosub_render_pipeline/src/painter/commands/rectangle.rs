use crate::common::geo::Rect;
use crate::painter::commands::border::Border;
use crate::painter::commands::brush::Brush;

#[derive(Clone, Debug, Copy)]
pub struct Radius {
    pub x: f64,
    pub y: f64,
}

impl Default for Radius {
    fn default() -> Self {
        Radius { x: 0.0, y: 0.0 }
    }
}

impl Radius {
    pub const NONE: Radius = Radius { x: 0.0, y: 0.0 };

    pub fn new(radius: f64) -> Self {
        Radius { x: radius, y: radius }
    }

    pub fn new_double(x: f64, y: f64) -> Self {
        Radius { x, y }
    }

    pub fn as_64(&self) -> (f64, f64) {
        (self.x, self.y)
    }
}

#[derive(Clone, Debug)]
pub struct Rectangle {
    rect: Rect,
    background: Option<Brush>,
    border: Border,
    radius_top: Radius,
    radius_right: Radius,
    radius_bottom: Radius,
    radius_left: Radius,
}

impl Rectangle {
    pub fn new(rect: Rect) -> Self {
        Rectangle {
            rect,
            background: None,
            border: Border::new(0.0, Default::default(), [
                Brush::Solid(Default::default()),
                Brush::Solid(Default::default()),
                Brush::Solid(Default::default()),
                Brush::Solid(Default::default()),
            ]
            ),
            radius_top: Radius::NONE,
            radius_right: Radius::NONE,
            radius_bottom: Radius::NONE,
            radius_left: Radius::NONE,
        }
    }

    pub(crate) fn is_rounded(&self) -> bool {
        self.radius_top.x > 0.0 || self.radius_right.x > 0.0 || self.radius_bottom.x > 0.0 || self.radius_left.x > 0.0
    }

    pub fn with_radius_tlrb(mut self, top: Radius, right: Radius, bottom: Radius, left: Radius) -> Self {
        self.radius_top = top;
        self.radius_right = right;
        self.radius_bottom = bottom;
        self.radius_left = left;
        self
    }

    pub fn with_radius(mut self, radius: Radius) -> Self {
        self.radius_top = radius;
        self.radius_right = radius;
        self.radius_bottom = radius;
        self.radius_left = radius;
        self
    }

    pub fn with_background(mut self, brush: Brush) -> Self {
        self.background = Some(brush);
        self
    }

    pub fn with_border(mut self, border: Border) -> Self {
        self.border = border;
        self
    }

    pub fn rect(&self) -> Rect {
        self.rect
    }

    pub fn background(&self) -> Option<&Brush> {
        self.background.as_ref()
    }

    pub fn border(&self) -> &Border {
        &self.border
    }

    pub fn radius(&self) -> (Radius, Radius, Radius, Radius) {
        (self.radius_top, self.radius_right, self.radius_bottom, self.radius_left)
    }
}
