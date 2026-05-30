use gosub_render_pipeline::painter::commands::PaintCommand;
use gosub_render_pipeline::tiler::Tile;
use skia_safe::Canvas;

#[allow(dead_code)]
pub fn do_paint(_canvas: &Canvas, _tile: &Tile, _cmd: &PaintCommand) {
    log::warn!("Skia paint rasterization is not yet implemented");
}
