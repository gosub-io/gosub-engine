use crate::common::media::{MAX_IMAGE_EDGE, MAX_KEPT_PIXELS};
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
    /// Gradient line within a `w`x`h` box, relative to the box origin. Per spec the line is
    /// centred and long enough that the `0%`/`100%` stops land on the box's edges/corners.
    pub fn line(&self, w: f32, h: f32) -> ((f32, f32), (f32, f32)) {
        let theta = self.angle_deg.to_radians();
        // CSS direction vector: 0deg -> up (0,-1), 90deg -> right (1,0), 180deg -> down (0,1).
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

    /// Rasterize one `tiling` tile into straight-alpha RGBA8 (row-major, 4 bytes per pixel),
    /// to be repeated across a tiled `background-image` layer. The raster is bounded by
    /// [`clamp_tile_size`]; the returned `scale` stretches it back to the tile's real size.
    pub fn rasterize_tile(&self, tiling: &Tiling) -> GradientTile {
        let (tw, th) = clamp_tile_size(tiling.tile_size);
        let (w, h) = (tw as f32, th as f32);
        let ((x0, y0), (x1, y1)) = self.line(w, h);
        let (dx, dy) = (x1 - x0, y1 - y0);
        let len2 = dx * dx + dy * dy;
        let (tw_us, th_us) = (tw as usize, th as usize);
        let mut out = vec![0u8; tw_us * th_us * 4];
        for py in 0..th_us {
            for px in 0..tw_us {
                // Sample at the pixel centre and project onto the gradient line.
                let (sx, sy) = (px as f32 + 0.5, py as f32 + 0.5);
                let t = if len2 <= 0.0 {
                    0.0
                } else {
                    (((sx - x0) * dx + (sy - y0) * dy) / len2).clamp(0.0, 1.0)
                };
                let c = self.color_at(t);
                let i = (py * tw_us + px) * 4;
                out[i] = c.r8();
                out[i + 1] = c.g8();
                out[i + 2] = c.b8();
                out[i + 3] = c.a8();
            }
        }
        GradientTile {
            rgba: out,
            width: tw,
            height: th,
            scale: (tile_scale(tiling.tile_size.0, tw), tile_scale(tiling.tile_size.1, th)),
        }
    }
}

/// One rasterized gradient tile, ready for a backend to wrap in a repeating pattern.
#[derive(Clone, Debug)]
pub struct GradientTile {
    /// Straight-alpha RGBA8, row-major, `width * height * 4` bytes.
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Device pixels per raster pixel along x / y, to apply in the pattern transform. `1.0`
    /// for every tile within [`clamp_tile_size`]'s bounds.
    pub scale: (f64, f64),
}

/// Raster size for one `tile_size` (device pixels) gradient tile: rounded to whole pixels, at
/// least 1x1, each edge capped at [`MAX_IMAGE_EDGE`] and the whole at [`MAX_KEPT_PIXELS`] - the
/// bounds a decoded image gets. `background-size` is page-controlled, so an absurd tile is
/// rasterized smaller and scaled up by the backend rather than allocated in full. Scaling, not
/// clipping to the painted area, keeps `background-position`/`-repeat` geometry intact; a
/// gradient is smooth, so only hard stops soften, and only on tiles far past any viewport.
pub fn clamp_tile_size(tile_size: (f32, f32)) -> (u32, u32) {
    // `as u32` saturates: NaN and negatives become 0 (then 1), infinity the edge cap.
    let edge = |v: f32| (v.round() as u32).clamp(1, MAX_IMAGE_EDGE);
    let (w, h) = (edge(tile_size.0), edge(tile_size.1));
    let pixels = u64::from(w) * u64::from(h);
    if pixels <= MAX_KEPT_PIXELS {
        return (w, h);
    }
    let scale = (MAX_KEPT_PIXELS as f64 / pixels as f64).sqrt();
    (
        ((f64::from(w) * scale) as u32).max(1),
        ((f64::from(h) * scale) as u32).max(1),
    )
}

/// Device pixels per raster pixel for a tile edge of `size` device pixels rasterized at `raster`
/// pixels: exactly `1.0` unless [`clamp_tile_size`] shrank it. A non-finite size has no real
/// extent to restore, so it is left unscaled.
fn tile_scale(size: f32, raster: u32) -> f64 {
    if size.is_finite() {
        f64::from(size.round().max(1.0)) / f64::from(raster)
    } else {
        1.0
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

    fn tiling(w: f32, h: f32) -> Tiling {
        Tiling {
            tile_size: (w, h),
            position: (0.0, 0.0),
            repeat: (true, true),
        }
    }

    #[test]
    fn clamp_tile_size_keeps_normal_tiles() {
        assert_eq!(clamp_tile_size((20.0, 30.0)), (20, 30));
        assert_eq!(clamp_tile_size((19.6, 0.4)), (20, 1));
        assert_eq!(clamp_tile_size((2048.0, 2048.0)), (2048, 2048));
    }

    #[test]
    fn clamp_tile_size_bounds_oversized_tiles() {
        // Edge cap to 16384 each (2^28 px), then sqrt(2^22 / 2^28) = 1/8 to fit 4 Mpx.
        assert_eq!(clamp_tile_size((100_000.0, 100_000.0)), (2048, 2048));
        assert_eq!(clamp_tile_size((2049.0, 2048.0)).0, 2048);
    }

    #[test]
    fn clamp_tile_size_caps_a_thin_tile_by_edge() {
        // 1 x 10M passes any pixel budget on its own; the edge cap is what bounds it.
        assert_eq!(clamp_tile_size((1.0, 10_000_000.0)), (1, MAX_IMAGE_EDGE));
    }

    #[test]
    fn clamp_tile_size_handles_degenerate_input() {
        assert_eq!(clamp_tile_size((0.0, -5.0)), (1, 1));
        assert_eq!(clamp_tile_size((f32::NAN, 10.0)), (1, 10));
        assert_eq!(clamp_tile_size((f32::INFINITY, f32::INFINITY)), (2048, 2048));
    }

    #[test]
    fn rasterize_tile_is_unscaled_within_bounds() {
        let tile = lg(90.0).rasterize_tile(&tiling(4.0, 3.0));
        assert_eq!((tile.width, tile.height), (4, 3));
        assert_eq!(tile.rgba.len(), 4 * 3 * 4);
        assert_eq!(tile.scale, (1.0, 1.0));
    }

    #[test]
    fn tile_scale_restores_a_clamped_edge() {
        let (w, h) = clamp_tile_size((40_000.0, 20.0));
        assert_eq!((w, h), (MAX_IMAGE_EDGE, 20));
        assert_eq!(tile_scale(40_000.0, w) * f64::from(w), 40_000.0);
        assert_eq!(tile_scale(20.0, h), 1.0);
        assert_eq!(tile_scale(f32::INFINITY, w), 1.0);
    }
}
