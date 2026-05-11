use crate::painter::commands::text::Text;
use crate::tiler::Tile;
use gtk4::cairo::{Context, Error};

#[allow(dead_code)]
pub(crate) fn do_paint_text(_cr: &Context, _tile: &Tile, _cmd: &Text) -> Result<(), Error> {
    Err(Error::UserFontNotImplemented)
}
