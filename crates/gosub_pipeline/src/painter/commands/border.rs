use crate::painter::commands::brush::Brush;
use crate::painter::commands::Trbl;

#[derive(Clone, Debug)]
pub enum BorderStyle {
    Solid,
    Dashed,
    Dotted,
    Double,
    Groove,
    Ridge,
    Inset,
    Outset,
    None,
    Hidden,
}

impl Default for BorderStyle {
    fn default() -> Self {
        BorderStyle::None
    }
}

#[derive(Clone, Debug)]
pub enum BorderRadius {
    Uniform(f32),
    Elliptical {
        horizontal: f32,
        vertical: f32,
    },
}

#[derive(Clone, Debug)]
pub struct Border {
    width: f32,
    style: BorderStyle,
    brushes: [Brush; 4],
    radius: Option<Trbl<BorderRadius>>,
}

impl Border {
    pub fn new(width: f32, style: BorderStyle, brushes: [Brush; 4]) -> Self {
        Border {
            width,
            style,
            brushes,
            radius: None,
        }
    }

    pub fn with_radius(mut self, radius: BorderRadius) -> Self {
        self.radius = Some(Trbl {
            top: radius.clone(),
            right: radius.clone(),
            bottom: radius.clone(),
            left: radius,
        });
        self
    }

    pub fn with_radius_trbl(mut self, radius: Trbl<BorderRadius>) -> Self {
        self.radius = Some(radius);
        self
    }

    pub fn width(&self) -> f32 {
        self.width
    }

    pub fn style(&self) -> BorderStyle {
        self.style.clone()
    }

    pub fn brushes(&self) -> [Brush; 4] {
        self.brushes.clone()
    }

    pub fn radius(&self) -> Option<Trbl<BorderRadius>> {
        self.radius.clone()
    }
}
