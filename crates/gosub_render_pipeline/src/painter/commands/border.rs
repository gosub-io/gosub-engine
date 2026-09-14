use crate::common::geo::Rect;
use crate::painter::commands::brush::Brush;
use crate::painter::commands::Trbl;

/// Axis-aligned strips that paint one side of a non-uniform border while keeping its
/// style: `Double` is two thin strips, `Dashed`/`Dotted` a run of segments along the edge,
/// everything else (incl. the 3D styles, which have no two-tone rendering yet) one solid
/// strip. Sides are `[top, right, bottom, left]`; a side with no visible border is empty.
/// Backends only need a rect fill to consume this, so all three stay in sync.
pub fn per_side_strips(rect: Rect, widths: [f32; 4], styles: &[BorderStyle; 4]) -> [Vec<Rect>; 4] {
    let w = |i: usize| widths[i] as f64;
    let edges = [
        (rect.x, rect.y, rect.width, w(0), true),
        (rect.x + rect.width - w(1), rect.y, w(1), rect.height, false),
        (rect.x, rect.y + rect.height - w(2), rect.width, w(2), true),
        (rect.x, rect.y, w(3), rect.height, false),
    ];
    let mut out: [Vec<Rect>; 4] = Default::default();
    for (i, &(x, y, width, height, horizontal)) in edges.iter().enumerate() {
        let thickness = if horizontal { height } else { width };
        if thickness <= 0.0 || styles[i].is_invisible() {
            continue;
        }
        let strips = &mut out[i];
        match styles[i] {
            BorderStyle::Double if thickness >= 3.0 => {
                let strand = (thickness / 3.0).floor();
                let inner = thickness - strand;
                if horizontal {
                    strips.push(Rect::new(x, y, width, strand));
                    strips.push(Rect::new(x, y + inner, width, strand));
                } else {
                    strips.push(Rect::new(x, y, strand, height));
                    strips.push(Rect::new(x + inner, y, strand, height));
                }
            }
            BorderStyle::Dashed | BorderStyle::Dotted => {
                // Same dash rhythm as the uniform stroke paths: dots are `w` on / `w` off,
                // dashes `3w` (min 3px) on / off.
                let seg = if styles[i] == BorderStyle::Dotted {
                    thickness
                } else {
                    (thickness * 3.0).max(3.0)
                };
                let length = if horizontal { width } else { height };
                let mut pos = 0.0;
                while pos < length {
                    let len = seg.min(length - pos);
                    strips.push(if horizontal {
                        Rect::new(x + pos, y, len, thickness)
                    } else {
                        Rect::new(x, y + pos, thickness, len)
                    });
                    pos += 2.0 * seg;
                }
            }
            _ => strips.push(Rect::new(x, y, width, height)),
        }
    }
    out
}

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

    /// Independent per-side widths and styles. The single-value `width()`/`style()` fall back to
    /// the first side that actually paints, for the uniform fast path and other scalar consumers.
    pub fn new_per_side(widths: [f32; 4], styles: [BorderStyle; 4], brushes: [Brush; 4]) -> Self {
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

    /// All four sides share width and style, so the whole-rectangle stroke path applies. Per-side
    /// colours are still allowed, but that fast path only uses the first brush.
    pub fn is_uniform(&self) -> bool {
        self.widths.iter().all(|&w| w == self.widths[0])
            && self.styles.iter().all(|s| *s == self.styles[0])
            // Same-width same-style borders can still differ per side in COLOR (collapsed
            // table boundaries owned by different neighbours) - the single-stroke path
            // would paint all four sides with one brush. Non-solid brushes conservatively
            // count as differing.
            && self.brushes.iter().all(|b| match (b, &self.brushes[0]) {
                (crate::painter::commands::brush::Brush::Solid(a), crate::painter::commands::brush::Brush::Solid(c)) => {
                    a.r() == c.r() && a.g() == c.g() && a.b() == c.b() && a.a() == c.a()
                }
                _ => false,
            })
    }

    pub fn widths(&self) -> [f32; 4] {
        self.widths
    }

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
