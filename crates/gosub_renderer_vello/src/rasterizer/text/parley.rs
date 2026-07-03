use crate::font::parley::get_parley_layout;
use crate::rasterizer::brush::set_brush;
use gosub_fontmanager::ParleyFontSystem;
use gosub_interface::font::FontStyle;
use gosub_interface::font_system::{FontQuery, FontStretch, FontWeight};
use gosub_render_pipeline::common::geo::{Dimension, Rect};
use gosub_render_pipeline::common::media::MediaStore;
use gosub_render_pipeline::painter::commands::brush::Brush;
use gosub_render_pipeline::painter::commands::text::Text;
use parley::layout::{GlyphRun, PositionedLayoutItem};
use vello::kurbo::Affine;
use vello::peniko::Fill;
use vello::Scene;

pub fn do_paint_text(
    scene: &mut Scene,
    cmd: &Text,
    _tile_size: Dimension,
    affine: Affine,
    media_store: &MediaStore,
    parley: &mut ParleyFontSystem,
) -> Result<(), anyhow::Error> {
    // Resolve the CSS family list to a concrete font through the *same* path the layouter measured
    // against (`ParleyFontSystem::resolve`, with the implicit `sans-serif` fallback). Shaping
    // against the identical font is what keeps render-time line breaking in lockstep with the
    // measured box; letting Parley independently re-resolve the raw family list can land on a
    // wider `sans-serif` and wrap text the layouter sized as a single line.
    let families = gosub_fontmanager::parley_system::split_css_families(&cmd.font_info.family);
    let query = FontQuery {
        families: &families,
        style: FontStyle::Normal,
        weight: FontWeight(cmd.font_info.weight.clamp(1, 1000) as u16),
        stretch: FontStretch::NORMAL,
    };
    let resolved_family = match parley.resolve(&query) {
        Ok(r) => r.family,
        Err(_) => cmd.font_info.family.clone(),
    };

    let font_cx = parley.font_cx_mut();
    // Lay out (and align) within the fragment's own box width. Each text fragment is its own
    // box, already positioned by the layout engine (taffy applies `text-align` at the container
    // level), so wrapping and `text-align` must both be relative to `rect.width`. Using the parent
    // content width instead would center each fragment within the whole line — pushing short runs
    // like the " | " separators ~half the line width to the right. Because render now shapes the
    // same concrete font the layouter measured, `rect.width >= laid_width`, so this never wraps a
    // run the layouter sized as a single line.
    let layout = get_parley_layout(
        cmd.text.as_str(),
        &cmd.font_info,
        &resolved_family,
        cmd.rect.width,
        font_cx,
    );

    for line in layout.lines() {
        for item in line.items() {
            match item {
                PositionedLayoutItem::GlyphRun(glyph_run) => {
                    render_glyph_run(scene, glyph_run, &cmd.brush, &cmd.rect, affine, media_store);
                }
                PositionedLayoutItem::InlineBox(_inline_box) => {
                    continue;
                }
            };
        }
    }

    Ok(())
}

fn render_glyph_run(
    scene: &mut Scene,
    glyph_run: GlyphRun<[u8; 4]>,
    brush: &Brush,
    rect: &Rect,
    affine: Affine,
    media_store: &MediaStore,
) {
    let vello_brush = set_brush(brush, *rect, media_store);

    let mut x = glyph_run.offset() + rect.x as f32;
    let y = glyph_run.baseline() + rect.y as f32;
    let run = glyph_run.run();
    let font = run.font();
    let font_size = run.font_size();
    let synthesis = run.synthesis();
    let glyph_xform = synthesis
        .skew()
        .map(|angle| Affine::skew(angle.to_radians().tan() as f64, 0.0));
    let coords = run.normalized_coords();

    scene
        .draw_glyphs(font)
        .brush(&vello_brush)
        .glyph_transform(glyph_xform)
        .font_size(font_size)
        .hint(true)
        .transform(affine)
        .normalized_coords(coords)
        .draw(
            Fill::NonZero,
            glyph_run.glyphs().map(|glyph| {
                let gx = x + glyph.x;
                let gy = y + glyph.y;
                x += glyph.advance;

                vello::Glyph {
                    id: glyph.id as _,
                    x: gx.round(),
                    y: gy.round(),
                }
            }),
        );
}
