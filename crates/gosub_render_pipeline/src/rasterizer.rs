use crate::common::media::MediaStore;
use crate::common::texture::TextureId;
use crate::common::texture_store::TextureStore;
use crate::tiler::Tile;
use gosub_interface::font_system::FontSystem;
use parking_lot::Mutex;
use std::any::Any;
use std::sync::Arc;

// `RasterStrategy` lives in `gosub_interface` (it is named by the `RenderBackend` trait);
// re-exported here so existing `gosub_render_pipeline::rasterizer::RasterStrategy` paths work.
pub use gosub_interface::render::backend::RasterStrategy;

pub trait Rasterable {
    fn rasterize(&self, tile: &Tile, texture_store: &mut TextureStore, media_store: &MediaStore) -> Option<TextureId>;

    /// The font system this rasterizer draws with, exposed as `dyn FontSystem` so the
    /// layout engine can share the same instance — text is then measured and drawn against
    /// one font collection (consistent metrics, fonts loaded once).
    ///
    /// Returns `None` for rasterizers that don't shape through a [`FontSystem`] (the null
    /// rasterizer, or the Pango/Cairo path); the layouter then uses its own instance.
    fn font_system(&self) -> Option<Arc<Mutex<dyn FontSystem>>> {
        None
    }
}

/// No-op rasterizer for backends that don't rasterize tiles (e.g. the null backend).
pub struct NullRasterizer;

impl Rasterable for NullRasterizer {
    fn rasterize(
        &self,
        _tile: &Tile,
        _texture_store: &mut TextureStore,
        _media_store: &MediaStore,
    ) -> Option<TextureId> {
        None
    }
}

/// Type-erase a backend's rasterizer for return from `RenderBackend::create_rasterizer`.
///
/// `Rasterable` references pipeline-internal types (`Tile`, `TextureStore`, `MediaStore`) that
/// cannot live in `gosub_interface`, so the trait method returns `Box<dyn Any>`. Backends box
/// their `Box<dyn Rasterable>` through this helper; the engine recovers it with
/// [`downcast_rasterizer`].
pub fn erase_rasterizer(rasterizer: Box<dyn Rasterable + Send + Sync>) -> Box<dyn Any + Send + Sync> {
    Box::new(rasterizer)
}

/// Recover a `Box<dyn Rasterable>` from the type-erased value produced by
/// [`erase_rasterizer`] (via `RenderBackend::create_rasterizer`). Returns `None` for backends
/// that don't supply a real rasterizer (the default no-op marker).
pub fn downcast_rasterizer(erased: Box<dyn Any + Send + Sync>) -> Option<Box<dyn Rasterable + Send + Sync>> {
    erased.downcast::<Box<dyn Rasterable + Send + Sync>>().ok().map(|b| *b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn erase_downcast_roundtrip_recovers_rasterizer() {
        let erased = erase_rasterizer(Box::new(NullRasterizer));
        assert!(downcast_rasterizer(erased).is_some());
    }

    #[test]
    fn downcast_of_non_rasterizer_marker_is_none() {
        // The default `RenderBackend::create_rasterizer` returns `Box::new(())`; the engine
        // must treat that as "no rasterizer" rather than panicking.
        let marker: Box<dyn Any + Send + Sync> = Box::new(());
        assert!(downcast_rasterizer(marker).is_none());
    }
}
