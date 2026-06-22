use gosub_render_pipeline::painter::commands::brush::Brush;
use gosub_render_pipeline::painter::commands::gradient::Gradient;
use gosub_render_pipeline::painter::commands::text::Text;
use gosub_render_pipeline::tiler::Tile;
use skia_safe::{Canvas, Color4f, Font, FontMgr, FontStyle, Paint, Typeface};
use std::cell::RefCell;
use std::collections::HashMap;

// FontMgr is the font resolver; the typeface cache avoids repeated family-style lookups.
// Key: (family, weight, width, slant_nonzero, size_bits) — size is baked into the Font object.
thread_local! {
    static FONT_MGR: FontMgr = FontMgr::new();
    static TYPEFACE_CACHE: RefCell<HashMap<(String, i32, i32, bool), Typeface>> =
        RefCell::new(HashMap::new());
}

fn get_font(family: &str, weight: i32, width: i32, slant: i32, size: f32) -> Option<Font> {
    let font_style = FontStyle::new(
        skia_safe::font_style::Weight::from(weight),
        skia_safe::font_style::Width::from(width.clamp(1, 9)),
        if slant > 0 {
            skia_safe::font_style::Slant::Italic
        } else {
            skia_safe::font_style::Slant::Upright
        },
    );

    let cache_key = (family.to_string(), weight, width, slant > 0);

    // Try to reuse a cached typeface — match_family_style is non-trivial work.
    TYPEFACE_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(tf) = cache.get(&cache_key) {
            return Some(Font::new(tf.clone(), size));
        }
        let tf = FONT_MGR.with(|fm| {
            fm.match_family_style(family, font_style)
                .or_else(|| fm.legacy_make_typeface(None, font_style))
                .or_else(|| fm.legacy_make_typeface(None, FontStyle::normal()))
        })?;
        cache.insert(cache_key, tf.clone());
        Some(Font::new(tf, size))
    })
}

pub fn do_paint_text(canvas: &Canvas, _tile: &Tile, cmd: &Text, _dpi_scale_factor: f32) -> Result<(), anyhow::Error> {
    let color4f = brush_to_color4f(&cmd.brush);
    let mut paint = Paint::new(color4f, None);
    paint.set_anti_alias(true);

    let font_size = cmd.font_info.size as f32;
    let Some(font) = get_font(
        &cmd.font_info.family,
        cmd.font_info.weight,
        cmd.font_info.width / 100,
        cmd.font_info.slant,
        font_size,
    ) else {
        return Ok(());
    };

    // Measure the full text as a single line to determine the natural (unconstrained) width.
    // This matches Parley's measuring step so we use the same wrap decision Parley made.
    let text_single_line: String = cmd.text.lines().collect::<Vec<_>>().join(" ");
    let (natural_width, _) = font.measure_str(&text_single_line, Some(&paint));
    let base_width = (cmd.rect.width as f32).max(cmd.available_width as f32);
    let max_width = if natural_width <= base_width + 20.0 {
        natural_width.max(base_width)
    } else {
        base_width
    };

    // O(N) word-wrap: measure each word once, accumulate widths with a precomputed space
    // advance.  The previous O(N²) approach measured the whole accumulated string on every
    // word, multiplying font-shaping work by the average line length.
    let space_width = font.measure_str(" ", Some(&paint)).0;
    let mut lines: Vec<String> = Vec::new();
    for paragraph in cmd.text.lines() {
        let mut current = String::new();
        let mut current_width = 0.0f32;
        for word in paragraph.split_whitespace() {
            let (word_width, _) = font.measure_str(word, Some(&paint));
            let candidate_width = if current.is_empty() {
                word_width
            } else {
                current_width + space_width + word_width
            };
            if candidate_width <= max_width || current.is_empty() {
                if !current.is_empty() {
                    current.push(' ');
                }
                current.push_str(word);
                current_width = candidate_width;
            } else {
                lines.push(std::mem::take(&mut current));
                current = word.to_string();
                current_width = word_width;
            }
        }
        lines.push(current);
    }

    let line_height = cmd.font_info.line_height as f32;
    let x = cmd.rect.x as f32;
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
        // Gradient text fills aren't supported in the text path; approximate with the
        // first colour stop so glyphs stay visible rather than defaulting to black.
        Brush::Gradient(Gradient::Linear(g)) => match g.stops.first() {
            Some(stop) => Color4f::new(stop.color.r(), stop.color.g(), stop.color.b(), stop.color.a()),
            None => Color4f::new(0.0, 0.0, 0.0, 1.0),
        },
        Brush::Image(_) => Color4f::new(0.0, 0.0, 0.0, 1.0),
    }
}
