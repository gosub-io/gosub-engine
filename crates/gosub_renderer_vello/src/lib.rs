// wgpu's deeply nested generic types push auto-trait (`Send`/`Sync`) solving past the default
// limit of 128; nightly's `recursion_depth_exceeding_limit` lint makes that a hard error.
#![recursion_limit = "256"]

pub mod backend;
pub(crate) mod gpu_tiles;
pub mod rasterizer;

pub use backend::{VelloBackend, WgpuContextProvider, WgpuResources};
pub use rasterizer::VelloRasterizer;
