//! `box-shadow` geometry (css-backgrounds-3 §7.1), worked out once here so every backend paints
//! the same shapes and only has to know how to blur one.

use crate::common::geo::Rect;
use crate::painter::commands::color::Color;
use crate::painter::commands::rectangle::Radius;

/// A box with its corner radii in CSS order: top-left, top-right, bottom-right, bottom-left.
#[derive(Clone, Copy, Debug)]
pub struct RoundedRect {
    pub rect: Rect,
    pub radii: [Radius; 4],
}

impl RoundedRect {
    pub fn new(rect: Rect, radii: [Radius; 4]) -> Self {
        RoundedRect { rect, radii }
    }

    pub fn is_rounded(&self) -> bool {
        self.radii.iter().any(|radius| radius.x > 0.0 || radius.y > 0.0)
    }

    fn translate(self, dx: f64, dy: f64) -> Self {
        let rect = Rect::new(self.rect.x + dx, self.rect.y + dy, self.rect.width, self.rect.height);
        RoundedRect { rect, ..self }
    }

    /// Grown by `distance` on every side, or shrunk when it is negative, down to nothing.
    ///
    /// A radius grows by the distance too, but one smaller than the distance grows by less -
    /// by the distance times `1 + (r - 1)³`, `r` being the radius over the distance - so a
    /// square corner stays square and a slightly rounded one does not suddenly turn round
    /// (css-backgrounds-3 §7.1.1). A shrinking radius just shrinks.
    fn spread(self, distance: f64) -> Self {
        let rect = Rect::new(
            self.rect.x - distance,
            self.rect.y - distance,
            (self.rect.width + distance * 2.0).max(0.0),
            (self.rect.height + distance * 2.0).max(0.0),
        );
        let grow = |radius: f64| {
            if distance >= 0.0 && radius < distance {
                let ratio = radius / distance;
                radius + distance * (1.0 + (ratio - 1.0).powi(3))
            } else {
                (radius + distance).max(0.0)
            }
        };
        let radii = self
            .radii
            .map(|radius| Radius::new_double(grow(radius.x), grow(radius.y)));
        RoundedRect { rect, radii }
    }
}

/// One shadow, ready to paint: fill `shape` with `color`, blur it, and keep only what falls
/// outside `clip` for an outer shadow, inside it for an inset one.
///
/// An inset shadow is the other way round from an outer one: what is shadowed is the padding
/// box *around* `shape`, the hole the shadow leaves.
#[derive(Clone, Debug)]
pub struct Shadow {
    pub inset: bool,
    pub shape: RoundedRect,
    /// The border box for an outer shadow, the padding box for an inset one.
    pub clip: RoundedRect,
    /// The Gaussian's standard deviation: half the blur radius.
    pub sigma: f64,
    pub color: Color,
}

impl Shadow {
    /// An outer shadow of a box: the border box moved by the offset and grown by the spread.
    /// `None` when a negative spread leaves nothing to cast it.
    pub fn outer(border_box: RoundedRect, offset: (f64, f64), blur: f64, spread: f64, color: Color) -> Option<Self> {
        let shape = border_box.translate(offset.0, offset.1).spread(spread);
        if shape.rect.width <= 0.0 || shape.rect.height <= 0.0 {
            return None;
        }
        Some(Shadow {
            inset: false,
            shape,
            clip: border_box,
            sigma: blur / 2.0,
            color,
        })
    }

    /// An inset shadow: the padding box moved by the offset and shrunk by the spread is the
    /// hole, and the rest of the padding box is in shadow. A hole that shrinks away leaves all
    /// of it in shadow.
    pub fn inset(padding_box: RoundedRect, offset: (f64, f64), blur: f64, spread: f64, color: Color) -> Self {
        Shadow {
            inset: true,
            shape: padding_box.translate(offset.0, offset.1).spread(-spread),
            clip: padding_box,
            sigma: blur / 2.0,
            color,
        }
    }

    /// Everything this shadow can paint.
    pub fn ink_rect(&self) -> Rect {
        if self.inset {
            return self.clip.rect;
        }
        let reach = blur_reach(self.sigma);
        let r = self.shape.rect;
        Rect::new(r.x - reach, r.y - reach, r.width + reach * 2.0, r.height + reach * 2.0)
    }
}

/// How far a blurred edge reaches past the shape: three standard deviations, past which the
/// Gaussian adds less than half a percent.
pub fn blur_reach(sigma: f64) -> f64 {
    (sigma * 3.0).ceil()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(size: f64, radius: f64) -> RoundedRect {
        RoundedRect::new(Rect::new(0.0, 0.0, size, size), [Radius::new(radius); 4])
    }

    #[test]
    fn spread_grows_the_box_and_its_corners() {
        let shadow = Shadow::outer(square(10.0, 0.0), (2.0, 3.0), 0.0, 4.0, Color::BLACK).expect("a shadow");
        let r = shadow.shape.rect;
        assert_eq!((r.x, r.y, r.width, r.height), (-2.0, -1.0, 18.0, 18.0));
        assert!(!shadow.shape.is_rounded(), "a square corner stays square");

        let shadow = Shadow::outer(square(40.0, 8.0), (0.0, 0.0), 0.0, 4.0, Color::BLACK).expect("a shadow");
        assert_eq!(
            shadow.shape.radii[0].x, 12.0,
            "a radius past the spread grows by all of it"
        );

        // Below the spread it grows by less: 2 + 4 · (1 + (0.5 - 1)³) = 5.5.
        let shadow = Shadow::outer(square(40.0, 2.0), (0.0, 0.0), 0.0, 4.0, Color::BLACK).expect("a shadow");
        assert_eq!(shadow.shape.radii[0].x, 5.5);
    }

    #[test]
    fn a_negative_spread_can_leave_no_outer_shadow() {
        assert!(Shadow::outer(square(10.0, 0.0), (0.0, 0.0), 4.0, -5.0, Color::BLACK).is_none());

        // An inset shadow's hole shrinks away instead, and the whole box is in shadow.
        let shadow = Shadow::inset(square(10.0, 0.0), (0.0, 0.0), 0.0, 6.0, Color::BLACK);
        assert_eq!(shadow.shape.rect.width, 0.0);
        assert_eq!(shadow.ink_rect().width, 10.0);
    }

    #[test]
    fn an_outer_shadow_reaches_three_deviations_past_its_shape() {
        let shadow = Shadow::outer(square(10.0, 0.0), (0.0, 5.0), 8.0, 0.0, Color::BLACK).expect("a shadow");
        let ink = shadow.ink_rect();
        assert_eq!((ink.x, ink.y, ink.width, ink.height), (-12.0, -7.0, 34.0, 34.0));
    }
}
