use crate::painter::commands::PaintCommand;
use crate::tiler::Tile;
use skia_safe::Canvas;

#[allow(dead_code)]
pub fn do_paint(_canvas: &Canvas, _tile: &Tile, _cmd: &PaintCommand) {
    unimplemented!("Skia paint rasterization is not yet implemented")
}
