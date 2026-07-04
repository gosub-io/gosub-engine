use crate::painter::commands::color::Color;

/// A single colour stop along a gradient line.
#[derive(Clone, Debug)]
pub struct ColorStop {
    /// Position along the gradient line, `0.0` (start) .. `1.0` (end).
    pub offset: f32,
    pub color: Color,
}

/// A CSS `linear-gradient()`.
#[derive(Clone, Debug)]
pub struct LinearGradient {
    /// Gradient-line angle in CSS degrees: `0` = to top, `90` = to right, `180` = to
    /// bottom, increasing clockwise.
    pub angle_deg: f32,
    /// Colour stops in source order, each with a resolved `offset` in `0.0..=1.0`.
    pub stops: Vec<ColorStop>,
}

impl LinearGradient {
    /// Start and end points of the gradient line within a box of size `w`×`h`, following
    /// the CSS spec geometry (the line is centred on the box and long enough that the
    /// `0%`/`100%` stops sit on the box's edges/corners). Points are relative to the box
    /// origin `(0, 0)`.
    pub fn line(&self, w: f32, h: f32) -> ((f32, f32), (f32, f32)) {
        let theta = self.angle_deg.to_radians();
        // CSS direction vector: 0deg → up (0,-1), 90deg → right (1,0), 180deg → down (0,1).
        let dx = theta.sin();
        let dy = -theta.cos();
        // Half the length of the gradient line projected onto the box.
        let half = (w * dx.abs() + h * dy.abs()) / 2.0;
        let cx = w / 2.0;
        let cy = h / 2.0;
        ((cx - dx * half, cy - dy * half), (cx + dx * half, cy + dy * half))
    }
}

/// A CSS gradient. Only `linear-gradient()` is supported today; the enum leaves room for
/// radial/conic variants.
#[derive(Clone, Debug)]
pub enum Gradient {
    Linear(LinearGradient),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lg(angle_deg: f32) -> LinearGradient {
        LinearGradient {
            angle_deg,
            stops: Vec::new(),
        }
    }

    fn approx(a: (f32, f32), b: (f32, f32)) {
        assert!((a.0 - b.0).abs() < 0.01 && (a.1 - b.1).abs() < 0.01, "{a:?} != {b:?}");
    }

    #[test]
    fn to_bottom_runs_top_to_bottom() {
        // 180deg = `to bottom`: start at top-centre, end at bottom-centre.
        let (start, end) = lg(180.0).line(100.0, 200.0);
        approx(start, (50.0, 0.0));
        approx(end, (50.0, 200.0));
    }

    #[test]
    fn to_right_runs_left_to_right() {
        let (start, end) = lg(90.0).line(100.0, 200.0);
        approx(start, (0.0, 100.0));
        approx(end, (100.0, 100.0));
    }

    #[test]
    fn to_top_runs_bottom_to_top() {
        let (start, end) = lg(0.0).line(100.0, 200.0);
        approx(start, (50.0, 200.0));
        approx(end, (50.0, 0.0));
    }
}
