use crate::painter::commands::brush::Brush;
use crate::painter::commands::Trbl;

#[derive(Clone, Debug, Default, PartialEq)]
pub enum BorderStyle {
    Solid,
    Dashed,
    Dotted,
    Double,
    Groove,
    Ridge,
    Inset,
    Outset,
    #[default]
    None,
    Hidden,
}

impl BorderStyle {
    /// True when this side paints nothing.
    pub fn is_invisible(&self) -> bool {
        matches!(self, BorderStyle::None | BorderStyle::Hidden)
    }
}

#[derive(Clone, Debug)]
pub enum BorderRadius {
    Uniform(f32),
    Elliptical { horizontal: f32, vertical: f32 },
}

#[derive(Clone, Debug)]
pub struct Border {
    width: f32,
    style: BorderStyle,
    /// Per-side widths in `[top, right, bottom, left]` order (matching `brushes`).
    widths: [f32; 4],
    /// Per-side styles in `[top, right, bottom, left]` order.
    styles: [BorderStyle; 4],
    brushes: [Brush; 4],
    radius: Option<Trbl<BorderRadius>>,
}

impl Border {
    /// A uniform border: same width and style on all four sides.
    pub fn new(width: f32, style: BorderStyle, brushes: [Brush; 4]) -> Self {
        Border {
            width,
            widths: [width; 4],
            styles: [style.clone(), style.clone(), style.clone(), style.clone()],
            style,
            brushes,
            radius: None,
        }
    }

    /// A border with independent per-side widths and styles (`[top, right, bottom, left]`).
    /// The representative `width()`/`style()` (used by the uniform fast path and any single-value
    /// consumer) are taken from the first visible side.
    pub fn new_per_side(widths: [f32; 4], styles: [BorderStyle; 4], brushes: [Brush; 4]) -> Self {
        // Pick a representative width/style from the first side that actually paints.
        let rep = (0..4).find(|&i| widths[i] > 0.0 && !styles[i].is_invisible());
        let (width, style) = match rep {
            Some(i) => (widths[i], styles[i].clone()),
            None => (0.0, BorderStyle::None),
        };
        Border {
            width,
            style,
            widths,
            styles,
            brushes,
            radius: None,
        }
    }

    /// True when all four sides share the same width and style, so the whole-rectangle stroke
    /// path can be used. (Per-side colours are still allowed but only the first brush is used by
    /// that fast path, matching prior behaviour.)
    pub fn is_uniform(&self) -> bool {
        self.widths.iter().all(|&w| w == self.widths[0]) && self.styles.iter().all(|s| *s == self.styles[0])
    }

    /// Per-side widths `[top, right, bottom, left]`.
    pub fn widths(&self) -> [f32; 4] {
        self.widths
    }

    /// Per-side styles `[top, right, bottom, left]`.
    pub fn styles(&self) -> [BorderStyle; 4] {
        self.styles.clone()
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

    pub fn brush(&self) -> Brush {
        self.brushes[0].clone()
    }

    pub fn radius(&self) -> Option<Trbl<BorderRadius>> {
        self.radius.clone()
    }
}
