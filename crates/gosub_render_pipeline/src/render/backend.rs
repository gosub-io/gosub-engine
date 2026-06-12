use crate::render::render_context::RenderContext;
use crate::render::viewport::Viewport;
use gosub_shared::tab_id::TabId;
use parking_lot::RwLock;
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

    /// Returns the shared wgpu resources (device, queue, renderer) when this is a Vello backend.
    /// Returns `None` for all other backends.
    #[cfg(feature = "backend_vello")]
    fn wgpu_resources(&self) -> Option<std::sync::Arc<crate::render::backends::vello::WgpuResources>> {
        None
    }
}

/// Interface for compositors to receive frames from backends.
pub trait CompositorSink: Send + Sync {
    fn submit_frame(&mut self, tab: TabId, handle: ExternalHandle);
}

/// Thread-safe router for switching between multiple render backends at runtime.
pub struct RenderBackendRouter {
    inner: RwLock<Arc<dyn RenderBackend + Send + Sync>>,
}

impl RenderBackendRouter {
    pub fn new(initial: Arc<dyn RenderBackend + Send + Sync>) -> Arc<Self> {
        Arc::new(Self {
            inner: RwLock::new(initial),
        })
    }

    pub fn set_backend(&self, backend: Arc<dyn RenderBackend + Send + Sync>) {
        *self.inner.write() = backend;
    }

    #[inline]
    pub fn current(&self) -> Arc<dyn RenderBackend + Send + Sync> {
        self.inner.read().clone()
    }
}

impl RenderBackend for RenderBackendRouter {
    fn name(&self) -> &'static str {
        self.current().name()
    }

    fn create_surface(&self, size: SurfaceSize, present: PresentMode) -> anyhow::Result<Box<dyn ErasedSurface + Send>> {
        self.current().create_surface(size, present)
    }

    fn render(&self, context: &mut dyn RenderContext, surface: &mut dyn ErasedSurface) -> anyhow::Result<()> {
        self.current().render(context, surface)
    }

    fn snapshot(&self, surface: &mut dyn ErasedSurface, max_dim: u32) -> anyhow::Result<RgbaImage> {
        self.current().snapshot(surface, max_dim)
    }

    fn external_handle(&self, surface: &mut dyn ErasedSurface) -> anyhow::Result<ExternalHandle> {
        self.current().external_handle(surface)
    }

    #[cfg(feature = "backend_vello")]
    fn wgpu_resources(&self) -> Option<std::sync::Arc<crate::render::backends::vello::WgpuResources>> {
        self.current().wgpu_resources()
    }
}
