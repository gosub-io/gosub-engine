use gosub_render_pipeline::common::media::MediaStore;
use gosub_render_pipeline::painter::commands::text::Text;
use gosub_render_pipeline::tiler::Tile;
use cairo::{Context, Error};

#[allow(dead_code)]
pub(crate) fn do_paint_text(_cr: &Context, _tile: &Tile, _cmd: &Text, _media_store: &MediaStore) -> Result<(), Error> {
    log::warn!("Skia text rendering is not implemented for the Cairo backend");
    Ok(())
}
