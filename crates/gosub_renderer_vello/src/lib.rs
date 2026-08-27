// wgpu's resource types nest deeply enough that the trait solver gives up proving `Send`,
// `Sync` and the `Arc`/`Box` unsizing coercions for them at the default depth. Nightly
// reports that as a future-compat error rather than backing off, so raise the ceiling.
// See rust-lang/rust#159228.
#![recursion_limit = "256"]

pub mod backend;
pub(crate) mod gpu_tiles;
pub mod rasterizer;

pub use backend::{VelloBackend, WgpuContextProvider, WgpuResources};
pub use rasterizer::VelloRasterizer;
