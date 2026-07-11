//! Generic glyph-run text painter (`text_glyphs` feature).
//!
//! Engine-neutral: asks the configured [`FontSystem`] — *whichever* engine that is — to shape the
//! text, then paints the returned glyph runs with `Scene::draw_glyphs`. Works with any font
//! system because the contract is raw font bytes + glyph IDs, not engine internals.

use crate::rasterizer::brush::set_brush;
use gosub_interface::font_system::{FontSystem, ShapedRun};
use gosub_render_pipeline::common::geo::Dimension;
use gosub_render_pipeline::common::media::MediaStore;
use gosub_render_pipeline::painter::commands::text::Text;
use vello::kurbo::{Affine, Rect as KurboRect};
use vello::peniko::{Blob, Fill, FontData};
use vello::Scene;

fn peniko_font(run: &ShapedRun) -> FontData {
    FontData::new(Blob::new(run.font.blob.data.clone()), run.font.blob.index)
}

/// `_font_system` is unused: the command carries its pre-shaped glyph runs (shaped once at
/// paint-command build time by the pipeline Painter). The parameter exists so this variant
/// shares a signature with the engine-native `text_parley` rasterizer, which re-shapes.
pub fn do_paint_text(
    scene: &mut Scene,
    cmd: &Text,
    _tile_size: Dimension,
    affine: Affine,
    media_store: &MediaStore,
    _font_system: &mut dyn FontSystem,
) -> Result<(), anyhow::Error> {
    let shaped = &cmd.shaped;
    if shaped.is_empty() {
        return Ok(());
    }

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
    use gosub_interface::font_system::TextStyle;
    use gosub_render_pipeline::common::font::{FontAlignment, FontInfo};
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
        let mut style = TextStyle::new("sans-serif", 24.0);
        style.line_height = Some(28.0);
        style.max_width = Some(180.0);
        let shaped = fs.shape("Hello", &style);
        let cmd = Text::new(
            GeoRect::new(10.0, 10.0, 180.0, 40.0),
            "Hello",
            &font_info,
            Brush::Solid(Color::BLACK),
            180.0,
            shaped,
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
