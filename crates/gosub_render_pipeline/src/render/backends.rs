/// Default backend that doesn't render or return anything. Real backends (Cairo, Skia, Vello)
/// live in their own `gosub_renderer_*` crates.
pub mod null;
