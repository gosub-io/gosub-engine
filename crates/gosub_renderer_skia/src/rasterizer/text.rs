use gosub_pipeline::painter::commands::text::Text;
use gosub_pipeline::tiler::Tile;
use skia_safe::Canvas;

pub fn do_paint_text(_canvas: &Canvas, _tile: &Tile, _cmd: &Text, _dpi_scale_factor: f32) -> Result<(), anyhow::Error> {
    anyhow::bail!("Skia text rasterization is not yet implemented")
}
