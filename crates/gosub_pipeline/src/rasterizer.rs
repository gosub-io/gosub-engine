use crate::common::texture::TextureId;
use crate::tiler::Tile;

// Rasterizing can be pretty simple by itself, since it only needs to execute the paint commands for
// the specific 2D library we are using. All calculations should have been done in the layouter.

#[cfg(feature = "backend_cairo")]
pub mod cairo;
#[cfg(feature = "backend_vello")]
pub mod vello;

pub trait Rasterable {
    fn rasterize(&self, tile: &Tile) -> Option<TextureId>;
}
