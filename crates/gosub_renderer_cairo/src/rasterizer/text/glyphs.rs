//! Generic glyph-run text painter (`text_glyphs` feature).
//!
//! Engine-neutral: asks the configured [`FontSystem`] — *whichever* engine that is — to shape the
//! text, then paints the returned glyph runs with `cairo_show_glyphs` against FreeType font faces
//! created from the runs' raw font bytes. Works with any font system because the contract is font
//! bytes + glyph IDs, not engine internals.
//!
//! Colour emoji render via cairo's FreeType colour-bitmap support (CBDT strikes); COLR-only
//! fonts may still fall back to monochrome outlines depending on the cairo version.

use crate::rasterizer::brush::set_brush;
use cairo::{Antialias, Context, Error, FontOptions, Glyph, HintMetrics, HintStyle};
use gosub_interface::font::FontStyle;
use gosub_interface::font_system::{FontStretch, FontSystem, FontWeight, TextAlign, TextStyle};
use gosub_render_pipeline::common::font::{FontAlignment, FontInfo};
use gosub_render_pipeline::common::media::MediaStore;
use gosub_render_pipeline::painter::commands::text::Text;
use gosub_render_pipeline::tiler::Tile;
use std::collections::HashMap;
use std::rc::Rc;

/// Pango marks glyphs missing from the font with this flag (`PANGO_GLYPH_UNKNOWN_FLAG`); their
/// IDs don't index the font file, so painting them through FreeType would draw garbage.
const PANGO_GLYPH_UNKNOWN_FLAG: u32 = 0x1000_0000;

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

/// Process-global, immortal FreeType library + cairo font faces, one per distinct font file.
///
/// Global and immortal is **load-bearing**, not convenience: cairo internally caches
/// `cairo_font_face_t`s keyed by `FT_Face` *pointer*. Per-thread caches (tiles rasterize on
/// short-lived pool threads) freed their faces on thread death, and a later thread's new
/// `FT_Face` allocated at a recycled address made cairo resurrect the stale entry — painting
/// entire runs as solid boxes. Faces created once and never freed can't collide; it also means
/// the multi-MB CJK/emoji fonts are copied once per process instead of once per thread.
struct FaceCache {
    faces: HashMap<(u64, u32), Option<(freetype::face::Face, cairo::FontFace)>>,
    library: Option<freetype::Library>,
}

#[allow(unsafe_code)]
// SAFETY: the raw FT_Library/FT_Face/cairo_font_face_t pointers inside are only dereferenced
// while holding the `FACES` mutex (creation) or by cairo during painting — and cairo-ft
// serialises its own FT_Face access internally (the unscaled-font lock around every FreeType
// call). cairo font-face reference counting is itself thread-safe.
unsafe impl Send for FaceCache {}

fn faces() -> &'static parking_lot::Mutex<FaceCache> {
    static FACES: std::sync::OnceLock<parking_lot::Mutex<FaceCache>> = std::sync::OnceLock::new();
    FACES.get_or_init(|| {
        parking_lot::Mutex::new(FaceCache {
            faces: HashMap::new(),
            library: None,
        })
    })
}

/// Fallback font system for rasterizers created without a shared engine font system: the same
/// Pango engine the layouter falls back to on this backend, so measure and draw still agree.
#[cfg(feature = "text_pango")]
pub(crate) fn fallback_font_system() -> &'static parking_lot::Mutex<crate::font::pango::PangoFontSystem> {
    static FS: std::sync::OnceLock<parking_lot::Mutex<crate::font::pango::PangoFontSystem>> =
        std::sync::OnceLock::new();
    FS.get_or_init(|| parking_lot::Mutex::new(crate::font::pango::PangoFontSystem::new()))
}

/// A cairo font face for a shaped run's font bytes. The FreeType face it wraps stays alive in
/// the global cache forever (cairo references, but does not own, the `FT_Face`).
fn cairo_face_for(blob: &gosub_interface::font::FontBlob) -> Option<cairo::FontFace> {
    let key = blob_fingerprint(blob);
    let mut cache = faces().lock();
    if !cache.faces.contains_key(&key) {
        if cache.library.is_none() {
            cache.library = freetype::Library::init().ok();
        }
        let entry = cache.library.as_ref().and_then(|lib| {
            let bytes = Rc::new(blob.as_u8().to_vec());
            let face = lib.new_memory_face(bytes, blob.index as isize).ok()?;
            let font_face = cairo::FontFace::create_from_ft(&face).ok()?;
            Some((face, font_face))
        });
        cache.faces.insert(key, entry);
    }
    cache.faces.get(&key).and_then(|e| e.as_ref().map(|(_, ff)| ff.clone()))
}

pub(crate) fn do_paint_text(
    cr: &Context,
    tile: &Tile,
    cmd: &Text,
    media_store: &MediaStore,
    font_system: &mut dyn FontSystem,
) -> Result<(), Error> {
    if cmd.text.is_empty() || cmd.font_info.size <= 0.0 {
        return Ok(());
    }

    // Wrap limit: Start-aligned text wraps within the container width the layouter used, so the
    // painted line breaks reproduce the measured ones (fragments can carry whole multi-line
    // paragraphs). Center/End/Justify text instead uses the fragment's own box as its alignment
    // container — glyphs shifted outside the fragment rect would land in tiles that never
    // repaint this command.
    let start_width = cmd.available_width.max(cmd.rect.width).max(1.0) as f32;
    let mut style = text_style_for(&cmd.font_info, start_width);
    if style.align != TextAlign::Start {
        style.max_width = Some(cmd.rect.width.max(1.0) as f32);
    }
    let shaped = font_system.shape(&cmd.text, &style);

    cr.save()?;
    // The current path is NOT part of cairo's save/restore state, and this context is shared
    // with every other painter in the tile loop: a leftover path from a previous command would
    // be included in our decoration `fill()`, painting its whole rect in the text colour.
    cr.new_path();
    // Map page coordinates onto the tile; the context's existing scale handles DPR.
    cr.translate(-tile.rect.x, -tile.rect.y);

    if let Ok(mut font_opts) = FontOptions::new() {
        font_opts.set_antialias(Antialias::Gray);
        // Match the Pango-native path: slight hinting nudges stems toward the pixel grid for
        // crispness without the heavy snapping that distorts glyph shapes at small sizes.
        font_opts.set_hint_style(HintStyle::Slight);
        font_opts.set_hint_metrics(HintMetrics::On);
        cr.set_font_options(&font_opts);
    }
    set_brush(cr, &cmd.brush, cmd.rect, media_store);

    for run in &shaped.runs {
        let Some(face) = cairo_face_for(&run.font.blob) else {
            continue;
        };
        cr.set_font_face(&face);
        cr.set_font_size(run.font_size as f64);

        let glyphs: Vec<Glyph> = run
            .glyphs
            .iter()
            .filter(|g| g.id & PANGO_GLYPH_UNKNOWN_FLAG == 0)
            .map(|g| {
                Glyph::new(
                    g.id as std::os::raw::c_ulong,
                    cmd.rect.x + g.x as f64,
                    cmd.rect.y + g.y as f64,
                )
            })
            .collect();
        if !glyphs.is_empty() {
            cr.show_glyphs(&glyphs)?;
        }

        // Text decorations: a filled rect per run, using the run font's own metrics.
        let decoration = |offset: f32, size: f32| -> Result<(), Error> {
            cr.rectangle(
                cmd.rect.x + run.x as f64,
                cmd.rect.y + (run.baseline + offset) as f64,
                run.width as f64,
                size.max(1.0) as f64,
            );
            cr.fill()
        };
        if cmd.font_info.underline {
            decoration(run.metrics.underline_offset, run.metrics.underline_size)?;
        }
        if cmd.font_info.line_through {
            decoration(run.metrics.strikethrough_offset, run.metrics.strikethrough_size)?;
        }
    }

    cr.restore()?;
    Ok(())
}

#[cfg(all(test, feature = "text_pango"))]
mod tests {
    use super::*;
    use crate::font::pango::PangoFontSystem;
    use gosub_render_pipeline::common::geo::Rect as GeoRect;
    use gosub_render_pipeline::painter::commands::brush::Brush;
    use gosub_render_pipeline::painter::commands::color::Color;


    /// End-to-end paint through the generic glyph path: shape "Hello" with the Pango font system,
    /// paint it via FreeType + `show_glyphs` onto a white surface, and assert dark pixels landed.
    #[test]
    fn paints_visible_glyphs() {
        let mut fs = PangoFontSystem::new();

        let Ok(surface) = cairo::ImageSurface::create(cairo::Format::ARgb32, 200, 60) else {
            panic!("failed to create surface");
        };
        let Ok(cr) = Context::new(&surface) else {
            panic!("failed to create context");
        };
        cr.set_source_rgb(1.0, 1.0, 1.0);
        let _ = cr.paint();

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
        let tile = Tile {
            id: gosub_render_pipeline::tiler::TileId::new(0),
            layer_id: gosub_render_pipeline::layering::layer::LayerId::new(0),
            elements: Vec::new(),
            texture_id: None,
            state: gosub_render_pipeline::tiler::TileState::Dirty,
            rect: GeoRect::new(0.0, 0.0, 200.0, 60.0),
            bgcolor: None,
        };

        let media_store = MediaStore::new();
        let res = do_paint_text(&cr, &tile, &cmd, &media_store, &mut fs);
        assert!(res.is_ok(), "painting failed: {res:?}");

        drop(cr);
        let mut surface = surface;
        surface.flush();
        let Ok(data) = surface.data() else {
            panic!("failed to read surface data");
        };
        let dark = data
            .chunks_exact(4)
            .filter(|px| px[0] < 128 && px[1] < 128 && px[2] < 128)
            .count();
        assert!(dark > 20, "expected dark glyph pixels on the surface, found {dark}");
    }
}
