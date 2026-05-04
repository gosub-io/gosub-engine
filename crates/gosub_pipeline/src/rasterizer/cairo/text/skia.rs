use crate::painter::commands::text::Text;
use crate::tiler::Tile;
use gtk4::cairo::{Context, Error};

pub(crate) fn do_paint_text(_cr: &Context, _tile: &Tile, _cmd: &Text) -> Result<(), Error> {
    unimplemented!("Skia text rendering is not implemented for the Cairo backend")
}
