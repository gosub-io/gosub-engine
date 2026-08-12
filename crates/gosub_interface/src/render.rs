//! Render backend contract types.
//!
//! These traits and value types define the boundary between the render pipeline
//! (which produces a [`RenderList`]) and the concrete render backends (Cairo, Skia,
//! Vello) that consume it. They live here, in the interface crate, so that
//! `ModuleConfiguration` can name a `RenderBackend`/`CompositorSink` without inverting
//! the dependency direction. `gosub_render_pipeline` re-exports them for downstream code.

pub mod backend;
pub mod render_context;
pub mod render_list;
pub mod viewport;

pub use backend::{
    blend_over_argb_u32, CompositorSink, ErasedSurface, ExternalHandle, GpuPixelFormat, PixelFormat, PresentMode,
    RasterStrategy, RenderBackend, RgbaImage, SurfaceRect, SurfaceSize, WgpuTextureId,
};
pub use render_context::RenderContext;
pub use render_list::{Color, DisplayItem, RenderList};
pub use viewport::{DevicePixelRatio, Viewport, DEVICE_PIXEL_RATIO};
