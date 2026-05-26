use gosub_pipeline::common::media::MediaId;
use gosub_pipeline::painter::commands::rectangle::Rectangle;
use gosub_pipeline::tiler::Tile;
use skia_safe::Canvas;

pub fn do_paint_svg(_canvas: &Canvas, _tile: &Tile, _media_id: MediaId, _rect: &Rectangle) {
    unimplemented!("Skia SVG rasterization is not yet implemented")
}
