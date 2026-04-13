//! Render backend abstraction.
//!
//! This module defines the traits and data structures needed to implement
//! different rendering backends (e.g. Cairo, Vello, Skia). Backends provide
//! surfaces for rendering, expose handles to host applications, and generate
//! snapshots for thumbnails/tab switchers.
//!
//! # Key Concepts
//!
//! - [`RenderBackend`]: Core trait that all backends must implement.
//! - [`ErasedSurface`]: Type-erased handle to backend-specific surfaces.
//! - [`ExternalHandle`]: Exported surface handle for compositing in the host.
//! - [`CompositorSink`]: Interface through which backends submit frames.
//! - [`SurfaceSize`], [`PresentMode`], [`PixelFormat`]: Configuration types
//!   for creating surfaces.
//!
//! Backends differ in how they manage memory, synchronization, and ownership.
//! Some are CPU-bound (Cairo), others GPU-accelerated (Vello, Skia, OpenGL).

use crate::render::{RenderList, Viewport};
use parking_lot::RwLock;
use std::any::Any;
use std::ptr::NonNull;
use std::sync::Arc;

/// A surface rect has the same properties as a viewport, but a surface rect
/// is usually computed with DevicePixelRatio.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct SurfaceRect {
    pub x: i32, // physical px
    pub y: i32,
    pub width: u32, // physical px
    pub height: u32,
}

/// Size of a rendering surface in pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceSize {
    /// Width of the surface in pixels.
    pub width: u32,
    /// Height of the surface in pixels.
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

/// Present modes for rendering.
///
/// These modes influence how frames are synchronized with the display.
#[derive(Clone, Copy, Debug)]
pub enum PresentMode {
    /// FIFO (vsync-aligned). Produces stable frame pacing.
    Fifo,
    /// Immediate mode. Frames are presented as soon as possible.
    Immediate,
    // Mailbox,
    // FifoRelaxed,
}

/// Pixel format for surfaces and snapshots.
#[derive(Clone, Copy, Debug)]
pub enum PixelFormat {
    /// 32-bit ARGB with premultiplied alpha.
    PreMulArgb32,
    /// 8-bit RGBA.
    Rgba8,
}

/// Pixel format for GPU textures.
#[derive(Clone, Copy, Debug)]
pub enum GpuPixelFormat {
    /// 32-bit BGRA with sRGB color space.
    Bgra8UnormSrgb,
    /// 32-bit RGBA with sRGB color space.
    Rgba8UnormSrgb,
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub struct WgpuTextureId(pub u64);

/// Safety: `ExternalHandle` can be sent between threads, but not shared.
unsafe impl Send for ExternalHandle {}
unsafe impl Sync for ExternalHandle {}

/// Handle that the host/browser can use to composite a surface.
///
/// Ownership and synchronization rules are backend-specific.
/// Each variant provides different trade-offs (safety, performance).
#[derive(Clone, Debug)]
pub enum ExternalHandle {
    /// No-op handle. Useful for testing or headless operation. Never shows any pixels
    NullHandle {
        /// Width of the surface in pixels.
        width: u32,
        /// Height of the surface in pixels.
        height: u32,
        /// Frame ID for synchronization. Optional, can be `0` if not used.
        frame_id: u64,
    },

    /// CPU pixels in RGBA8. Safer owned alternative to raw pointers.
    CpuPixelsOwned {
        /// Width of the image in pixels.
        width: u32,
        /// Height of the image in pixels.
        height: u32,
        /// Stride in bytes. This is the number of bytes per row of pixels.
        stride: u32,
        /// Raw pixel data in RGBA8 format.
        pixels: Vec<u8>,
        /// Pixel format of the image.
        format: PixelFormat,
    },

    /// CPU pixels as a borrowed pointer. UNSAFE: caller must respect lifetime/size/stride.
    /// Valid for at least `height * stride` bytes until the next `render()` call on this surface.
    CpuPixelsPtr {
        /// Width of the image in pixels.
        width: u32,
        /// Height of the image in pixels.
        height: u32,
        /// Stride in bytes. This is the number of bytes per row of pixels.
        stride: u32,
        /// Raw pixel data pointer in RGBA8 format.
        pixel_buf: NonNull<u8>, // This is not SEND + SYNC, we override this above
    },

    /// GL / GLES texture. `target` is usually GL_TEXTURE_2D or GL_TEXTURE_EXTERNAL_OES.
    /// Optional `frame_id` helps hosts avoid sampling stale frames.
    GlTexture {
        /// OpenGL texture ID.
        tex: u32,
        /// OpenGL texture target (e.g., GL_TEXTURE_2D).
        target: u32,
        /// Width of the texture in pixels.
        width: u32,
        /// Height of the texture in pixels.
        height: u32,
        /// Frame ID for synchronization. Optional, can be `0` if not used.
        frame_id: u64,
    },

    /// WGPU/Vello app-owned indirection. Contract: host can resolve `id` to a usable texture.
    WgpuTextureId {
        /// Unique texture ID managed by the host application (for instance, in its texture store)
        id: u64,
        /// Width of the texture in pixels.
        width: u32,
        /// Height of the texture in pixels.
        height: u32,
        /// WGPU texture format (e.g., TextureFormat::Rgba8UnormSrgb).
        format: GpuPixelFormat,
        /// Frame ID for synchronization. Optional, can be `0` if not used.
        frame_id: u64,
    },

    /// Skia image handle/ID (e.g., promise image). Contract to be defined with the host.
    SkiaImageId {
        /// Unique image ID managed by the host application.
        id: u64,
        /// Width of the image in pixels.
        width: u32,
        /// Height of the image in pixels.
        height: u32,
        /// Frame ID for synchronization. Optional, can be `0` if not used.
        frame_id: u64,
    },
}

/// Small RGBA image, typically used for thumbnails or previews.
#[derive(Clone)]
pub struct RgbaImage {
    /// Raw pixel data in RGBA8 format.
    pub pixels: Vec<u8>,
    /// Width of the image in pixels.
    pub width: u32,
    /// Height of the image.
    pub height: u32,
    /// Stride in bytes. This is the number of bytes per row of pixels.
    pub stride: u32,
    /// Pixel format of the image.
    pub format: PixelFormat,
}

impl RgbaImage {
    /// Construct an [`RgbaImage`] from raw pixel data.
    ///
    /// # Panics
    ///
    /// Panics if `pixels.len()` is smaller than `height * stride`.
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

/// Type-erased surface so the engine can hold backend-specific surfaces
/// without requiring generics or enums.
///
/// Each backend defines its own concrete surface type and erases it behind
/// this trait for use by the engine core.
pub trait ErasedSurface: Any {
    /// Returns a reference to the underlying concrete type.
    fn as_any(&self) -> &dyn Any;

    /// Returns a mutable reference to the underlying concrete type.
    fn as_any_mut(&mut self) -> &mut dyn Any;

    /// Returns the surface size in pixels.
    fn size(&self) -> SurfaceSize;
}

/// Type-erased interface used by render backends to access the data they need
/// from a browsing context without requiring a concrete generic type parameter.
pub trait RenderContext {
    /// Returns the current viewport.
    fn viewport(&self) -> &Viewport;
    /// Returns the current render list.
    fn render_list(&self) -> &RenderList;
}

/// Core backend interface.
///
/// Implemented by all rendering backends. The engine calls these methods
/// on the backend's owning thread.
pub trait RenderBackend: Send {
    fn name(&self) -> &'static str;

    /// Create a new surface with the given size and present mode.
    fn create_surface(&self, size: SurfaceSize, present: PresentMode) -> anyhow::Result<Box<dyn ErasedSurface + Send>>;

    /// Render the current state of the browsing context to the given surface.
    fn render(&self, context: &mut dyn RenderContext, surface: &mut dyn ErasedSurface) -> anyhow::Result<()>;

    /// Generate a small RGBA8 snapshot of the surface, suitable for thumbnails or previews.
    fn snapshot(&self, surface: &mut dyn ErasedSurface, max_dim: u32) -> anyhow::Result<RgbaImage>;

    /// Returns an external handle for the surface, if supported.
    fn external_handle(&self, surface: &mut dyn ErasedSurface) -> anyhow::Result<ExternalHandle>;
}

/// Interface for compositors to receive frames from backends.
///
/// A [`CompositorSink`] is typically implemented by the host application.
/// After rendering, the backend calls [`CompositorSink::submit_frame`] with an [`ExternalHandle`]
/// that the host can composite into its UI.
pub trait CompositorSink: Send + Sync {
    /// Submit a rendered frame for the given tab.
    fn submit_frame(&mut self, tab: crate::tab::TabId, handle: ExternalHandle);
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
}
