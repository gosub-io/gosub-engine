use crate::common::media::MediaStore;
use crate::common::texture::TextureId;
use crate::common::texture_store::TextureStore;
use crate::tiler::Tile;

pub trait Rasterable {
    fn rasterize(&self, tile: &Tile, texture_store: &mut TextureStore, media_store: &MediaStore) -> Option<TextureId>;
}

/// How the engine should drive a backend's rasterizer over the page's tiles.
///
/// Reported by [`crate::render::backend::RenderBackend::raster_strategy`] so the engine
/// doesn't need to know which concrete backend is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RasterStrategy {
    /// Parallel per-tile rasterization with a dirty-tile pixel cache (CPU backends: Cairo, Skia).
    ParallelCached,
    /// Sequential rasterization without a dirty-tile cache (Vello: shared `Mutex<Renderer>`).
    Sequential,
    /// No rasterization at all (the null/headless backend).
    None,
}

/// No-op rasterizer for backends that don't rasterize tiles (e.g. the null backend).
pub struct NullRasterizer;

impl Rasterable for NullRasterizer {
    fn rasterize(&self, _tile: &Tile, _texture_store: &mut TextureStore, _media_store: &MediaStore) -> Option<TextureId> {
        None
    }
}
