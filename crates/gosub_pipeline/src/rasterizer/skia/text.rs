use crate::painter::commands::text::Text;
use crate::tiler::Tile;
use skia_safe::Canvas;

pub fn do_paint_text(_canvas: &Canvas, _tile: &Tile, _cmd: &Text, _dpi_scale_factor: f32) -> Result<(), anyhow::Error> {
    unimplemented!("Skia text rasterization is not yet implemented")
}
