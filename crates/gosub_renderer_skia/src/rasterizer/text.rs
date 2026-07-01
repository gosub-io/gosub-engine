use crate::font::skia::{is_generic_family, split_font_families, web_font_generation, web_font_mgr};
use gosub_render_pipeline::painter::commands::brush::Brush;
use gosub_render_pipeline::painter::commands::gradient::Gradient;
use gosub_render_pipeline::painter::commands::text::Text;
use gosub_render_pipeline::tiler::Tile;
use skia_safe::{Canvas, Color4f, Font, FontMgr, FontStyle, Paint, Typeface};
use std::cell::RefCell;
use std::collections::HashMap;

// FontMgr is the font resolver; the typeface cache avoids repeated family-style lookups.
// Key: (family, weight, width, slant_nonzero, web_font_generation) — the generation evicts
// stale entries when an @font-face web font is registered. Size is baked into the Font.
type TypefaceCacheKey = (String, i32, i32, bool, u64);

thread_local! {
    static FONT_MGR: FontMgr = FontMgr::new();
    static TYPEFACE_CACHE: RefCell<HashMap<TypefaceCacheKey, Typeface>> = RefCell::new(HashMap::new());
}

fn css_font_style(weight: i32, width: i32, slant: i32) -> FontStyle {
    FontStyle::new(
        skia_safe::font_style::Weight::from(weight),
        skia_safe::font_style::Width::from(width.clamp(1, 9)),
        if slant > 0 {
            skia_safe::font_style::Slant::Italic
        } else {
            skia_safe::font_style::Slant::Upright
        },
    )
}

fn get_font(family: &str, weight: i32, width: i32, slant: i32, size: f32) -> Option<Font> {
    let font_style = css_font_style(weight, width, slant);

    let cache_key = (family.to_string(), weight, width, slant > 0, web_font_generation());

    // Try to reuse a cached typeface — match_family_style is non-trivial work.
    TYPEFACE_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(tf) = cache.get(&cache_key) {
            return Some(Font::new(tf.clone(), size));
        }
        let tf = FONT_MGR.with(|fm| resolve_typeface(fm, family, font_style))?;
        cache.insert(cache_key, tf.clone());
        Some(Font::new(tf, size))
    })
}

/// Resolve a typeface by walking the CSS `font-family` fallback chain. Registered
/// `@font-face` web fonts take priority for an exact family match; otherwise each requested
/// family is tried in order, and a concrete family is only accepted when the manager
/// actually has it (its returned typeface name-matches), so a missing font like
/// `Source Serif 4` falls through to the next entry (e.g. `Georgia`, then the generic
/// `serif`) instead of silently becoming the default sans-serif. Generic keywords accept
/// whatever the manager maps them to.
fn resolve_typeface(fm: &FontMgr, families: &str, font_style: FontStyle) -> Option<Typeface> {
    let web = web_font_mgr();
    for name in split_font_families(families) {
        if let Some(web) = web.as_ref() {
            if let Some(tf) = web.match_family_style(&name, font_style) {
                return Some(tf);
            }
        }
        let Some(tf) = fm.match_family_style(&name, font_style) else {
            continue;
        };
        if is_generic_family(&name) || tf.family_name().eq_ignore_ascii_case(&name) {
            return Some(tf);
        }
    }
    // Nothing in the chain resolved — fall back to the platform default.
    fm.legacy_make_typeface(None, font_style)
        .or_else(|| fm.legacy_make_typeface(None, FontStyle::normal()))
}

pub fn do_paint_text(canvas: &Canvas, _tile: &Tile, cmd: &Text, _dpi_scale_factor: f32) -> Result<(), anyhow::Error> {
    let color4f = brush_to_color4f(&cmd.brush);
    let mut paint = Paint::new(color4f, None);
    paint.set_anti_alias(true);

    let font_size = cmd.font_info.size as f32;
    let font_style = css_font_style(cmd.font_info.weight, cmd.font_info.width / 100, cmd.font_info.slant);
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
            draw_line_with_fallback(canvas, line, &font, font_style, font_size, x, y, &paint);
        }
        y += line_height;
        if y > (cmd.rect.y + cmd.rect.height) as f32 + line_height {
            break;
        }
    }

    Ok(())
}

/// Draw one already-wrapped line, falling back to another font for any character the primary
/// `base_font` has no glyph for. Skia's `draw_str` renders with a single typeface and shows
/// missing glyphs as tofu boxes (e.g. an arrow/icon char in a serif body font). We split the
/// line into runs that share a font — the base font, or a per-character fallback resolved via
/// the font manager — and draw each run, advancing the pen by its measured width.
fn draw_line_with_fallback(
    canvas: &Canvas,
    line: &str,
    base_font: &Font,
    base_style: FontStyle,
    size: f32,
    mut x: f32,
    y: f32,
    paint: &Paint,
) {
    let mut run = String::new();
    let mut run_font = base_font.clone();

    for ch in line.chars() {
        // Whitespace and characters the base font can render stay on the base font; only a
        // genuine missing glyph triggers a fallback lookup.
        let ch_font = if ch.is_whitespace() || base_font.unichar_to_glyph(ch as i32) != 0 {
            base_font.clone()
        } else {
            FONT_MGR
                .with(|fm| fm.match_family_style_character("", base_style, &[], ch as i32))
                .map(|tf| Font::new(tf, size))
                .unwrap_or_else(|| base_font.clone())
        };

        if run.is_empty() {
            run_font = ch_font;
        } else if ch_font.typeface().unique_id() != run_font.typeface().unique_id() {
            canvas.draw_str(run.as_str(), (x, y), &run_font, paint);
            x += run_font.measure_str(run.as_str(), Some(paint)).0;
            run.clear();
            run_font = ch_font;
        }
        run.push(ch);
    }

    if !run.is_empty() {
        canvas.draw_str(run.as_str(), (x, y), &run_font, paint);
    }
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
