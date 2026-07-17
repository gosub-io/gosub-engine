use crate::painter::commands::color::Color;

#[derive(Clone, Debug)]
pub struct ColorStop {
    /// Position along the gradient line, `0.0` (start) .. `1.0` (end).
    pub offset: f32,
    pub color: Color,
}

/// Gradient as a repeated `background-image` layer: paints one `tile_size` cell and repeats it.
/// Absent means the gradient fills the whole box (the plain `linear-gradient(...)` case).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tiling {
    /// One tile's size in device pixels (resolved `background-size`).
    pub tile_size: (f32, f32),
    /// Offset of the first tile's origin from the box origin, in device pixels
    /// (resolved `background-position`). May be negative.
    pub position: (f32, f32),
    /// Whether the tile repeats along the x / y axis (`background-repeat`).
    pub repeat: (bool, bool),
}

#[derive(Clone, Debug)]
pub struct LinearGradient {
    /// CSS degrees: `0` = to top, `90` = to right, `180` = to bottom, increasing clockwise.
    pub angle_deg: f32,
    /// Source order, each with a resolved `offset` in `0.0..=1.0`.
    pub stops: Vec<ColorStop>,
    /// Tiling for a repeated `background-image` layer, or `None` to fill the whole box.
    pub tiling: Option<Tiling>,
}

impl LinearGradient {
    /// Gradient line within a `w`×`h` box, relative to the box origin. Per spec the line is
    /// centred and long enough that the `0%`/`100%` stops land on the box's edges/corners.
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

    /// Interpolated colour at `t` (0.0 = line start, 1.0 = line end). Stops must be sorted by
    /// non-decreasing offset; two stops sharing one offset yield a hard edge.
    pub fn color_at(&self, t: f32) -> Color {
        match self.stops.as_slice() {
            [] => Color::TRANSPARENT,
            [only] => only.color.clone(),
            stops => {
                if t <= stops[0].offset {
                    return stops[0].color.clone();
                }
                let last = &stops[stops.len() - 1];
                if t >= last.offset {
                    return last.color.clone();
                }
                for pair in stops.windows(2) {
                    let (a, b) = (&pair[0], &pair[1]);
                    if t >= a.offset && t <= b.offset {
                        let span = b.offset - a.offset;
                        if span <= f32::EPSILON {
                            // Hard stop: pick the colour on the far side of the edge.
                            return b.color.clone();
                        }
                        let f = (t - a.offset) / span;
                        return Color::from_rgba(
                            a.color.r() + (b.color.r() - a.color.r()) * f,
                            a.color.g() + (b.color.g() - a.color.g()) * f,
                            a.color.b() + (b.color.b() - a.color.b()) * f,
                            a.color.a() + (b.color.a() - a.color.a()) * f,
                        );
                    }
                }
                last.color.clone()
            }
        }
    }

    /// Rasterize one `tw`×`th` tile into straight-alpha RGBA8 (row-major, 4 bytes per pixel),
    /// to be repeated across a tiled `background-image` layer.
    pub fn rasterize_tile(&self, tw: u32, th: u32) -> Vec<u8> {
        let (w, h) = (tw as f32, th as f32);
        let ((x0, y0), (x1, y1)) = self.line(w, h);
        let (dx, dy) = (x1 - x0, y1 - y0);
        let len2 = dx * dx + dy * dy;
        let mut out = vec![0u8; (tw as usize) * (th as usize) * 4];
        for py in 0..th {
            for px in 0..tw {
                // Sample at the pixel centre and project onto the gradient line.
                let (sx, sy) = (px as f32 + 0.5, py as f32 + 0.5);
                let t = if len2 <= 0.0 {
                    0.0
                } else {
                    (((sx - x0) * dx + (sy - y0) * dy) / len2).clamp(0.0, 1.0)
                };
                let c = self.color_at(t);
                let i = ((py * tw + px) * 4) as usize;
                out[i] = c.r8();
                out[i + 1] = c.g8();
                out[i + 2] = c.b8();
                out[i + 3] = c.a8();
            }
        }
        out
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
            tiling: None,
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
