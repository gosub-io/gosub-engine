use gosub_render_pipeline::painter::commands::rectangle::Rectangle;
use gosub_render_pipeline::tiler::Tile;
use skia_safe::Canvas;

pub fn do_paint_rectangle(_canvas: &Canvas, _tile: &Tile, _cmd: &Rectangle) {
    log::warn!("Skia rectangle rasterization is not yet implemented");
}
