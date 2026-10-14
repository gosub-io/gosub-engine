//! `box-shadow` for Cairo, which has no blur of its own: the shadow's shape is drawn into an
//! alpha mask, blurred here, and the colour painted through it.
//!
//! Only the part of the shadow on this tile is worked out, plus the margin the blur reads from:
//! a big card's shadow costs each tile it touches no more than that tile's share.

use cairo::{Context, FillRule, Format, ImageSurface};
use gosub_render_pipeline::common::geo::Rect;
use gosub_render_pipeline::painter::commands::shadow::{blur_reach, RoundedRect, Shadow};
use gosub_render_pipeline::tiler::Tile;
use std::f64::consts::PI;

/// Paints the outer or the inset ones of `shadows`, last to first so the first listed ends up on
/// top. `cr` is the tile's context, in page CSS pixels scaled by `dpr`.
pub(crate) fn paint_shadows(cr: &Context, tile: &Tile, shadows: &[Shadow], inset: bool, dpr: f64) {
    for shadow in shadows.iter().rev().filter(|shadow| shadow.inset == inset) {
        let Some(on_tile) = intersect(shadow.ink_rect(), tile.rect) else {
            continue;
        };
        let reach = if shadow.sigma > 0.0 {
            blur_reach(shadow.sigma) + 1.0
        } else {
            0.0
        };
        // Whole CSS pixels, so the mask lands on the device grid.
        let x = (on_tile.x - reach).floor();
        let y = (on_tile.y - reach).floor();
        let area = Rect::new(
            x,
            y,
            (on_tile.x + on_tile.width + reach).ceil() - x,
            (on_tile.y + on_tile.height + reach).ceil() - y,
        );
        let Some(mask) = shadow_mask(shadow, area, dpr) else {
            continue;
        };

        _ = cr.save();
        cr.new_path();
        if inset {
            rounded_path(cr, &shadow.clip);
        } else {
            cr.set_fill_rule(FillRule::EvenOdd);
            cr.rectangle(area.x - 1.0, area.y - 1.0, area.width + 2.0, area.height + 2.0);
            rounded_path(cr, &shadow.clip);
        }
        cr.clip();
        let c = &shadow.color;
        cr.set_source_rgba(f64::from(c.r()), f64::from(c.g()), f64::from(c.b()), f64::from(c.a()));
        _ = cr.mask_surface(&mask, area.x, area.y);
        _ = cr.restore();
        cr.new_path();
    }
}

/// The shadow's coverage over `area` (CSS pixels), blurred, at `dpr` device pixels per CSS pixel.
fn shadow_mask(shadow: &Shadow, area: Rect, dpr: f64) -> Option<ImageSurface> {
    let width = (area.width * dpr).ceil() as i32;
    let height = (area.height * dpr).ceil() as i32;
    let mut mask = ImageSurface::create(Format::A8, width, height).ok()?;
    {
        let m = Context::new(&mask).ok()?;
        m.scale(dpr, dpr);
        m.translate(-area.x, -area.y);
        if shadow.inset {
            // What is in shadow is everything around the hole. The area reaches past the padding
            // box by the blur's reach, so its own edge never blurs into what is painted.
            m.set_fill_rule(FillRule::EvenOdd);
            m.rectangle(area.x, area.y, area.width, area.height);
        }
        rounded_path(&m, &shadow.shape);
        m.fill().ok()?;
    }
    mask.flush();
    if shadow.sigma > 0.0 {
        let stride = mask.stride() as usize;
        let mut data = mask.data().ok()?;
        blur_alpha(&mut data, width as usize, height as usize, stride, shadow.sigma * dpr);
    }
    mask.set_device_scale(dpr, dpr);
    Some(mask)
}

fn intersect(a: Rect, b: Rect) -> Option<Rect> {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    let x2 = (a.x + a.width).min(b.x + b.width);
    let y2 = (a.y + a.height).min(b.y + b.height);
    (x2 > x && y2 > y).then(|| Rect::new(x, y, x2 - x, y2 - y))
}

/// Corners by their horizontal radius, as the rest of this backend draws them.
fn rounded_path(cr: &Context, rounded: &RoundedRect) {
    let r = rounded.rect;
    let [tl, tr, br, bl] = rounded.radii.map(|radius| radius.x);
    if !rounded.is_rounded() {
        cr.rectangle(r.x, r.y, r.width, r.height);
        return;
    }
    let (x, y, x2, y2) = (r.x, r.y, r.x + r.width, r.y + r.height);
    cr.new_sub_path();
    cr.arc(x2 - tr, y + tr, tr, -0.5 * PI, 0.0);
    cr.arc(x2 - br, y2 - br, br, 0.0, 0.5 * PI);
    cr.arc(x + bl, y2 - bl, bl, 0.5 * PI, PI);
    cr.arc(x + tl, y + tl, tl, PI, 1.5 * PI);
    cr.close_path();
}

/// A Gaussian blur of an 8-bit alpha plane, as three box blurs in each direction - within a few
/// percent of the real thing, and linear in the image whatever the radius.
fn blur_alpha(data: &mut [u8], width: usize, height: usize, stride: usize, sigma: f64) {
    let mut line = Vec::new();
    for radius in box_radii(sigma) {
        for y in 0..height {
            line.clear();
            line.extend((0..width).map(|x| data[y * stride + x]));
            box_blur_line(&line, radius, |x, value| data[y * stride + x] = value);
        }
        for x in 0..width {
            line.clear();
            line.extend((0..height).map(|y| data[y * stride + x]));
            box_blur_line(&line, radius, |y, value| data[y * stride + x] = value);
        }
    }
}

/// Each output is the mean of the `2 * radius + 1` inputs around it, nothing past the ends.
fn box_blur_line(line: &[u8], radius: usize, mut write: impl FnMut(usize, u8)) {
    let window = (2 * radius + 1) as u32;
    let at = |i: isize| -> u32 {
        if i < 0 {
            0
        } else {
            line.get(i as usize).copied().map_or(0, u32::from)
        }
    };
    let mut sum: u32 = (0..=radius as isize).map(at).sum();
    for i in 0..line.len() {
        write(i, ((sum + window / 2) / window) as u8);
        let i = i as isize;
        sum = sum + at(i + radius as isize + 1) - at(i - radius as isize);
    }
}

/// The three box radii whose successive blurs come closest to a Gaussian of `sigma`
/// (W. Jarosz, "Fast image convolutions").
fn box_radii(sigma: f64) -> [usize; 3] {
    let n = 3.0;
    let ideal = (12.0 * sigma * sigma / n + 1.0).sqrt();
    let mut lower = ideal.floor() as i64;
    if lower % 2 == 0 {
        lower -= 1;
    }
    let lower = lower.max(1);
    let upper = lower + 2;
    let l = lower as f64;
    let wanted = ((12.0 * sigma * sigma - n * l * l - 4.0 * n * l - 3.0 * n) / (-4.0 * l - 4.0)).round();
    std::array::from_fn(|i| {
        let size = if (i as f64) < wanted { lower } else { upper };
        ((size - 1) / 2) as usize
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Three box passes spread a single bright pixel like a Gaussian: its variance is the sum
    /// of the boxes', which is sigma squared.
    #[test]
    fn box_radii_add_up_to_the_gaussian() {
        for sigma in [1.0, 2.5, 6.0, 20.0] {
            let variance: f64 = box_radii(sigma)
                .iter()
                .map(|&radius| {
                    let size = (2 * radius + 1) as f64;
                    (size * size - 1.0) / 12.0
                })
                .sum();
            assert!(
                (variance.sqrt() - sigma).abs() < 0.6,
                "sigma {sigma}: {}",
                variance.sqrt()
            );
        }
    }

    /// A hard edge blurs to half coverage on the edge itself, and to nothing and full well away
    /// from it.
    #[test]
    fn an_edge_blurs_symmetrically() {
        let (width, height) = (64, 64);
        let mut data: Vec<u8> = (0..width * height)
            .map(|i| if i % width < 32 { 255 } else { 0 })
            .collect();
        blur_alpha(&mut data, width, height, width, 4.0);
        let row = &data[32 * width..33 * width];
        assert!(row[12..20].iter().all(|&a| a > 200), "{row:?}");
        assert!(row[44..].iter().all(|&a| a < 10), "{row:?}");
        assert!((i32::from(row[31]) + i32::from(row[32]) - 255).abs() < 12, "{row:?}");
    }
}
