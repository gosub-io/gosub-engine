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
///
/// Halves round upward, as tiles are placed (`tile_device_px`). Each tile snaps in its own
/// coordinates, so a glyph on a tile edge is at 255.5 in one tile and -0.5 in the next;
/// `f64::round` takes both away from zero and the two copies landed a pixel apart.
///
/// `grid` is the whole page-to-device transform, see [`device_grid`].
fn snap(grid: Affine, x: f64, y: f64) -> (f32, f32) {
    let device = grid * Point::new(x, y);
    let back = grid.inverse() * Point::new((device.x + 0.5).floor(), (device.y + 0.5).floor());
    (back.x as f32, back.y as f32)
}

/// Where page space lands on the device: the glyphs' own `affine` (a tile's or the scroll's
/// translation, in CSS pixels), then the device scale. The scene is built in CSS pixels and the
/// renderer scales it to the device afterwards, so `affine` alone does not have it.
fn device_grid(affine: Affine, device_scale: f64) -> Affine {
    Affine::scale(device_scale) * affine
}

fn peniko_font(run: &ShapedRun) -> FontData {
    FontData::new(Blob::new(run.font.blob.data.clone()), run.font.blob.index)
}

pub fn do_paint_text(
    scene: &mut Scene,
    cmd: &Text,
    surface: Dimension,
    affine: Affine,
    device_scale: f64,
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
    let grid = device_grid(affine, device_scale);

    for run in &shaped.runs {
        let font = peniko_font(run);
        // Only the glyphs that land on the surface. A command is culled whole when it misses
        // the surface, but one that touches it brings every glyph it has, and a single run can
        // hold millions (one unbroken 1 MB line): Vello then dispatches more workgroups than
        // the GPU allows. A glyph reaches about its font size past its origin either way.
        let pad = f64::from(run.font_size) * 2.0;
        let visible = KurboRect::new(0.0, 0.0, surface.width, surface.height).inflate(pad, pad);
        scene
            .draw_glyphs(&font)
            .brush(&vello_brush)
            .font_size(run.font_size)
            .hint(true)
            .transform(affine)
            .draw(
                Fill::NonZero,
                run.glyphs
                    .iter()
                    .map(|g| (g, cmd.rect.x + f64::from(g.x), cmd.rect.y + f64::from(g.y)))
                    .filter(|&(_, x, y)| visible.contains(affine * Point::new(x, y)))
                    .map(|(g, x, y)| {
                        let (x, y) = snap(grid, x, y);
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

        // The tile path: the glyphs get only the tile's translation, in CSS pixels, and the
        // finished tile is scaled to the device afterwards. The grid has both.
        let tile = Affine::translate((-256.0, -256.0));
        let (x, _) = snap(device_grid(tile, 3.0), 300.4, 300.0);
        let device_x = (300.4_f64 - 256.0) * 3.0;
        assert!(
            (((f64::from(x) - 256.0) * 3.0) - device_x.round()).abs() < 1e-3,
            "x = {x}"
        );
        assert!((x - 300.4).abs() < 0.2, "x = {x}: snapped to the CSS grid");
    }

    /// A glyph half a device pixel left of a tile edge is drawn in both tiles, at 255.5 in the
    /// first and -0.5 in the second. Both copies land on the same device pixel.
    #[test]
    fn a_glyph_on_a_tile_edge_snaps_the_same_in_both_tiles() {
        let next = Affine::translate((-256.0, 0.0));
        assert_eq!(snap(Affine::IDENTITY, 255.5, 0.0).0, 256.0);
        assert_eq!(snap(next, 255.5, 0.0).0, 256.0);

        // At 2x a quarter CSS pixel is the same half device pixel.
        assert_eq!(snap(device_grid(Affine::IDENTITY, 2.0), 255.75, 0.0).0, 256.0);
        assert_eq!(snap(device_grid(next, 2.0), 255.75, 0.0).0, 256.0);
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
            word_break: gosub_interface::style::WordBreak::Normal,
            overflow_wrap: gosub_interface::style::OverflowWrap::Normal,
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
            1.0,
            &MediaStore::new(),
        );
        assert!(res.is_ok(), "painting failed: {res:?}");
    }

    /// One unbroken line far wider than the surface encodes only the glyphs that land on it.
    /// All of a 1 MB line went into the scene once, and Vello dispatched more workgroups than
    /// the GPU allows (GOS-287).
    #[test]
    fn encodes_only_the_glyphs_on_the_surface() {
        let mut fs = ParleyFontSystem::new();
        let font_info = FontInfo {
            family: "sans-serif".to_string(),
            size: 16.0,
            weight: 400,
            width: 100,
            slant: 0,
            line_height: Some(20.0),
            letter_spacing: 0.0,
            alignment: FontAlignment::Start,
            underline: false,
            line_through: false,
            word_break: gosub_interface::style::WordBreak::Normal,
            overflow_wrap: gosub_interface::style::OverflowWrap::Normal,
        };
        let text = "x".repeat(100_000);
        let shaped = std::sync::Arc::new(fs.shape(&text, &TextStyle::new("sans-serif", 16.0)));
        let total: usize = shaped.runs.iter().map(|r| r.glyphs.len()).sum();
        assert_eq!(total, 100_000);
        let cmd = Text::new(
            GeoRect::new(0.0, 0.0, f64::from(shaped.width), 20.0),
            &text,
            &font_info,
            Brush::Solid(Color::BLACK),
            1_000_000_000.0,
            shaped,
        );

        // Scrolled halfway along the line: the glyphs there, and none of the rest.
        let mut scene = Scene::new();
        let scrolled = Affine::translate((-f64::from(cmd.shaped.width) / 2.0, 0.0));
        do_paint_text(
            &mut scene,
            &cmd,
            Dimension::new(200.0, 60.0),
            scrolled,
            1.0,
            &MediaStore::new(),
        )
        .unwrap();
        let encoded = scene.encoding().resources.glyphs.len();
        assert!(encoded > 0 && encoded < 100, "{encoded} glyphs encoded");
    }
}
