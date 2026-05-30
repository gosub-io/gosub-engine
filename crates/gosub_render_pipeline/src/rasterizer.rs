use crate::common::media::MediaStore;
use crate::common::texture::TextureId;
use crate::common::texture_store::TextureStore;
use crate::tiler::Tile;

pub trait Rasterable {
    fn rasterize(&self, tile: &Tile, texture_store: &mut TextureStore, media_store: &MediaStore) -> Option<TextureId>;
}
