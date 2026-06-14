/// Default backend that doesn't render or return anything.
pub mod null;

/// Backend using the Vello graphics library.
#[cfg(feature = "backend_vello")]
pub mod vello;

/// Backend using the Skia graphics library (CPU rasterizer).
#[cfg(feature = "backend_skia")]
pub mod skia;

/// Backend using Skia with OpenGL GPU acceleration.
#[cfg(feature = "backend_skia_gl")]
pub mod skia_gpu;
