use crate::font::skia::build_paragraph;
use gosub_render_pipeline::painter::commands::brush::Brush;
use gosub_render_pipeline::painter::commands::gradient::Gradient;
use gosub_render_pipeline::painter::commands::text::Text;
use skia_safe::{Canvas, Color4f, Paint};

/// Draws through Skia's own textlayout (via the shared thread-local font collection),
/// re-shaping from `cmd.text` + `cmd.font_info`; `cmd.shaped` is ignored.
pub fn do_paint_text(canvas: &Canvas, cmd: &Text, _dpi_scale_factor: f32) -> Result<(), anyhow::Error> {
    if cmd.text.is_empty() || cmd.font_info.size <= 0.0 {
        return Ok(());
    }

    let color4f = brush_to_color4f(&cmd.brush);
    let mut paint = Paint::new(color4f, None);
    paint.set_anti_alias(true);

    // Lay out and paint through the same Skia `textlayout` engine used for measurement, so the draw
    // metrics match the layout metrics (correct wrapping and line-height) and CSS alignment /
    // underline / line-through are honoured natively.
    //
    // Wrap within `available_width` — the container width the layout engine used as its wrap limit
    // — so we reproduce the same line breaks it did; fall back to the box width if it is larger.
    // For centre/right alignment this same width is the box the paragraph aligns within, and we
    // paint at the box's left edge, so aligned text lands where layout expects.
    let layout_width = (cmd.available_width as f32).max(cmd.rect.width as f32).max(1.0);

    let paragraph = build_paragraph(&cmd.text, &cmd.font_info, &paint, layout_width);
    // `paint` takes the paragraph's top-left; the line box (including line-height leading) starts
    // at `rect.y`, matching how the box was sized during measurement.
    paragraph.paint(canvas, (cmd.rect.x as f32, cmd.rect.y as f32));

    Ok(())
}

fn brush_to_color4f(brush: &Brush) -> Color4f {
    match brush {
        Brush::Solid(c) => Color4f::new(c.r(), c.g(), c.b(), c.a()),
        // Gradient text fills aren't supported in the text path; approximate with the
        // first colour stop so glyphs stay visible rather than defaulting to black.
        Brush::Gradient(Gradient::Linear(g)) => match g.stops.first() {
            Some(stop) => Color4f::new(stop.color.r(), stop.color.g(), stop.color.b(), stop.color.a()),
            None => Color4f::new(0.0, 0.0, 0.0, 1.0),
        },
        Brush::Image(_) => Color4f::new(0.0, 0.0, 0.0, 1.0),
    }
}
