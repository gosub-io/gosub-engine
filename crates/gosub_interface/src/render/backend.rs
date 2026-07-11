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
    #[inline(always)] // hot per-pixel compositor helper; force-inline even in debug (-O0)
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
#[inline(always)] // hot per-pixel compositor helper; force-inline even in debug (-O0)
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

/// Scale a premultiplied `0xAARRGGBB` pixel by `opacity` (0.0..=1.0), returning a premultiplied
/// pixel. All four channels (alpha included) are multiplied by the same factor, which keeps the
/// pixel premultiplied and realises CSS group opacity for an opacity-promoted layer. Apply this to
/// a tile's source pixel before [`blend_over_argb_u32`] to fade the whole layer as a unit.
#[inline(always)] // hot per-pixel compositor helper; force-inline even in debug (-O0)
pub fn scale_premul_argb_u32(argb: u32, opacity: f32) -> u32 {
    if opacity >= 1.0 {
        return argb;
    }
    let factor = (opacity.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
    let rb = mul_div255_pair(argb & 0x00FF_00FF, factor);
    let ag = mul_div255_pair((argb >> 8) & 0x00FF_00FF, factor);
    (rb & 0x00FF_00FF) | ((ag & 0x00FF_00FF) << 8)
}

/// Multiply each of the two 8-bit channels packed in `pair` (`0x00XX00YY`) by `factor`
/// (0..=255) and divide by 255 with rounding, returning the packed result.
#[inline(always)] // hot per-pixel compositor helper; force-inline even in debug (-O0)
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

/// Geometry to resolve a `position: sticky` layer's offset at composite time. All values are in
/// page space (the same space as a tile's `page_x`/`page_y`). The sticky element lays out in normal
/// flow (like `relative`); this constraint shifts its whole promoted layer by a scroll-dependent,
/// cage-clamped translation when it would otherwise scroll past one of its insets. Insets are `None`
/// when `auto` (that edge does not stick). `bottom`/`right` are not represented yet — they need the
/// viewport extent, which this struct does not carry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StickyConstraint {
    /// `top` sticky inset in CSS px, `None` when `auto`.
    pub inset_top: Option<f64>,
    /// `left` sticky inset in CSS px, `None` when `auto`.
    pub inset_left: Option<f64>,
    /// The sticky element's own natural (in-flow) margin box, page space.
    pub natural_x: f64,
    pub natural_y: f64,
    pub natural_w: f64,
    pub natural_h: f64,
    /// The containing block's content box (the cage the element may not escape), page space.
    pub cage_x: f64,
    pub cage_y: f64,
    pub cage_w: f64,
    pub cage_h: f64,
}

impl StickyConstraint {
    /// Page-space translation to add on top of normal `page - scroll` placement. The same value
    /// applies to every tile in the layer, so the layer translates as a rigid unit. The clamp
    /// yields all three sticky regimes: flowing (0), stuck (tracking the inset), and shoved off by
    /// the containing block (pinned to the cage edge).
    #[inline]
    pub fn offset(&self, scroll_x: f64, scroll_y: f64) -> (f64, f64) {
        let mut dy = 0.0;
        if let Some(top) = self.inset_top {
            // Where the element's top edge would sit in the viewport under plain scrolling.
            let natural_vp_y = self.natural_y - scroll_y;
            // Push down so the top rests at `top`; never negative (never pulled above flow).
            let want = (top - natural_vp_y).max(0.0);
            // Cage slack: how far it may move before its bottom hits the container bottom. Scroll
            // cancels here, so this is pure geometry.
            let slack = ((self.cage_y + self.cage_h) - (self.natural_y + self.natural_h)).max(0.0);
            dy = want.min(slack);
        }

        let mut dx = 0.0;
        if let Some(left) = self.inset_left {
            let natural_vp_x = self.natural_x - scroll_x;
            let want = (left - natural_vp_x).max(0.0);
            let slack = ((self.cage_x + self.cage_w) - (self.natural_x + self.natural_w)).max(0.0);
            dx = want.min(slack);
        }

        (dx, dy)
    }
}

/// How a tile's layer responds to page scroll at composite time.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum TileAnchor {
    /// Normal flow: the tile scrolls with the page (composited at `page - scroll`).
    #[default]
    Scroll,
    /// `position: fixed`: the tile is pinned to the viewport and ignores scroll
    /// (composited at its page position, which equals its viewport position).
    Fixed,
    /// `position: sticky`: the tile scrolls normally until it would cross one of its insets, then
    /// it sticks at the inset, clamped so it never leaves its containing block.
    Sticky(StickyConstraint),
}

/// Effective top-left of a tile in viewport coordinates, given its page-space position, the current
/// scroll offset and its anchor. Fixed tiles ignore scroll so they stay pinned to the viewport;
/// sticky tiles scroll normally plus a clamped catch-up translation.
#[inline]
pub fn anchored_tile_pos(page_x: f64, page_y: f64, scroll_x: f64, scroll_y: f64, anchor: TileAnchor) -> (f64, f64) {
    match anchor {
        TileAnchor::Scroll => (page_x - scroll_x, page_y - scroll_y),
        TileAnchor::Fixed => (page_x, page_y),
        TileAnchor::Sticky(c) => {
            let (dx, dy) = c.offset(scroll_x, scroll_y);
            (page_x - scroll_x + dx, page_y - scroll_y + dy)
        }
    }
}

/// A single pre-rasterized tile for direct compositing in the host draw callback.
/// Pixel data is reference-counted (`Bytes`) so handing out a handle is zero-copy.
#[derive(Clone, Debug)]
pub struct CachedTile {
    pub page_x: f32,
    pub page_y: f32,
    pub width: u32,
    pub height: u32,
    pub data: bytes::Bytes,
    /// In-memory byte order of `data`, set by the rasterizer that produced it.
    pub format: PixelFormat,
    /// Group opacity (1.0 = opaque) of the tile's layer. The compositor scales the tile's
    /// premultiplied pixels by this before the source-over blend, fading opacity-promoted layers
    /// (e.g. a translucent fixed navbar) as a whole.
    pub opacity: f32,
    /// How this tile's layer responds to scroll (normal flow vs. `position: fixed`).
    pub anchor: TileAnchor,
    /// True when every pixel is fully opaque (alpha == 255). Computed once when the tile is cached;
    /// lets a CPU compositor blit the tile with a plain row copy instead of a per-pixel source-over.
    pub opaque: bool,
}

/// A rasterized tile that lives in a GPU backend's texture store, positioned in page coordinates.
/// The `texture_id` is opaque outside the backend that produced it — it resolves the GPU texture
/// in that backend's own store. Handed to [`RenderBackend::composite_tiles`].
#[derive(Clone, Debug)]
pub struct PlacedGpuTile {
    pub page_x: f32,
    pub page_y: f32,
    pub width: u32,
    pub height: u32,
    pub texture_id: u64,
    /// Group opacity (1.0 = opaque) of the tile's layer, applied by the GPU compositor.
    pub opacity: f32,
    /// How this tile's layer responds to scroll (normal flow vs. `position: fixed`).
    pub anchor: TileAnchor,
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
    ///
    /// `font_system` is the engine's single shared font system (the config's `FontSystem`).
    /// The rasterizer exposes it to the layouter so measurement uses the configured instance;
    /// painting consumes the pre-shaped glyph runs carried on the text paint commands.
    fn create_rasterizer(
        &self,
        font_system: Arc<parking_lot::Mutex<dyn crate::font_system::FontSystem>>,
    ) -> Box<dyn Any + Send + Sync> {
        let _ = font_system;
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

    /// Whether the backend composites its rasterized tiles into a GPU texture and exposes it via
    /// [`Self::render`] + [`Self::external_handle`], rather than shipping CPU tiles for the host
    /// to composite (an `ExternalHandle::TileCache`).
    ///
    /// `false` (default) keeps the CPU TileCache path used by Cairo/Skia. `true` routes the tab
    /// worker through the display-list path so the host receives an `ExternalHandle::WgpuTextureId`.
    /// Only meaningful for backends whose [`Self::raster_strategy`] rasterizes tiles.
    fn renders_to_gpu_texture(&self) -> bool {
        false
    }

    /// Whether this GPU backend wants the **shared tile pipeline** rather than its own one-shot
    /// scene path: the engine rasterizes tiles (into GPU textures, via [`Self::create_rasterizer`])
    /// and calls [`Self::composite_tiles`] to present them. Only consulted when
    /// [`Self::renders_to_gpu_texture`] is also true. Default `false` keeps the scene path.
    fn gpu_tile_compositing(&self) -> bool {
        false
    }

    /// Composite GPU-resident tiles (produced by this backend's rasterizer, see
    /// [`Self::create_rasterizer`]) into `surface`, for the given viewport and scroll offset.
    ///
    /// This is the GPU analogue of the host's CPU tile compositing: the shared tile pipeline
    /// rasterizes every tile (CPU bytes *or* a GPU texture id) and a GPU backend blits the visible
    /// GPU tiles into its surface here, after which [`Self::external_handle`] yields the presentable
    /// `WgpuTextureId`. `tiles` carry backend-owned `texture_id`s in page coordinates.
    ///
    /// Default is unsupported; only GPU backends override it. Lets one tile pipeline serve CPU and
    /// GPU backends, differing only in where tile pixels live and who composites them.
    fn composite_tiles(
        &self,
        _surface: &mut dyn ErasedSurface,
        _tiles: &[PlacedGpuTile],
        _viewport: (u32, u32),
        _scroll: (f32, f32),
        _page_height: f32,
    ) -> anyhow::Result<()> {
        anyhow::bail!("composite_tiles not supported by backend '{}'", self.name())
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
    fn sticky_top_three_regimes() {
        // A 60px-tall navbar with natural top at y=220, top:0 inset, inside a 1000px-tall cage
        // starting at y=200. Cage bottom = 1200; element bottom = 280; slack = 920.
        let c = StickyConstraint {
            inset_top: Some(0.0),
            inset_left: None,
            natural_x: 0.0,
            natural_y: 220.0,
            natural_w: 100.0,
            natural_h: 60.0,
            cage_x: 0.0,
            cage_y: 200.0,
            cage_w: 100.0,
            cage_h: 1000.0,
        };

        // Phase 1 — flowing: scrolled less than the natural top, element still below the inset.
        let (_, dy) = c.offset(0.0, 100.0); // natural_vp_y = 120 > 0 → no stick
        assert_eq!(dy, 0.0);

        // Phase 2 — stuck: scrolled past the natural top, element pinned at inset 0.
        let (_, dy) = c.offset(0.0, 500.0); // natural_vp_y = -280; want = 280, < slack 920
        assert_eq!(dy, 280.0);
        // viewport top = natural_y - scroll + dy = 220 - 500 + 280 = 0 (pinned at top:0).
        assert_eq!(220.0 - 500.0 + dy, 0.0);

        // Phase 3 — shoved off: scrolled so far the cage bottom drags it; offset clamps to slack.
        let (_, dy) = c.offset(0.0, 5000.0); // want = 4780, clamped to slack 920
        assert_eq!(dy, 920.0);
    }

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
