use crate::common::media::MediaId;
use crate::painter::commands::rectangle::Rectangle;
use crate::tiler::Tile;
use skia_safe::Canvas;

pub fn do_paint_svg(_canvas: &Canvas, _tile: &Tile, _media_id: MediaId, _rect: &Rectangle) {
    unimplemented!("Skia SVG rasterization is not yet implemented")
}
