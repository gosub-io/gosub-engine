//! Generic glyph-run text painter (`text_glyphs` feature).
//!
//! Engine-neutral: asks the configured [`FontSystem`] — *whichever* engine that is — to shape the
//! text, then paints the returned glyph runs as Skia text blobs built from the runs' raw font
//! bytes. Works with any font system because the contract is font bytes + glyph IDs, not engine
//! internals.

use gosub_interface::font::FontStyle;
use gosub_interface::font_system::{FontStretch, FontSystem, FontWeight, TextAlign, TextStyle};
use gosub_render_pipeline::common::font::{FontAlignment, FontInfo};
use gosub_render_pipeline::painter::commands::brush::Brush;
use gosub_render_pipeline::painter::commands::gradient::Gradient;
use gosub_render_pipeline::painter::commands::text::Text;
use skia_safe::{Canvas, Color4f, Font as SkFont, FontMgr, Paint, Point, Rect, TextBlobBuilder, Typeface};
use std::cell::RefCell;
use std::collections::HashMap;

/// The neutral [`TextStyle`] for a display-list text command — the same mapping the layouter's
/// measure path uses, plus wrap width and alignment, so shaping reproduces the measured box.
fn text_style_for(font_info: &FontInfo, max_width: f32) -> TextStyle {
    TextStyle {
        family: font_info.family.clone(),
        size: font_info.size as f32,
        weight: FontWeight(font_info.weight.clamp(1, 1000) as u16),
        style: if font_info.slant != 0 {
            FontStyle::Italic
        } else {
            FontStyle::Normal
        },
        stretch: FontStretch::NORMAL,
        line_height: Some(font_info.line_height as f32),
        letter_spacing: font_info.letter_spacing as f32,
        max_width: Some(max_width),
        align: match font_info.alignment {
            FontAlignment::Start => TextAlign::Start,
            FontAlignment::Center => TextAlign::Center,
            FontAlignment::End => TextAlign::End,
            FontAlignment::Justify => TextAlign::Justify,
        },
        display_scale: 1.0,
    }
}

/// A cheap, stable identity for a font blob: length + head/tail content hash + collection index.
/// Deliberately *not* the `Arc` data pointer — an address can be recycled for a different font
/// after a blob is dropped, which would alias cache keys.
fn blob_fingerprint(blob: &gosub_interface::font::FontBlob) -> (u64, u32) {
    use std::hash::{Hash, Hasher};
    let bytes = blob.as_u8();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.len().hash(&mut h);
    bytes[..bytes.len().min(1024)].hash(&mut h);
    if bytes.len() > 1024 {
        bytes[bytes.len() - 1024..].hash(&mut h);
    }
    (h.finish(), blob.index)
}

thread_local! {
    /// Typefaces instantiated from shaped-run font bytes; parsing a font file per glyph run
    /// would be prohibitive.
    static RUN_TYPEFACES: RefCell<HashMap<(u64, u32), Option<Typeface>>> = RefCell::new(HashMap::new());
}

fn typeface_for(blob: &gosub_interface::font::FontBlob) -> Option<Typeface> {
    let key = blob_fingerprint(blob);
    RUN_TYPEFACES.with(|cell| {
        cell.borrow_mut()
            .entry(key)
            .or_insert_with(|| FontMgr::new().new_from_data(blob.as_u8(), blob.index as usize))
            .clone()
    })
}

pub fn do_paint_text(
    canvas: &Canvas,
    cmd: &Text,
    _dpi_scale_factor: f32,
    font_system: &mut dyn FontSystem,
) -> Result<(), anyhow::Error> {
    if cmd.text.is_empty() || cmd.font_info.size <= 0.0 {
        return Ok(());
    }

    // Wrap limit: Start-aligned text wraps within the container width the layouter used, so the
    // painted line breaks reproduce the measured ones (fragments can carry whole multi-line
    // paragraphs). Center/End/Justify text instead uses the fragment's own box as its alignment
    // container — glyphs shifted outside the fragment rect would land in tiles that never
    // repaint this command.
    let start_width = (cmd.available_width as f32).max(cmd.rect.width as f32).max(1.0);
    let mut style = text_style_for(&cmd.font_info, start_width);
    if style.align != TextAlign::Start {
        style.max_width = Some((cmd.rect.width as f32).max(1.0));
    }
    let shaped = font_system.shape(&cmd.text, &style);

    let mut paint = Paint::new(brush_to_color4f(&cmd.brush), None);
    paint.set_anti_alias(true);

    let (x0, y0) = (cmd.rect.x as f32, cmd.rect.y as f32);

    for run in &shaped.runs {
        let Some(typeface) = typeface_for(&run.font.blob) else {
            continue;
        };
        let font = SkFont::from_typeface(typeface, run.font_size);

        let mut builder = TextBlobBuilder::new();
        let (glyph_ids, points) = builder.alloc_run_pos(&font, run.glyphs.len(), None);
        for (i, g) in run.glyphs.iter().enumerate() {
            glyph_ids[i] = g.id as u16;
            points[i] = Point::new(g.x, g.y);
        }
        if let Some(text_blob) = builder.make() {
            canvas.draw_text_blob(&text_blob, (x0, y0), &paint);
        }

        // Text decorations: a filled rect per run, using the run font's own metrics.
        let decoration = |offset: f32, size: f32| {
            let dx = x0 + run.x;
            let dy = y0 + run.baseline + offset;
            canvas.draw_rect(Rect::new(dx, dy, dx + run.width, dy + size.max(1.0)), &paint);
        };
        if cmd.font_info.underline {
            decoration(run.metrics.underline_offset, run.metrics.underline_size);
        }
        if cmd.font_info.line_through {
            decoration(run.metrics.strikethrough_offset, run.metrics.strikethrough_size);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font::skia::SkiaFontSystem;
    use gosub_render_pipeline::common::geo::Rect as GeoRect;
    use gosub_render_pipeline::painter::commands::color::Color;

    /// End-to-end paint through the generic glyph path: shape "Hello" with the trait, paint it
    /// onto a white raster canvas, and assert dark pixels landed inside the text box.
    #[test]
    fn paints_visible_glyphs() {
        let mut fs = SkiaFontSystem;

        let info = skia_safe::ImageInfo::new(
            skia_safe::ISize::new(200, 60),
            skia_safe::ColorType::BGRA8888,
            skia_safe::AlphaType::Premul,
            None,
        );
        let Some(mut surface) = skia_safe::surfaces::raster(&info, None, None) else {
            panic!("failed to create raster surface");
        };
        let canvas = surface.canvas();
        canvas.clear(Color4f::new(1.0, 1.0, 1.0, 1.0));

        let font_info = FontInfo {
            family: "sans-serif".to_string(),
            size: 24.0,
            weight: 400,
            width: 100,
            slant: 0,
            line_height: 28.0,
            letter_spacing: 0.0,
            alignment: FontAlignment::Start,
            underline: true,
            line_through: false,
        };
        let cmd = Text::new(
            GeoRect::new(10.0, 10.0, 180.0, 40.0),
            "Hello",
            &font_info,
            Brush::Solid(Color::BLACK),
            180.0,
        );

        let res = do_paint_text(canvas, &cmd, 1.0, &mut fs);
        assert!(res.is_ok(), "painting failed: {res:?}");

        let Some(pixmap) = canvas.peek_pixels() else {
            panic!("failed to peek pixels");
        };
        let Some(bytes) = pixmap.bytes() else {
            panic!("failed to read pixel bytes");
        };
        // Count pixels that are meaningfully darker than the white background.
        let dark = bytes.chunks_exact(4).filter(|px| px[0] < 128 && px[1] < 128 && px[2] < 128).count();
        assert!(dark > 20, "expected dark glyph pixels on the canvas, found {dark}");
    }
}
