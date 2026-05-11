use crate::painter::commands::rectangle::Rectangle;
use crate::tiler::Tile;
use skia_safe::Canvas;

pub fn do_paint_rectangle(_canvas: &Canvas, _tile: &Tile, _cmd: &Rectangle) {
    unimplemented!("Skia rectangle rasterization is not yet implemented")
}
