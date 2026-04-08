/// Bridge: convert a gosub_interface document to a pipeline document.
pub mod bridge;

#[allow(unused)]
pub mod rendertree_builder;
#[allow(unused)]
pub mod layouter;
pub mod layering;
#[allow(unused)]
pub mod tiler;
#[allow(unused)]
pub mod painter;
pub mod common;

// Backend-specific rasterizers and compositors (only compiled when a backend feature is enabled)
#[cfg(any(feature = "backend_cairo", feature = "backend_vello", feature = "backend_skia"))]
pub mod rasterizer;
#[cfg(any(feature = "backend_cairo", feature = "backend_vello", feature = "backend_skia"))]
pub mod compositor;