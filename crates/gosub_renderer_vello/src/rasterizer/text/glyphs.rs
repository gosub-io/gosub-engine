//! Generic glyph-run text painter (`text_glyphs` feature).
//!
//! Engine-neutral: asks the configured [`FontSystem`] — *whichever* engine that is — to shape the
//! text, then paints the returned glyph runs with `Scene::draw_glyphs`. Works with any font
//! system because the contract is raw font bytes + glyph IDs, not engine internals.

use crate::rasterizer::brush::set_brush;
use gosub_interface::font::FontStyle;
use gosub_interface::font_system::{FontStretch, FontSystem, FontWeight, ShapedRun, TextAlign, TextStyle};
use gosub_render_pipeline::common::font::{FontAlignment, FontInfo};
use gosub_render_pipeline::common::geo::Dimension;
use gosub_render_pipeline::common::media::MediaStore;
use gosub_render_pipeline::painter::commands::text::Text;
use vello::kurbo::{Affine, Rect as KurboRect};
use vello::peniko::{Blob, Fill, FontData};
use vello::Scene;

/// The neutral [`TextStyle`] for a display-list text command: the same mapping the layouter's
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

fn peniko_font(run: &ShapedRun) -> FontData {
    FontData::new(Blob::new(run.font.blob.data.clone()), run.font.blob.index)
}

pub fn do_paint_text(
    scene: &mut Scene,
    cmd: &Text,
    _tile_size: Dimension,
    affine: Affine,
    media_store: &MediaStore,
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

    // Glyph runs take only the brush; an image brush transform has no meaningful mapping onto
    // individual glyphs, so it is intentionally dropped here.
    let (vello_brush, _) = set_brush(&cmd.brush, cmd.rect, media_store);

    for run in &shaped.runs {
        let font = peniko_font(run);
        scene
            .draw_glyphs(&font)
            .brush(&vello_brush)
            .font_size(run.font_size)
            .hint(true)
            .transform(affine)
            .draw(
                Fill::NonZero,
                run.glyphs.iter().map(|g| vello::Glyph {
                    id: g.id,
                    x: (cmd.rect.x as f32 + g.x).round(),
                    y: (cmd.rect.y as f32 + g.y).round(),
                }),
            );

        // Text decorations: a filled rect per run, using the run font's own metrics.
        let mut decoration = |offset: f32, size: f32| {
            let x0 = cmd.rect.x + run.x as f64;
            let y0 = cmd.rect.y + (run.baseline + offset) as f64;
            let rect = KurboRect::new(x0, y0, x0 + run.width as f64, y0 + size.max(1.0) as f64);
            scene.fill(Fill::NonZero, affine, &vello_brush, None, &rect);
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

#[cfg(test)]
mod tests {
    use super::*;
    use gosub_fontmanager::ParleyFontSystem;
    use gosub_render_pipeline::common::geo::Rect as GeoRect;
    use gosub_render_pipeline::painter::commands::brush::Brush;
    use gosub_render_pipeline::painter::commands::color::Color;

    /// Shape "Hello" through the trait and encode it into a Vello scene — exercises the
    /// `FontBlob` → `peniko::FontData` conversion and the glyph encoding without needing a GPU.
    #[test]
    fn encodes_glyphs_into_scene() {
        let mut fs = ParleyFontSystem::new();
        let mut scene = Scene::new();

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

        let res = do_paint_text(
            &mut scene,
            &cmd,
            Dimension::new(200.0, 60.0),
            Affine::IDENTITY,
            &MediaStore::new(),
            &mut fs,
        );
        assert!(res.is_ok(), "painting failed: {res:?}");
    }
}
