use gosub_render_pipeline::painter::commands::brush::Brush;
use gosub_render_pipeline::painter::commands::text::Text;
use gosub_render_pipeline::tiler::Tile;
use skia_safe::{Canvas, Color4f, Font, FontMgr, FontStyle, Paint};

thread_local! {
    static FONT_MGR: FontMgr = FontMgr::new();
}

pub fn do_paint_text(canvas: &Canvas, _tile: &Tile, cmd: &Text, _dpi_scale_factor: f32) -> Result<(), anyhow::Error> {
    let color4f = brush_to_color4f(&cmd.brush);
    let mut paint = Paint::new(color4f, None);
    paint.set_anti_alias(true);

    let font_style = FontStyle::new(
        skia_safe::font_style::Weight::from(cmd.font_info.weight),
        skia_safe::font_style::Width::from((cmd.font_info.width / 100).clamp(1, 9)),
        if cmd.font_info.slant > 0 {
            skia_safe::font_style::Slant::Italic
        } else {
            skia_safe::font_style::Slant::Upright
        },
    );

    let Some(typeface) = FONT_MGR.with(|fm| {
        fm.match_family_style(&cmd.font_info.family, font_style)
            .or_else(|| fm.legacy_make_typeface(None, font_style))
            .or_else(|| fm.legacy_make_typeface(None, FontStyle::normal()))
    }) else {
        return Ok(());
    };

    let font_size = cmd.font_info.size as f32;
    let font = Font::new(typeface, font_size);
    // Determine how many lines Parley allocated for this text node.
    // If it's a single line, use a huge max_width so Skia never wraps due to
    // font-metric differences. Only multi-line nodes need a real width constraint.
    let parley_line_count = (cmd.rect.height as f32 / cmd.font_info.line_height as f32 + 0.1).floor() as u32;
    let max_width = if parley_line_count <= 1 { 1_000_000_000.0_f32 } else { cmd.available_width as f32 };

    // Word-wrap: greedily pack words into lines that fit within max_width.
    let mut lines: Vec<String> = Vec::new();
    for paragraph in cmd.text.lines() {
        let mut current = String::new();
        for word in paragraph.split_whitespace() {
            let candidate = if current.is_empty() {
                word.to_string()
            } else {
                format!("{current} {word}")
            };
            let (measured, _) = font.measure_str(&candidate, Some(&paint));
            if measured <= max_width || current.is_empty() {
                current = candidate;
            } else {
                lines.push(std::mem::take(&mut current));
                current = word.to_string();
            }
        }
        lines.push(current);
    }

    let line_height = cmd.font_info.line_height as f32;
    let x = cmd.rect.x as f32;
    // y is the text baseline; offset by font_size from the top of the rect.
    let mut y = cmd.rect.y as f32 + font_size;

    for line in &lines {
        if !line.is_empty() {
            canvas.draw_str(line.as_str(), (x, y), &font, &paint);
        }
        y += line_height;
        if y > (cmd.rect.y + cmd.rect.height) as f32 + line_height {
            break;
        }
    }

    Ok(())
}

fn brush_to_color4f(brush: &Brush) -> Color4f {
    match brush {
        Brush::Solid(c) => Color4f::new(c.r(), c.g(), c.b(), c.a()),
        Brush::Image(_) => Color4f::new(0.0, 0.0, 0.0, 1.0),
    }
}
