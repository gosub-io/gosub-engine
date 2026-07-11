//! CPU host-side tile compositing.
//!
//! Tile-rasterizing backends (Cairo, Skia, Vello-CPU) hand the host an
//! [`ExternalHandle::TileCache`](crate::render::backend::ExternalHandle::TileCache): a set of
//! pre-rasterized, premultiplied tiles in page coordinates. To present a frame the host must place
//! each visible tile at its scroll/anchor-resolved position and source-over blend it onto a
//! background. Every windowing example used to reimplement that identical loop; this module is the
//! one shared copy.
//!
//! The compositor works in the canonical premultiplied **ARGB** packing (`0xAARRGGBB`, see
//! [`PixelFormat::pixel_to_argb_u32`]). Callers fill a `u32` buffer with an opaque background,
//! composite into a [`TileTarget`] region of it, and then either present the `u32` buffer directly
//! (softbuffer ignores the high byte) or convert it to RGBA8 for a GPU texture via
//! [`argb_u32_to_rgba8`].

use crate::render::backend::{anchored_tile_pos, blend_over_argb_u32, scale_premul_argb_u32, CachedTile};

/// A rectangular region of a premultiplied-ARGB (`0xAARRGGBB`) `u32` buffer that tiles composite
/// into.
///
/// `buf` is row-major with `stride` pixels per row. Tile content is clipped to the `width × height`
/// device-pixel region whose top-left corner sits at (`origin_x`, `origin_y`) within `buf` — the
/// offset lets a host reserve rows for its own chrome (e.g. an address bar) while still handing the
/// whole window buffer. For an own-buffer host sized exactly to the content, use `origin_x = 0`,
/// `origin_y = 0`, `stride = width`.
///
/// The caller must fill `buf` with the desired **opaque** background (e.g. `0xFFFF_FFFF` white)
/// before compositing; tiles are blended on top with source-over.
pub struct TileTarget<'a> {
    pub buf: &'a mut [u32],
    pub stride: usize,
    pub origin_x: usize,
    pub origin_y: usize,
    pub width: usize,
    pub height: usize,
}

/// Source-over composite the visible `tiles` into `target`.
///
/// Each tile is placed by resolving its anchor against the page `scroll` (CSS px) via
/// [`anchored_tile_pos`] — which handles normal-flow, `fixed` and `sticky` uniformly — then scaling
/// to device pixels by `dpr`. Tiles are premultiplied; per-tile `opacity` fades the layer as a
/// whole before the blend. Tiles fully outside the target region are skipped; partially-visible
/// tiles are clipped.
///
/// This is the CPU counterpart to a GPU backend's own tile compositing, shared by the
/// tile-rasterizing windowing examples so no host reimplements it.
pub fn composite_tiles(tiles: &[CachedTile], dpr: u32, scroll: (f32, f32), target: &mut TileTarget<'_>) {
    let dpr_f = dpr as f64;
    let (scroll_x, scroll_y) = scroll;
    let clip_w = target.width as i64;
    let clip_h = target.height as i64;

    for tile in tiles {
        // Viewport position in CSS px from the engine's authoritative scroll, then device px.
        let (vx, vy) = anchored_tile_pos(
            tile.page_x as f64,
            tile.page_y as f64,
            scroll_x as f64,
            scroll_y as f64,
            tile.anchor,
        );
        let px = (vx * dpr_f).round() as i64;
        let py = (vy * dpr_f).round() as i64;
        let tw = tile.width as i64;
        let th = tile.height as i64;

        // Skip tiles wholly outside the content region.
        if px >= clip_w || py >= clip_h || px + tw <= 0 || py + th <= 0 {
            continue;
        }

        // Leading tile columns/rows that fall off the top/left edge.
        let col0 = (-px).max(0) as usize;
        let row0 = (-py).max(0) as usize;
        let dst_x = px.max(0) as usize;
        let dst_y0 = py.max(0) as usize;
        let tw = tw as usize;
        let th = th as usize;

        let src_u32 = bytemuck::cast_slice::<u8, u32>(&tile.data);

        for tile_row in row0..th {
            let dst_y = dst_y0 + (tile_row - row0);
            if dst_y >= target.height {
                break;
            }
            let copy_w = (tw - col0).min(target.width - dst_x);
            if copy_w == 0 {
                break;
            }
            let buf_row = (target.origin_y + dst_y) * target.stride + target.origin_x + dst_x;
            let src_row = tile_row * tw + col0;
            for col in 0..copy_w {
                let src_argb = tile.format.pixel_to_argb_u32(src_u32[src_row + col]);
                target.buf[buf_row + col] =
                    blend_over_argb_u32(scale_premul_argb_u32(src_argb, tile.opacity), target.buf[buf_row + col]);
            }
        }
    }
}

/// Convert a premultiplied-ARGB (`0xAARRGGBB`) `u32` buffer to RGBA8 bytes `[R, G, B, 255]`.
///
/// Alpha is forced opaque: the compositor blends onto an opaque background, so every output pixel
/// is opaque, and this is the layout wgpu/egui textures expect.
pub fn argb_u32_to_rgba8(buf: &[u32]) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(buf.len() * 4);
    for &px in buf {
        rgba.extend_from_slice(&[
            ((px >> 16) & 0xFF) as u8, // R
            ((px >> 8) & 0xFF) as u8,  // G
            (px & 0xFF) as u8,         // B
            255,                       // A (opaque)
        ]);
    }
    rgba
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::backend::{PixelFormat, TileAnchor};
    use bytes::Bytes;

    const WHITE: u32 = 0xFFFF_FFFF;

    /// A 1×1 opaque tile of the given RGBA bytes at page (page_x, page_y).
    fn tile_rgba(page_x: f32, page_y: f32, rgba: [u8; 4]) -> CachedTile {
        CachedTile {
            page_x,
            page_y,
            width: 1,
            height: 1,
            data: Bytes::copy_from_slice(&rgba),
            format: PixelFormat::Rgba8,
            opacity: 1.0,
            anchor: TileAnchor::Scroll,
            opaque: rgba[3] == 255,
        }
    }

    fn target_2x2(buf: &mut [u32]) -> TileTarget<'_> {
        TileTarget {
            buf,
            stride: 2,
            origin_x: 0,
            origin_y: 0,
            width: 2,
            height: 2,
        }
    }

    #[test]
    fn opaque_tile_overwrites_background() {
        let mut buf = [WHITE; 4];
        // Opaque red at (0,0).
        composite_tiles(&[tile_rgba(0.0, 0.0, [255, 0, 0, 255])], 1, (0.0, 0.0), &mut target_2x2(&mut buf));
        assert_eq!(buf[0], 0xFFFF_0000, "top-left becomes opaque red (ARGB)");
        assert_eq!(buf[1], WHITE, "other pixels untouched");
        assert_eq!(buf[3], WHITE);
    }

    #[test]
    fn scroll_moves_tiles_up() {
        // Tile at page y=1, scrolled down by 1 CSS px → lands at viewport (0,0).
        let mut buf = [WHITE; 4];
        composite_tiles(&[tile_rgba(0.0, 1.0, [0, 255, 0, 255])], 1, (0.0, 1.0), &mut target_2x2(&mut buf));
        assert_eq!(buf[0], 0xFF00_FF00, "scrolled tile lands top-left as green");
    }

    #[test]
    fn tiles_outside_region_are_culled() {
        let mut buf = [WHITE; 4];
        // Far off-screen; must not panic or write.
        composite_tiles(&[tile_rgba(1000.0, 1000.0, [255, 0, 0, 255])], 1, (0.0, 0.0), &mut target_2x2(&mut buf));
        assert!(buf.iter().all(|&p| p == WHITE));
    }

    #[test]
    fn origin_offset_reserves_top_rows() {
        // 2-wide, 2-tall buffer; content region offset down by 1 row (origin_y = 1), height 1.
        let mut buf = [WHITE; 4];
        let mut target = TileTarget {
            buf: &mut buf,
            stride: 2,
            origin_x: 0,
            origin_y: 1,
            width: 2,
            height: 1,
        };
        composite_tiles(&[tile_rgba(0.0, 0.0, [0, 0, 255, 255])], 1, (0.0, 0.0), &mut target);
        assert_eq!(buf[0], WHITE, "reserved top row untouched");
        assert_eq!(buf[2], 0xFF00_00FF, "tile lands in the offset region (row 1) as blue");
    }

    #[test]
    fn argb_to_rgba8_channel_order() {
        // 0xAARRGGBB red → [R,G,B,255].
        assert_eq!(argb_u32_to_rgba8(&[0xFF12_3456]), vec![0x12, 0x34, 0x56, 255]);
    }
}
