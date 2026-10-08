use crate::common::geo::Dimension;
use crate::common::media::decoder::fit_to_kept_pixels;
use crate::render::backend::PixelFormat;
use parking_lot::RwLock;
use resvg::usvg;
use std::sync::Arc;

/// Cached render of an SVG at a specific dimension and byte order.
pub struct RenderedSvg {
    pub dimension: Dimension,
    pub data: Vec<u8>,
    pub format: PixelFormat,
}

impl RenderedSvg {
    /// Whether this entry can be reused for `dimension` in `format`. Backends share one cache but
    /// store different byte orders (Cairo/Skia BGRA, Vello RGBA), so a dimension-only match hands
    /// back red/blue-swapped pixels.
    pub fn is_usable(&self, dimension: Dimension, format: PixelFormat) -> bool {
        !self.data.is_empty() && self.dimension == dimension && self.format == format
    }

    /// Replace the cached pixels, recording the byte order they were rendered in.
    pub fn store(&mut self, dimension: Dimension, format: PixelFormat, data: Vec<u8>) {
        self.dimension = dimension;
        self.format = format;
        self.data = data;
    }
}

/// Pixel size to rasterize an SVG at when it is drawn into a `css`-sized box at `dpr`.
///
/// The box size comes from the page, so `width: 60000px` on an inline `<svg>` would otherwise
/// ask for a 14 GB pixmap. Like a decoded photograph, the raster is held to
/// [`MAX_KEPT_PIXELS`](super::MAX_KEPT_PIXELS) and scaled up to the box when painted:
/// an oversized SVG still shows, just less crisp. Within the budget this is the box size in
/// physical pixels, whole CSS pixels times `dpr`, as the backends have always rendered it.
pub fn svg_raster_size(css: Dimension, dpr: u32) -> (u32, u32) {
    let dpr = f64::from(dpr.max(1));
    fit_to_kept_pixels(css.width.floor() * dpr, css.height.floor() * dpr)
}

#[derive(Clone)]
pub struct Svg {
    pub tree: usvg::Tree,
    /// Rendered cache - dimension and pixel data kept under one lock for consistency.
    pub rendered: Arc<RwLock<RenderedSvg>>,
}

impl Svg {
    pub fn new(tree: usvg::Tree) -> Svg {
        Svg {
            tree,
            rendered: Arc::new(RwLock::new(RenderedSvg {
                dimension: Dimension::ZERO,
                data: vec![],
                // Arbitrary until the first render; `is_usable` rejects the empty buffer first.
                format: PixelFormat::Rgba8,
            })),
        }
    }
}

impl std::fmt::Debug for Svg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Svg").field("tree", &self.tree).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::media::MAX_KEPT_PIXELS;

    fn entry(format: PixelFormat) -> RenderedSvg {
        RenderedSvg {
            dimension: Dimension::new(16.0, 16.0),
            data: vec![1, 2, 3, 4],
            format,
        }
    }

    /// Cairo/Skia (BGRA) and Vello (RGBA) share this cache, and at dpr 1 they key on the same
    /// dimension - so format must be part of the match or one backend paints the other's bytes
    /// with red and blue swapped.
    #[test]
    fn entry_is_not_reused_across_pixel_formats() {
        let bgra = entry(PixelFormat::PreMulArgb32);
        let dim = Dimension::new(16.0, 16.0);

        assert!(bgra.is_usable(dim, PixelFormat::PreMulArgb32));
        assert!(!bgra.is_usable(dim, PixelFormat::Rgba8));
    }

    #[test]
    fn entry_is_not_reused_across_dimensions() {
        let e = entry(PixelFormat::Rgba8);
        assert!(!e.is_usable(Dimension::new(32.0, 16.0), PixelFormat::Rgba8));
    }

    #[test]
    fn empty_entry_is_never_usable() {
        let fresh = RenderedSvg {
            dimension: Dimension::ZERO,
            data: vec![],
            format: PixelFormat::Rgba8,
        };
        assert!(!fresh.is_usable(Dimension::ZERO, PixelFormat::Rgba8));
    }

    #[test]
    fn raster_size_is_the_physical_box_size_within_the_budget() {
        assert_eq!(svg_raster_size(Dimension::new(100.0, 50.0), 1), (100, 50));
        assert_eq!(svg_raster_size(Dimension::new(100.0, 50.0), 2), (200, 100));
        // Whole CSS pixels, then dpr, as the backends rendered before the cap.
        assert_eq!(svg_raster_size(Dimension::new(10.5, 4.9), 2), (20, 8));
        assert_eq!(svg_raster_size(Dimension::new(0.5, 0.5), 2), (1, 1));
    }

    #[test]
    fn oversized_raster_is_held_to_the_budget_keeping_its_aspect_ratio() {
        let (w, h) = svg_raster_size(Dimension::new(60000.0, 60000.0), 1);
        assert!(u64::from(w) * u64::from(h) <= MAX_KEPT_PIXELS, "{w}x{h}");
        assert_eq!(w, h);
        assert!(w >= 2000, "the cap should not shrink it further than needed: {w}");

        let (w, h) = svg_raster_size(Dimension::new(8000.0, 2000.0), 2);
        assert!(u64::from(w) * u64::from(h) <= MAX_KEPT_PIXELS, "{w}x{h}");
        let ratio = f64::from(w) / f64::from(h);
        assert!((ratio - 4.0).abs() < 0.01, "ratio {ratio}");
    }

    #[test]
    fn hostile_raster_sizes_stay_in_range() {
        for (w, h, dpr) in [
            (f64::MAX, f64::MAX, 2),
            (f64::INFINITY, 10.0, 1),
            (f64::NAN, f64::NAN, 1),
            (-5.0, 0.0, 1),
            (1.0, 1e12, 1),
            (u32::MAX as f64, 1.0, u32::MAX),
        ] {
            let (rw, rh) = svg_raster_size(Dimension::new(w, h), dpr);
            assert!(rw >= 1 && rh >= 1, "{w}x{h}@{dpr} gave {rw}x{rh}");
            assert!(
                u64::from(rw) * u64::from(rh) <= MAX_KEPT_PIXELS,
                "{w}x{h}@{dpr} gave {rw}x{rh}"
            );
        }
        assert_eq!(svg_raster_size(Dimension::new(f64::NAN, -1.0), 1), (1, 1));
    }

    #[test]
    fn store_records_the_format_it_rendered_in() {
        let mut e = entry(PixelFormat::Rgba8);
        e.store(Dimension::new(8.0, 8.0), PixelFormat::PreMulArgb32, vec![9; 4]);

        assert!(e.is_usable(Dimension::new(8.0, 8.0), PixelFormat::PreMulArgb32));
        assert!(!e.is_usable(Dimension::new(8.0, 8.0), PixelFormat::Rgba8));
    }
}
