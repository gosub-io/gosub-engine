use crate::common::geo::Dimension;
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

    fn entry(format: PixelFormat) -> RenderedSvg {
        RenderedSvg {
            dimension: Dimension::new(16.0, 16.0),
            data: vec![1, 2, 3, 4],
            format,
        }
    }

    /// Cairo/Skia (BGRA) and Vello (RGBA) share this cache, and at dpr 1 they key on the same
    /// dimension — so format must be part of the match or one backend paints the other's bytes
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
    fn store_records_the_format_it_rendered_in() {
        let mut e = entry(PixelFormat::Rgba8);
        e.store(Dimension::new(8.0, 8.0), PixelFormat::PreMulArgb32, vec![9; 4]);

        assert!(e.is_usable(Dimension::new(8.0, 8.0), PixelFormat::PreMulArgb32));
        assert!(!e.is_usable(Dimension::new(8.0, 8.0), PixelFormat::Rgba8));
    }
}
