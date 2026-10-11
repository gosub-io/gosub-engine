//! Generic glyph-run text painter (`text_glyphs` feature).
//!
//! Font-engine-neutral: the contract is raw font bytes + glyph IDs, not engine internals, so any
//! [`FontSystem`] implementation works here.

use crate::rasterizer::brush::set_brush;
use gosub_interface::font_system::ShapedRun;
use gosub_render_pipeline::common::geo::Dimension;
use gosub_render_pipeline::common::media::MediaStore;
use gosub_render_pipeline::painter::commands::text::Text;
use vello::kurbo::{Affine, Point, Rect as KurboRect};
use vello::peniko::{Blob, Fill, FontData};
use vello::Scene;

/// Snap a glyph origin, given in page space, to the device-pixel grid it is drawn on.
///
/// Snapping keeps glyphs crisp, but the grid is the device's, not the CSS pixel's: under a 3x
/// scale (a phone) a whole CSS pixel is three device pixels, and rounding there moved each glyph
/// up to one and a half device pixels from where the shaper put it. Neighbouring glyphs moved
/// by different amounts, so "Show" read as "Sh ow". At 1x the two grids are the same.
fn snap(affine: Affine, x: f64, y: f64) -> (f32, f32) {
    let device = affine * Point::new(x, y);
    let back = affine.inverse() * Point::new(device.x.round(), device.y.round());
    (back.x as f32, back.y as f32)
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
) -> Result<(), anyhow::Error> {
    // Shaping already happened at paint-command build time, with the same font system the layouter
    // measured with; this function only paints the runs.
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
                run.glyphs.iter().map(|g| {
                    let (x, y) = snap(affine, cmd.rect.x + f64::from(g.x), cmd.rect.y + f64::from(g.y));
                    vello::Glyph { id: g.id, x, y }
                }),
            );

        // Decorations are a filled rect per run, using the run font's own metrics.
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
    use gosub_interface::font_system::{FontSystem, TextStyle};
    use gosub_render_pipeline::common::font::{FontAlignment, FontInfo};
    use gosub_render_pipeline::common::geo::Rect as GeoRect;
    use gosub_render_pipeline::painter::commands::brush::Brush;
    use gosub_render_pipeline::painter::commands::color::Color;

    /// Glyphs land on device pixels, not CSS pixels: under a 3x scale a glyph at 10.4 CSS px
    /// stays within a third of a CSS pixel of where it was shaped, instead of jumping to 10.
    #[test]
    fn glyphs_snap_to_device_pixels() {
        let phone = Affine::scale(3.0);
        let (x, y) = snap(phone, 10.4, 20.0);
        assert!((x - 31.0 / 3.0).abs() < 1e-4, "x = {x}");
        assert!((y - 20.0).abs() < 1e-4, "y = {y}");

        // At 1x the device grid is the CSS grid, as before.
        assert_eq!(snap(Affine::IDENTITY, 10.4, 20.6), (10.0, 21.0));

        // A tile's translation is part of the grid.
        let tile = Affine::scale(3.0).then_translate((-768.0, -256.0).into());
        let (x, _) = snap(tile, 300.4, 100.0);
        let device_x = 300.4_f64 * 3.0 - 768.0;
        assert!(
            ((f64::from(x) * 3.0 - 768.0) - device_x.round()).abs() < 1e-3,
            "x = {x}"
        );
    }

    /// Exercises the `FontBlob` → `peniko::FontData` conversion and glyph encoding without a GPU.
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
            line_height: Some(28.0),
            letter_spacing: 0.0,
            alignment: FontAlignment::Start,
            underline: true,
            line_through: false,
        };
        let mut style = TextStyle::new("sans-serif", 24.0);
        style.line_height = Some(28.0);
        style.max_width = Some(180.0);
        let shaped = std::sync::Arc::new(fs.shape("Hello", &style));
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
        );
        assert!(res.is_ok(), "painting failed: {res:?}");
    }
}
