/// Default backend that doesn't render or return anything.
///
/// Concrete rendering backends (Cairo, Skia, Vello) live in their own
/// `gosub_renderer_*` crates; the pipeline only defines the `RenderBackend`
/// trait and ships this null implementation.
pub mod null;
