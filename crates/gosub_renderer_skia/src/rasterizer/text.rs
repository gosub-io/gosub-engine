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

    // Determine the effective wrap width using the same approach as the Cairo/Pango backend:
    //
    // 1. Measure the text unconstrained (single line) to find its natural width.
    // 2. Base width = max(taffy rect width, available_width). The rect is the text node's
    //    own taffy allocation; available_width is the parent block's content width and acts
    //    as a floor when the flex algorithm squeezes a text node to near-zero.
    // 3. If natural ≤ base + 20px tolerance the text was measured as a single line by
    //    Parley — use the natural width so Skia's slightly-different font metrics don't
    //    introduce a spurious extra line break.
    // 4. Otherwise Parley intentionally wrapped the text; use the base width so Skia
    //    reproduces the same line-break points.
    let text_single_line: String = cmd.text.lines().collect::<Vec<_>>().join(" ");
    let (natural_width, _) = font.measure_str(&text_single_line, Some(&paint));
    let base_width = (cmd.rect.width as f32).max(cmd.available_width as f32);
    let metric_slack = 20.0_f32;
    let max_width = if natural_width <= base_width + metric_slack {
        natural_width.max(base_width)
    } else {
        base_width
    };

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
