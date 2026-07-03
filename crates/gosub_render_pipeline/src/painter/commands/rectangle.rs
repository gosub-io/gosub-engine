use crate::common::geo::Rect;
use crate::painter::commands::border::Border;
use crate::painter::commands::brush::Brush;

/// CSS `mix-blend-mode` for an element's painted box: how its pixels combine with the
/// backdrop already painted beneath it. `Normal` is plain source-over.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BlendMode {
    #[default]
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
}

impl BlendMode {
    /// Parse a CSS `mix-blend-mode` keyword; unknown keywords fall back to `Normal`.
    pub fn from_css_keyword(keyword: &str) -> Self {
        match keyword {
            "multiply" => BlendMode::Multiply,
            "screen" => BlendMode::Screen,
            "overlay" => BlendMode::Overlay,
            "darken" => BlendMode::Darken,
            "lighten" => BlendMode::Lighten,
            "color-dodge" => BlendMode::ColorDodge,
            "color-burn" => BlendMode::ColorBurn,
            "hard-light" => BlendMode::HardLight,
            "soft-light" => BlendMode::SoftLight,
            "difference" => BlendMode::Difference,
            "exclusion" => BlendMode::Exclusion,
            "hue" => BlendMode::Hue,
            "saturation" => BlendMode::Saturation,
            "color" => BlendMode::Color,
            "luminosity" => BlendMode::Luminosity,
            _ => BlendMode::Normal,
        }
    }

    /// Stable small integer for content hashing.
    pub fn id(&self) -> u8 {
        *self as u8
    }
}

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
    blend_mode: BlendMode,
}

impl Rectangle {
    pub fn new(rect: Rect) -> Self {
        Rectangle {
            rect,
            background: None,
            border: Border::new(
                0.0,
                Default::default(),
                [
                    Brush::Solid(Default::default()),
                    Brush::Solid(Default::default()),
                    Brush::Solid(Default::default()),
                    Brush::Solid(Default::default()),
                ],
            ),
            radius_top: Radius::NONE,
            radius_right: Radius::NONE,
            radius_bottom: Radius::NONE,
            radius_left: Radius::NONE,
            blend_mode: BlendMode::Normal,
        }
    }

    pub fn is_rounded(&self) -> bool {
        self.radius_top.x > 0.0
            || self.radius_top.y > 0.0
            || self.radius_right.x > 0.0
            || self.radius_right.y > 0.0
            || self.radius_bottom.x > 0.0
            || self.radius_bottom.y > 0.0
            || self.radius_left.x > 0.0
            || self.radius_left.y > 0.0
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

    pub fn with_blend_mode(mut self, blend_mode: BlendMode) -> Self {
        self.blend_mode = blend_mode;
        self
    }

    pub fn blend_mode(&self) -> BlendMode {
        self.blend_mode
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

    pub fn radius_x(&self) -> (f64, f64, f64, f64) {
        (
            self.radius_top.x,
            self.radius_right.x,
            self.radius_bottom.x,
            self.radius_left.x,
        )
    }

    pub fn radius_y(&self) -> (f64, f64, f64, f64) {
        (
            self.radius_top.y,
            self.radius_right.y,
            self.radius_bottom.y,
            self.radius_left.y,
        )
    }
}
