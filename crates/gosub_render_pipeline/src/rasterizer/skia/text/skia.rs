use std::fmt::Error;
use crate::painter::commands::text::Text;
use crate::common::font::skia::get_skia_paragraph;
use crate::rasterizer::skia::paint::create_paint;
use crate::tiler::Tile;

pub fn do_paint_text(canvas: &skia_safe::Canvas, _tile: &Tile, cmd: &Text, dpi_scale_factor: f32) -> Result<(), Error> {
    let skia_paint = create_paint(&cmd.brush);
    let paragraph = get_skia_paragraph(cmd.text.as_str(), &cmd.font_info, cmd.rect.width, Some(skia_paint.paint()), dpi_scale_factor);

    paragraph.paint(canvas, (cmd.rect.x as f32, cmd.rect.y as f32));

    Ok(())
}