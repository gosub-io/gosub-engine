use gosub_pipeline::common::media::MediaStore;
use gosub_pipeline::painter::commands::text::Text;
use gosub_pipeline::tiler::Tile;
use gtk4::cairo::{Context, Error};

#[allow(dead_code)]
pub(crate) fn do_paint_text(_cr: &Context, _tile: &Tile, _cmd: &Text, _media_store: &MediaStore) -> Result<(), Error> {
    Err(Error::UserFontNotImplemented)
}
