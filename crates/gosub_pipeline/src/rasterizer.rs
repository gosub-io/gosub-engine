use crate::common::media::MediaStore;
use crate::common::texture::TextureId;
use crate::common::texture_store::TextureStore;
use crate::tiler::Tile;

// Rasterizing can be pretty simple by itself, since it only needs to execute the paint commands for
// the specific 2D library we are using. All calculations should have been done in the layouter.

#[cfg(feature = "backend_cairo")]
pub mod cairo;
#[cfg(feature = "backend_skia")]
pub mod skia;
#[cfg(feature = "backend_vello")]
pub mod vello;

pub trait Rasterable {
    fn rasterize(&self, tile: &Tile, texture_store: &mut TextureStore, media_store: &MediaStore) -> Option<TextureId>;
}
