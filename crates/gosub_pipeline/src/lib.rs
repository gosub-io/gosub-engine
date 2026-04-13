/// Bridge: convert a gosub_interface document to a pipeline document.
pub mod bridge;

pub mod common;
pub mod layering;
#[allow(unused)]
pub mod layouter;
#[allow(unused)]
pub mod painter;
#[allow(unused)]
pub mod rendertree_builder;
#[allow(unused)]
pub mod tiler;

// Backend-specific rasterizers and compositors (only compiled when a backend feature is enabled)
#[cfg(any(feature = "backend_cairo", feature = "backend_vello", feature = "backend_skia"))]
pub mod compositor;
#[cfg(any(feature = "backend_cairo", feature = "backend_vello", feature = "backend_skia"))]
pub mod rasterizer;
