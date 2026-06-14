use crate::render::render_context::RenderContext;
use crate::render::viewport::Viewport;
use gosub_shared::tab_id::TabId;
use std::any::Any;
use std::ptr::NonNull;
use std::sync::Arc;

/// A surface rect has the same properties as a viewport, but computed with DevicePixelRatio.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct SurfaceRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Size of a rendering surface in pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceSize {
    pub width: u32,
    pub height: u32,
}

impl From<Viewport> for SurfaceSize {
    fn from(vp: Viewport) -> Self {
        Self {
            width: vp.width,
            height: vp.height,
        }
    }
}

impl From<Viewport> for SurfaceRect {
    fn from(vp: Viewport) -> Self {
        Self {
            x: vp.x,
            y: vp.y,
            width: vp.width,
            height: vp.height,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum PresentMode {
    Fifo,
    Immediate,
}

/// In-memory byte order of a rasterized tile / pixel buffer.
///
/// Both variants are premultiplied; they differ only in channel byte order, so
/// converting between them is a red/blue swap. A buffer is tagged with its format
/// at the point of production (the rasterizer) so consumers never have to assume an
/// order based on which backend feature happens to be compiled in. This matters
/// because Cargo feature unification (e.g. `cargo build --all`) can enable several
/// `backend_*` features at once, leaving a single rasterizer to win — its output
/// must be self-describing or colors silently swap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelFormat {
    /// Little-endian premultiplied ARGB32 — bytes are `[B, G, R, A]`. Produced by
    /// the Cairo and Skia rasterizers (Cairo `Format::ARgb32`, Skia n32).
    PreMulArgb32,
    /// Premultiplied RGBA8 — bytes are `[R, G, B, A]`. Produced by the Vello rasterizer.
    Rgba8,
}

impl PixelFormat {
    /// Returns `data` with bytes in `[R, G, B, A]` order, copying (and swapping R/B)
    /// only when the source is not already RGBA.
    pub fn to_rgba<'a>(self, data: &'a [u8]) -> std::borrow::Cow<'a, [u8]> {
        match self {
            PixelFormat::Rgba8 => std::borrow::Cow::Borrowed(data),
            PixelFormat::PreMulArgb32 => std::borrow::Cow::Owned(swap_rb(data)),
        }
    }

    /// Returns `data` with bytes in `[B, G, R, A]` order (little-endian ARGB32),
    /// copying (and swapping R/B) only when the source is not already in that order.
    pub fn to_argb32<'a>(self, data: &'a [u8]) -> std::borrow::Cow<'a, [u8]> {
        match self {
            PixelFormat::PreMulArgb32 => std::borrow::Cow::Borrowed(data),
            PixelFormat::Rgba8 => std::borrow::Cow::Owned(swap_rb(data)),
        }
    }
}

/// Swap the red and blue channels of a tightly-packed 4-bytes-per-pixel buffer.
fn swap_rb(data: &[u8]) -> Vec<u8> {
    let mut out = data.to_vec();
    for px in out.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
    out
}

impl PixelFormat {
    /// Reinterpret a little-endian 4-byte pixel (read as a `u32`) into the canonical
    /// `0xAARRGGBB` packing used for compositing, regardless of source channel order.
    ///
    /// On little-endian hosts, `PreMulArgb32` bytes `[B, G, R, A]` already read as
    /// `0xAARRGGBB`, while `Rgba8` bytes `[R, G, B, A]` read as `0xAABBGGRR` and need
    /// their red/blue channels swapped.
    #[inline]
    pub fn pixel_to_argb_u32(self, px: u32) -> u32 {
        match self {
            PixelFormat::PreMulArgb32 => px,
            PixelFormat::Rgba8 => {
                let a = px & 0xFF00_0000;
                let g = px & 0x0000_FF00;
                let r = px & 0x0000_00FF;
                let b = (px >> 16) & 0xFF;
                a | (r << 16) | g | b
            }
        }
    }
}

/// Alpha-blend a premultiplied source pixel over a premultiplied destination pixel,
/// both packed as `0xAARRGGBB`. Returns the premultiplied `0xAARRGGBB` result.
///
/// This is the "source-over" Porter-Duff operator: `out = src + dst * (1 - src_alpha)`.
/// Compositing tiles with this (rather than overwriting the destination) lets a
/// transparent upper-layer tile reveal the content of lower layers beneath it.
#[inline]
pub fn blend_over_argb_u32(src: u32, dst: u32) -> u32 {
    let sa = src >> 24;
    if sa == 0xFF {
        return src; // opaque source fully covers the destination
    }
    if sa == 0 {
        return dst; // fully transparent source contributes nothing
    }
    let inv = 255 - sa;
    // Blend the two pixels half a channel-pair at a time to keep it branch-free.
    // 0x00FF00FF mask isolates R and B; 0xFF00FF00 isolates A and G.
    let rb = (src & 0x00FF_00FF) + mul_div255_pair(dst & 0x00FF_00FF, inv);
    let ag = ((src >> 8) & 0x00FF_00FF) + mul_div255_pair((dst >> 8) & 0x00FF_00FF, inv);
    (rb & 0x00FF_00FF) | ((ag & 0x00FF_00FF) << 8)
}

/// Multiply each of the two 8-bit channels packed in `pair` (`0x00XX00YY`) by `factor`
/// (0..=255) and divide by 255 with rounding, returning the packed result.
#[inline]
fn mul_div255_pair(pair: u32, factor: u32) -> u32 {
    // Process both channels at once: add rounding bias, then the classic
    // `(x + (x >> 8) + 128) >> 8` approximation of `x / 255`.
    let t = pair * factor + 0x0080_0080;
    ((t + ((t >> 8) & 0x00FF_00FF)) >> 8) & 0x00FF_00FF
}

#[derive(Clone, Copy, Debug)]
pub enum GpuPixelFormat {
    Bgra8UnormSrgb,
    Rgba8UnormSrgb,
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub struct WgpuTextureId(pub u64);

/// A single pre-rasterized tile for direct compositing in the host draw callback.
/// Pixel data is reference-counted so handing out a handle is zero-copy.
#[derive(Clone, Debug)]
pub struct CachedTile {
    pub page_x: f32,
    pub page_y: f32,
    pub width: u32,
    pub height: u32,
    pub data: Arc<Vec<u8>>,
    /// In-memory byte order of `data`, set by the rasterizer that produced it.
    pub format: PixelFormat,
}

/// Safety: `ExternalHandle` can be sent between threads, but not shared.
#[allow(unsafe_code)]
unsafe impl Send for ExternalHandle {}
#[allow(unsafe_code)]
unsafe impl Sync for ExternalHandle {}

/// Handle that the host/browser can use to composite a surface.
#[derive(Clone, Debug)]
pub enum ExternalHandle {
    NullHandle {
        width: u32,
        height: u32,
        frame_id: u64,
    },

    CpuPixelsOwned {
        width: u32,
        height: u32,
        stride: u32,
        pixels: Vec<u8>,
        format: PixelFormat,
    },

    /// UNSAFE: caller must respect lifetime/size/stride.
    CpuPixelsPtr {
        width: u32,
        height: u32,
        stride: u32,
        pixel_buf: NonNull<u8>,
    },

    /// Pre-rasterized tile cache for zero-copy smooth scrolling.
    TileCache {
        viewport_width: u32,
        viewport_height: u32,
        dpr: u32,
        scroll_x: f32,
        scroll_y: f32,
        page_height: f32,
        tiles: Arc<Vec<CachedTile>>,
    },

    GlTexture {
        tex: u32,
        target: u32,
        width: u32,
        height: u32,
        frame_id: u64,
    },

    WgpuTextureId {
        id: u64,
        width: u32,
        height: u32,
        format: GpuPixelFormat,
        frame_id: u64,
    },

    SkiaImageId {
        id: u64,
        width: u32,
        height: u32,
        frame_id: u64,
    },

    /// Frame was rendered directly into an OpenGL framebuffer (e.g. GTK4 GLArea).
    /// No CPU pixels available — the GPU already wrote to the display framebuffer.
    GlFramebufferRendered {
        frame_id: u64,
    },
}

/// Small RGBA image, typically used for thumbnails or previews.
#[derive(Clone)]
pub struct RgbaImage {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub format: PixelFormat,
}

impl RgbaImage {
    pub fn from_raw(pixels: Vec<u8>, width: u32, height: u32, stride: u32, format: PixelFormat) -> Self {
        assert!(
            pixels.len() >= (height as usize) * (stride as usize),
            "pixel buffer too small for image dimensions"
        );
        Self {
            pixels,
            width,
            height,
            stride,
            format,
        }
    }
}

impl std::fmt::Debug for RgbaImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RgbaImage")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("len", &self.pixels.len())
            .finish()
    }
}

/// How the engine should drive a backend's rasterizer over the page's tiles.
///
/// Reported by [`RenderBackend::raster_strategy`] so the engine doesn't need to know
/// which concrete backend is active. The rasterizer itself ([`RenderBackend::create_rasterizer`])
/// is type-erased because it operates on pipeline-internal tile/texture types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RasterStrategy {
    /// Parallel per-tile rasterization with a dirty-tile pixel cache (CPU backends: Cairo, Skia).
    ParallelCached,
    /// Sequential rasterization without a dirty-tile cache (Vello: shared `Mutex<Renderer>`).
    Sequential,
    /// No rasterization at all (the null/headless backend).
    None,
}

/// Type-erased surface so the engine can hold backend-specific surfaces without generics.
pub trait ErasedSurface: Any {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
    fn size(&self) -> SurfaceSize;
}

/// Core backend interface.
pub trait RenderBackend: Send {
    fn name(&self) -> &'static str;

    fn create_surface(&self, size: SurfaceSize, present: PresentMode) -> anyhow::Result<Box<dyn ErasedSurface + Send>>;

    fn render(&self, context: &mut dyn RenderContext, surface: &mut dyn ErasedSurface) -> anyhow::Result<()>;

    fn snapshot(&self, surface: &mut dyn ErasedSurface, max_dim: u32) -> anyhow::Result<RgbaImage>;

    fn external_handle(&self, surface: &mut dyn ErasedSurface) -> anyhow::Result<ExternalHandle>;

    /// Returns the backend's shared GPU resources, type-erased, when it has any
    /// (e.g. a Vello backend's wgpu device/queue/renderer). Returns `None` otherwise.
    ///
    /// The concrete type lives in the backend's own crate (the interface is renderer-agnostic),
    /// so callers downcast the `Any` to the expected resource type.
    fn wgpu_resources(&self) -> Option<Arc<dyn Any + Send + Sync>> {
        None
    }

    /// Builds the per-tile rasterizer this backend pairs with, type-erased.
    ///
    /// The rasterizer operates on pipeline-internal types (`Tile`, `TextureStore`, `MediaStore`)
    /// that cannot live in this interface crate, so it is returned as `Box<dyn Any>`. The render
    /// pipeline boxes a `Box<dyn Rasterable>` inside it (see `gosub_render_pipeline::rasterizer`)
    /// and downcasts it back. The engine calls this once and drives it per [`Self::raster_strategy`].
    /// Defaults to a no-op marker; only backends with [`RasterStrategy`] other than
    /// [`RasterStrategy::None`] need to override it.
    fn create_rasterizer(&self) -> Box<dyn Any + Send + Sync> {
        Box::new(())
    }

    /// How the engine should drive [`Self::create_rasterizer`] over the tile set.
    /// Defaults to [`RasterStrategy::None`] (no rasterization).
    fn raster_strategy(&self) -> RasterStrategy {
        RasterStrategy::None
    }

    /// The device-pixel ratio this backend rasterizes at. Backends that rasterize at physical
    /// pixels (Cairo) override this; CSS-pixel backends (Skia, Vello) and the null backend use 1.
    fn device_pixel_ratio(&self) -> u32 {
        1
    }
}

/// Interface for compositors to receive frames from backends.
pub trait CompositorSink: Send + Sync {
    fn submit_frame(&mut self, tab: TabId, handle: ExternalHandle);
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHITE: u32 = 0xFFFF_FFFF; // opaque white, premultiplied
    const BLACK: u32 = 0xFF00_0000; // opaque black, premultiplied

    #[test]
    fn transparent_source_preserves_destination() {
        // This is the bug the blend fixes: a transparent upper-layer tile must NOT
        // erase the content drawn beneath it.
        assert_eq!(blend_over_argb_u32(0x0000_0000, WHITE), WHITE);
        assert_eq!(blend_over_argb_u32(0x0000_0000, BLACK), BLACK);
        assert_eq!(blend_over_argb_u32(0x0000_0000, 0xFF12_3456), 0xFF12_3456);
    }

    #[test]
    fn opaque_source_replaces_destination() {
        assert_eq!(blend_over_argb_u32(BLACK, WHITE), BLACK);
        assert_eq!(blend_over_argb_u32(0xFFAB_CDEF, WHITE), 0xFFAB_CDEF);
    }

    #[test]
    fn half_alpha_black_over_white_is_grey() {
        // Premultiplied 50% black = alpha 0x80, rgb 0. Over opaque white → ~50% grey,
        // still fully opaque.
        let out = blend_over_argb_u32(0x8000_0000, WHITE);
        assert_eq!(out >> 24, 0xFF, "result must be opaque");
        let r = (out >> 16) & 0xFF;
        assert!((126..=129).contains(&r), "expected ~half grey, got {r}");
    }

    #[test]
    fn rgba8_pixel_normalizes_to_argb() {
        // Rgba8 little-endian bytes [R,G,B,A] = [0x11,0x22,0x33,0xFF] read as 0xFF332211.
        let le = u32::from_le_bytes([0x11, 0x22, 0x33, 0xFF]);
        assert_eq!(PixelFormat::Rgba8.pixel_to_argb_u32(le), 0xFF11_2233);
    }

    #[test]
    fn premul_argb32_pixel_is_unchanged() {
        // PreMulArgb32 little-endian bytes [B,G,R,A] = [0x33,0x22,0x11,0xFF] read as 0xFF112233.
        let le = u32::from_le_bytes([0x33, 0x22, 0x11, 0xFF]);
        assert_eq!(PixelFormat::PreMulArgb32.pixel_to_argb_u32(le), 0xFF11_2233);
    }
}
