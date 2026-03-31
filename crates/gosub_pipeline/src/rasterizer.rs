use crate::common::texture::TextureId;
use crate::tiler::Tile;

// Rasterizing can be pretty simple by itself, since it only needs to execute the paint commands for
// the specific 2D library we are using. All calculations should have been done in the layouter.

#[cfg(not(any(feature = "backend_cairo", feature = "backend_vello", feature = "backend_skia")))]
compile_error!("Either the 'backend_cairo' 'backend_skia' or 'backend_vello' feature must be enabled");

#[cfg(feature="backend_cairo")]
pub mod cairo;
#[cfg(feature="backend_vello")]
pub mod vello;
#[cfg(feature="backend_skia")]
pub mod skia;

pub trait Rasterable {
    fn rasterize(&self, tile: &Tile) -> Option<TextureId>;
}