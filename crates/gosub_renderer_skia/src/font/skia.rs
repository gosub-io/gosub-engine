use gosub_interface::font::{FontBlob, FontError, FontStyle as CssFontStyle};
use gosub_interface::font_system::{
    FontQuery, FontSystem, ResolvedFont, RunMetrics, ShapedGlyph, ShapedRun, ShapedText,
    TextAlign as GosubTextAlign, TextStyle as GosubTextStyle,
};
#[cfg(not(feature = "text_glyphs"))]
use gosub_render_pipeline::common::font::{FontAlignment, FontInfo};
use parking_lot::Mutex;
use skia_safe::textlayout::{
    FontCollection, Paragraph, ParagraphBuilder, ParagraphStyle, TextAlign, TextStyle, TypefaceFontProvider,
};
#[cfg(not(feature = "text_glyphs"))]
use skia_safe::textlayout::{TextDecoration, TextDirection};
#[cfg(not(feature = "text_glyphs"))]
use skia_safe::Paint;
use skia_safe::{FontMgr, FontStyle};
use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

// ── Registered web fonts ──────────────────────────────────────────────────────
//
// `@font-face` fonts are registered as raw bytes in a process-global registry (bytes are
// `Send + Sync`; Skia's `Typeface`/`FontMgr` are not). Each thread lazily materialises the
// bytes into a `TypefaceFontProvider` when the registry's generation changes, so both the
// measure path (this module's `FontCollection`) and the draw path (`rasterizer::text`) see
// newly-registered fonts without sharing non-`Send` Skia objects across threads.

struct FontRegistry {
    generation: u64,
    fonts: Vec<(String, Vec<u8>)>,
}

fn registry() -> &'static Mutex<FontRegistry> {
    static REGISTRY: OnceLock<Mutex<FontRegistry>> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        Mutex::new(FontRegistry {
            generation: 0,
            fonts: Vec::new(),
        })
    })
}

/// The current registry generation; bumped each time a font is registered.
pub(crate) fn web_font_generation() -> u64 {
    registry().lock().generation
}

/// Build a `FontMgr` (backed by a `TypefaceFontProvider`) containing every registered web
/// font, or `None` if none are registered or none could be decoded.
///
/// A variable font is registered as one instance per standard CSS weight stop (100–900)
/// within its `wght` axis range, not just its default instance. Google Fonts serves a single
/// variable file for a multi-weight `@font-face` set (e.g. `wght@600;700` → one file whose
/// default instance is 400); registering only the default made every bold-ish weight request
/// fall back to faux-bold of the 400 instance. With per-weight instances,
/// `FontCollection::matchStyle` selects the genuinely-instanced weight.
fn build_web_font_mgr() -> Option<FontMgr> {
    let reg = registry().lock();
    if reg.fonts.is_empty() {
        return None;
    }
    let fm = FontMgr::new();
    let mut provider = TypefaceFontProvider::new();
    let mut any = false;
    for (family, bytes) in reg.fonts.iter() {
        let Some(tf) = fm.new_from_data(bytes, None) else {
            continue;
        };
        for instance in weight_instances(&tf) {
            provider.register_typeface(instance, Some(family.as_str()));
            any = true;
        }
    }
    any.then(|| provider.into())
}

/// The typefaces to register for `tf`: for a variable font with a `wght` axis, one instance
/// per standard CSS weight stop (100–900, clamped to the axis range, deduplicated); otherwise
/// just the typeface itself.
fn weight_instances(tf: &skia_safe::Typeface) -> Vec<skia_safe::Typeface> {
    use skia_safe::font_arguments::variation_position::Coordinate;
    use skia_safe::font_arguments::VariationPosition;
    use skia_safe::{FontArguments, FourByteTag};

    let wght = FourByteTag::from_chars('w', 'g', 'h', 't');
    let axis = tf
        .variation_design_parameters()
        .and_then(|axes| axes.into_iter().find(|a| a.tag == wght));
    let Some(axis) = axis else {
        return vec![tf.clone()];
    };

    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for stop in (1..=9).map(|i| (i * 100) as f32) {
        let value = stop.clamp(axis.min, axis.max);
        if !seen.insert(value as i32) {
            continue;
        }
        let coords = [Coordinate { axis: wght, value }];
        let args = FontArguments::new().set_variation_design_position(VariationPosition { coordinates: &coords });
        if let Some(instance) = tf.clone_with_arguments(&args) {
            out.push(instance);
        }
    }
    if out.is_empty() {
        vec![tf.clone()]
    } else {
        out
    }
}

thread_local! {
    /// Per-thread cache of the registered-web-font manager, refreshed when the registry's
    /// generation advances. `None` means no web fonts are registered.
    static WEB_FONT_MGR: RefCell<(u64, Option<FontMgr>)> = const { RefCell::new((0, None)) };
}

/// A `FontMgr` containing the registered web fonts for this thread, or `None` if there are
/// none. Cheap to call repeatedly: rebuilt only when a new font is registered.
pub(crate) fn web_font_mgr() -> Option<FontMgr> {
    WEB_FONT_MGR.with(|cell| {
        let mut cell = cell.borrow_mut();
        let gen = web_font_generation();
        if cell.0 != gen {
            cell.1 = build_web_font_mgr();
            cell.0 = gen;
        }
        cell.1.clone()
    })
}

thread_local! {
    static FC: RefCell<(u64, FontCollection)> = RefCell::new((u64::MAX, base_font_collection()));
}

fn base_font_collection() -> FontCollection {
    let mut fc = FontCollection::new();
    fc.set_default_font_manager(FontMgr::new(), None);
    fc
}

/// Run `f` with this thread's `FontCollection`, after refreshing its asset font manager
/// (the registered web fonts) if new fonts have been registered since the last call.
pub(crate) fn with_font_collection<R>(f: impl FnOnce(&FontCollection) -> R) -> R {
    FC.with(|cell| {
        let mut cell = cell.borrow_mut();
        let gen = web_font_generation();
        if cell.0 != gen {
            cell.1.set_asset_font_manager(web_font_mgr());
            cell.0 = gen;
        }
        f(&cell.1)
    })
}

/// Split a CSS `font-family` value (`"Source Serif 4", Georgia, serif`) into its individual
/// family names, trimmed and unquoted, in priority order. The font system tries each in turn
/// so the CSS fallback chain (including the generic `serif`/`sans-serif`/`monospace`) is
/// honoured instead of only the first name being attempted.
pub(crate) fn split_font_families(families: &str) -> Vec<String> {
    families
        .split(',')
        .map(|f| f.trim().trim_matches(['"', '\'']).trim().to_string())
        .filter(|f| !f.is_empty())
        .collect()
}

/// CSS generic families that fontconfig/Skia resolve directly to a concrete family; always kept.
fn is_real_generic(name: &str) -> bool {
    ["serif", "sans-serif", "monospace", "cursive", "fantasy", "emoji"]
        .iter()
        .any(|generic| name.eq_ignore_ascii_case(generic))
}

/// Newer CSS generic keywords that fontconfig usually does *not* map. We drop them so resolution
/// falls through to the real generic (`monospace`, `sans-serif`, …) that CSS font stacks
/// conventionally end with, instead of letting the platform default masquerade as this family.
fn is_pseudo_generic(name: &str) -> bool {
    [
        "system-ui",
        "ui-serif",
        "ui-sans-serif",
        "ui-monospace",
        "ui-rounded",
        "math",
        "fangsong",
    ]
    .iter()
    .any(|generic| name.eq_ignore_ascii_case(generic))
}

thread_local! {
    /// Cache of a CSS family list → the pruned list actually handed to Skia. Keyed alongside the
    /// web-font generation so newly-registered `@font-face` fonts re-resolve. Family lists repeat
    /// across nearly every text node, so this avoids re-probing the font manager each measure/draw.
    static RESOLVED_FAMILIES: RefCell<(u64, HashMap<String, Vec<String>>)> =
        RefCell::new((u64::MAX, HashMap::new()));
}

/// Prune a CSS `font-family` list to the entries Skia should actually try, in order.
///
/// Skia's `FontCollection` walks the list and, on Linux, fontconfig returns *some* face for *every*
/// name — even an unknown one like `ui-monospace` — so an unavailable leading family silently
/// captures the platform default and the real generic (`monospace`) at the end of the chain is never
/// reached.
///
/// We keep an entry when it's a real generic, or when it resolves to a *genuine* face: either an
/// exact name match, or a fontconfig **alias** to a family other than the bare default fallback
/// (e.g. `Arial` → Liberation Sans, which is what Firefox uses). We drop the newer pseudo-generics
/// (`ui-*`, `system-ui`) and any name that only yields the default fallback, so the stack's trailing
/// real generic decides instead of the platform default impersonating an unavailable family. If
/// nothing survives, fall back to the original list so text still draws. Applied to both measure and
/// draw so they stay on the same faces.
pub(crate) fn resolve_family_list(families: &str) -> Vec<String> {
    RESOLVED_FAMILIES.with(|cell| {
        let mut cell = cell.borrow_mut();
        let gen = web_font_generation();
        if cell.0 != gen {
            cell.1.clear();
            cell.0 = gen;
        }
        if let Some(v) = cell.1.get(families) {
            return v.clone();
        }

        let fm = FontMgr::new();
        let web = web_font_mgr();
        let normal = FontStyle::normal();

        // The face fontconfig hands back for a name it doesn't actually have. A name that resolves
        // to anything *else* is a real family or a real alias; a name that only yields this is an
        // unavailable family we should drop.
        let default_fallback = fm
            .match_family_style("__gosub_nonexistent_family__", normal)
            .map(|tf| tf.family_name());

        let resolves_to_real = |name: &str| -> bool {
            if let Some(tf) = web.as_ref().and_then(|w| w.match_family_style(name, normal)) {
                if tf.family_name().eq_ignore_ascii_case(name) {
                    return true;
                }
            }
            match fm.match_family_style(name, normal) {
                Some(tf) => {
                    let fam = tf.family_name();
                    fam.eq_ignore_ascii_case(name) || default_fallback.as_deref() != Some(fam.as_str())
                }
                None => false,
            }
        };

        let mut out = Vec::new();
        for name in split_font_families(families) {
            if is_pseudo_generic(&name) {
                continue;
            }
            if is_real_generic(&name) {
                // Replace the generic with the concrete family fontconfig picks for it
                // (what `fc-match` and Firefox use). Skia's textlayout resolves families via
                // `matchFamily()`, whose fontconfig style-set is ordered differently from
                // `matchFamilyStyle()` — it hands back e.g. FreeMono for `monospace` and
                // FreeSerif for `serif` instead of DejaVu Sans Mono / Noto Serif. Concrete
                // names round-trip through textlayout unchanged, so resolve the generic here.
                match fm.match_family_style(&name, normal) {
                    Some(tf) => out.push(tf.family_name()),
                    None => out.push(name),
                }
            } else if resolves_to_real(&name) {
                out.push(name);
            }
        }
        if out.is_empty() {
            out = split_font_families(families);
        }

        cell.1.insert(families.to_string(), out.clone());
        out
    })
}

/// Cached font-file bytes + collection index for a typeface; `None` when Skia can't hand them back.
type CachedFontData = Option<(Arc<Vec<u8>>, u32)>;

thread_local! {
    /// Per-thread cache of typeface file bytes: `to_font_data` copies the whole font file, and
    /// shaping asks once per glyph run. `None` results are cached too, so faces whose bytes Skia
    /// can't hand back aren't retried on every run.
    static TYPEFACE_BLOBS: RefCell<HashMap<skia_safe::typeface::TypefaceId, CachedFontData>> =
        RefCell::new(HashMap::new());
}

/// Raw file bytes + collection index for a typeface, as a [`FontBlob`].
fn typeface_blob(typeface: &skia_safe::Typeface) -> Option<FontBlob> {
    TYPEFACE_BLOBS.with(|cell| {
        let mut map = cell.borrow_mut();
        let entry = map
            .entry(typeface.unique_id())
            .or_insert_with(|| typeface.to_font_data().map(|(data, index)| (Arc::new(data), index as u32)));
        entry.as_ref().map(|(data, index)| {
            let bytes: Arc<Vec<u8>> = Arc::clone(data);
            FontBlob::new(bytes, *index)
        })
    })
}

fn to_skia_slant(style: CssFontStyle) -> skia_safe::font_style::Slant {
    match style {
        CssFontStyle::Normal => skia_safe::font_style::Slant::Upright,
        CssFontStyle::Italic => skia_safe::font_style::Slant::Italic,
        CssFontStyle::Oblique => skia_safe::font_style::Slant::Oblique,
    }
}

/// Build and lay out the measurement/shaping paragraph for `text` in `style`.
///
/// This is the single source of truth for how a [`GosubTextStyle`] maps onto Skia's textlayout —
/// `measure` reads this paragraph's extents and `shape` exports its glyph runs, so the two can't
/// disagree.
fn build_style_paragraph(fc: &FontCollection, text: &str, style: &GosubTextStyle) -> Paragraph {
    let mut paragraph_style = ParagraphStyle::new();
    paragraph_style.set_text_align(match style.align {
        GosubTextAlign::Start => TextAlign::Start,
        GosubTextAlign::Center => TextAlign::Center,
        GosubTextAlign::End => TextAlign::End,
        GosubTextAlign::Justify => TextAlign::Justify,
    });
    let mut builder = ParagraphBuilder::new(&paragraph_style, fc.clone());

    let mut ts = TextStyle::new();
    ts.set_font_size(style.size);
    // Apply the CSS line-height (absolute px → multiple of font size) exactly as the draw
    // path (`build_paragraph`) does, so the measured box height matches what is painted.
    // Skipping this measured the font's natural ~1.2× box while draw rendered the CSS 1.7×,
    // overflowing the reserved box into the next element.
    if let Some(line_height) = style.line_height {
        if line_height > 0.0 && style.size > 0.0 {
            ts.set_height(line_height / style.size);
            ts.set_height_override(true);
        }
    }
    // Apply CSS letter-spacing (px) so the measured width matches the drawn width.
    if style.letter_spacing != 0.0 {
        ts.set_letter_spacing(style.letter_spacing);
    }
    // Pass the pruned family list so Skia's FontCollection reaches the real generic instead
    // of letting an unavailable leading family capture the platform default.
    ts.set_font_families(&resolve_family_list(&style.family));
    ts.set_font_style(FontStyle::new(
        skia_safe::font_style::Weight::from(style.weight.0 as i32),
        skia_safe::font_style::Width::NORMAL,
        to_skia_slant(style.style),
    ));
    builder.push_style(&ts);
    builder.add_text(text);

    let mut paragraph = builder.build();
    // `None` = no wrap; use a large finite advance (Skia dislikes INFINITY).
    paragraph.layout(style.max_width.unwrap_or(1.0e9));
    paragraph
}

/// A [`FontSystem`] backed by Skia's `skia_safe` text layout.
///
/// Measurement, shaping, and the Skia rasterizer's own drawing all go through the same
/// thread-local [`FontCollection`], so they can't disagree. `resolve`/`shape` export concrete
/// fonts (via `Typeface::to_font_data`) and positioned glyph runs in the neutral trait types, so
/// a [`ShapedText`]-painting backend can consume this font system like any other; the Skia
/// rasterizer itself still draws through textlayout natively.
#[derive(Debug, Default)]
pub struct SkiaFontSystem;

impl FontSystem for SkiaFontSystem {
    fn register_font(&mut self, data: Vec<u8>, family_override: Option<&str>) -> Result<(), FontError> {
        // Validate the bytes and derive the family name if none was supplied.
        let family = match family_override {
            Some(f) => f.to_string(),
            None => match FontMgr::new().new_from_data(&data, None) {
                Some(tf) => tf.family_name(),
                None => return Err(FontError::InvalidFont("could not decode font data".into())),
            },
        };
        if FontMgr::new().new_from_data(&data, None).is_none() {
            return Err(FontError::InvalidFont(format!("unsupported font data for '{family}'")));
        }
        let mut reg = registry().lock();
        reg.fonts.push((family, data));
        reg.generation += 1;
        Ok(())
    }

    fn resolve(&mut self, query: &FontQuery<'_>) -> Result<ResolvedFont, FontError> {
        let font_style = FontStyle::new(
            skia_safe::font_style::Weight::from(query.weight.0 as i32),
            width_from_css_percent((query.stretch.0 * 100.0).round() as i32),
            to_skia_slant(query.style),
        );

        let joined = query.families.join(", ");
        // Prune the list the same way measure/draw do, and end on the sans-serif generic so
        // resolution always has a last resort.
        let mut names = resolve_family_list(&joined);
        if !names.iter().any(|n| n.eq_ignore_ascii_case("sans-serif")) {
            names.push("sans-serif".to_string());
        }

        let web = web_font_mgr();
        let fm = FontMgr::new();
        for name in &names {
            let typeface = web
                .as_ref()
                .and_then(|w| w.match_family_style(name.as_str(), font_style))
                .or_else(|| fm.match_family_style(name.as_str(), font_style));
            if let Some(typeface) = typeface {
                if let Some(blob) = typeface_blob(&typeface) {
                    return Ok(ResolvedFont {
                        family: typeface.family_name(),
                        style: query.style,
                        weight: query.weight,
                        stretch: query.stretch,
                        blob,
                    });
                }
            }
        }
        Err(FontError::FontNotFound(joined))
    }

    fn shape(&mut self, text: &str, style: &GosubTextStyle) -> ShapedText {
        if text.is_empty() {
            return ShapedText::empty();
        }
        with_font_collection(|fc| {
            let mut paragraph = build_style_paragraph(fc, text, style);
            let width = paragraph.longest_line();
            let height = paragraph.height();
            let (ascent, line_height) = paragraph
                .get_line_metrics()
                .first()
                .map(|m| (m.baseline as f32, m.height as f32))
                .unwrap_or((0.0, 0.0));

            let mut runs: Vec<ShapedRun> = Vec::new();
            paragraph.visit(|_, info| {
                let Some(info) = info else { return };
                let font = info.font();
                let typeface = font.typeface();
                let Some(blob) = typeface_blob(&typeface) else { return };
                let origin = info.origin();
                let glyphs: Vec<ShapedGlyph> = info
                    .glyphs()
                    .iter()
                    .zip(info.positions())
                    .map(|(glyph, pos)| ShapedGlyph {
                        id: u32::from(*glyph),
                        x: origin.x + pos.x,
                        y: origin.y + pos.y,
                    })
                    .collect();
                if glyphs.is_empty() {
                    return;
                }
                // Skia metrics are already positive-down; missing table entries fall back to
                // the common em-relative conventions.
                let (_, fm) = font.metrics();
                let size = font.size();
                let metrics = RunMetrics {
                    underline_offset: fm.underline_position().unwrap_or(size * 0.1),
                    underline_size: fm.underline_thickness().unwrap_or(size / 14.0),
                    strikethrough_offset: fm.strikeout_position().unwrap_or(-size * 0.25),
                    strikethrough_size: fm.strikeout_thickness().unwrap_or(size / 14.0),
                };
                runs.push(ShapedRun {
                    font: ResolvedFont {
                        family: typeface.family_name(),
                        style: style.style,
                        weight: style.weight,
                        stretch: style.stretch,
                        blob,
                    },
                    font_size: size,
                    x: origin.x,
                    baseline: origin.y,
                    width: info.advance_x(),
                    metrics,
                    glyphs,
                });
            });

            ShapedText {
                runs,
                width,
                height,
                line_height,
                ascent,
            }
        })
    }

    fn measure(&mut self, text: &str, style: &GosubTextStyle) -> (f32, f32) {
        if text.is_empty() {
            return (0.0, 0.0);
        }
        with_font_collection(|fc| {
            let paragraph = build_style_paragraph(fc, text, style);
            (paragraph.longest_line(), paragraph.height())
        })
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Map a CSS `font-stretch` percentage (normal = 100) onto Skia's 1–9 width classes
/// (normal = 5), using the CSS-defined keyword percentages as bucket centres. The previous
/// `width / 100` mapping sent the default 100% to width class 1 (ultra-condensed), which
/// disagreed with the measure path's `Width::NORMAL` and could select a condensed face.
fn width_from_css_percent(pct: i32) -> skia_safe::font_style::Width {
    let class = match pct {
        ..=56 => 1,     // ultra-condensed (50%)
        57..=68 => 2,   // extra-condensed (62.5%)
        69..=81 => 3,   // condensed (75%)
        82..=93 => 4,   // semi-condensed (87.5%)
        94..=106 => 5,  // normal (100%)
        107..=118 => 6, // semi-expanded (112.5%)
        119..=137 => 7, // expanded (125%)
        138..=174 => 8, // extra-expanded (150%)
        _ => 9,         // ultra-expanded (200%)
    };
    skia_safe::font_style::Width::from(class)
}

/// Build and lay out a Skia `Paragraph` for `text`, drawn with `paint`, wrapped/aligned within
/// `layout_width`. This is the single text engine for the Skia backend: the same `textlayout`
/// machinery used by [`SkiaFontSystem::measure`], so draw metrics match the layout metrics. It
/// honours the CSS features carried on [`FontInfo`] — text alignment, absolute line-height,
/// `underline`/`line-through`, weight/width/slant — which the previous hand-rolled `draw_str`
/// path could not. The caller paints the returned paragraph at the text box's top-left.
#[cfg(not(feature = "text_glyphs"))]
pub(crate) fn build_paragraph(text: &str, font_info: &FontInfo, paint: &Paint, layout_width: f32) -> Paragraph {
    let mut paragraph_style = ParagraphStyle::new();
    paragraph_style.set_text_align(match font_info.alignment {
        FontAlignment::Start => TextAlign::Start,
        FontAlignment::Center => TextAlign::Center,
        FontAlignment::End => TextAlign::End,
        FontAlignment::Justify => TextAlign::Justify,
    });
    paragraph_style.set_text_direction(TextDirection::LTR);

    let mut builder = ParagraphBuilder::new(&paragraph_style, with_font_collection(|fc| fc.clone()));

    let font_size = font_info.size as f32;

    let mut ts = TextStyle::new();
    ts.set_foreground_paint(paint);
    ts.set_font_size(font_size);
    // `line_height` is an absolute CSS px value; Skia expects a multiple of the font size, applied
    // only when height-override is on. Skip it for a non-positive size to avoid a div-by-zero.
    if font_info.line_height > 0.0 && font_size > 0.0 {
        ts.set_height(font_info.line_height as f32 / font_size);
        ts.set_height_override(true);
    }
    // CSS letter-spacing (px). Matches the value applied during measurement so widths agree.
    if font_info.letter_spacing != 0.0 {
        ts.set_letter_spacing(font_info.letter_spacing as f32);
    }
    ts.set_font_families(&resolve_family_list(&font_info.family));
    ts.set_font_style(FontStyle::new(
        skia_safe::font_style::Weight::from(font_info.weight),
        width_from_css_percent(font_info.width),
        if font_info.slant > 0 {
            skia_safe::font_style::Slant::Italic
        } else {
            skia_safe::font_style::Slant::Upright
        },
    ));

    let mut decoration = TextDecoration::NO_DECORATION;
    if font_info.underline {
        decoration |= TextDecoration::UNDERLINE;
    }
    if font_info.line_through {
        decoration |= TextDecoration::LINE_THROUGH;
    }
    if decoration != TextDecoration::NO_DECORATION {
        ts.set_decoration_type(decoration);
        ts.set_decoration_color(paint.color());
    }

    builder.push_style(&ts);
    builder.add_text(text);

    let mut paragraph = builder.build();
    paragraph.layout(layout_width);
    paragraph
}

#[allow(dead_code)]
fn to_slant(slant: i32) -> skia_safe::font_style::Slant {
    match slant {
        0 => skia_safe::font_style::Slant::Upright,
        1 => skia_safe::font_style::Slant::Italic,
        2 => skia_safe::font_style::Slant::Oblique,
        _ => skia_safe::font_style::Slant::Upright,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// End-to-end resolve + shape through Skia: the resolved font must carry its file bytes,
    /// shaping must produce glyph runs, and the shape bounding box must agree with `measure`
    /// (both read the same textlayout paragraph).
    #[test]
    fn resolves_and_shapes_via_skia() {
        let mut fs = SkiaFontSystem;

        let query = FontQuery::new(&["sans-serif"]);
        let resolved = fs.resolve(&query).expect("sans-serif must resolve");
        assert!(!resolved.blob.as_u8().is_empty(), "resolved font must carry file bytes");

        let style = GosubTextStyle::new("sans-serif", 16.0);
        let shaped = fs.shape("Hello", &style);
        assert!(!shaped.runs.is_empty(), "expected at least one glyph run");
        let glyph_count: usize = shaped.runs.iter().map(|r| r.glyphs.len()).sum();
        assert!(glyph_count >= 4, "expected >= 4 glyphs for \"Hello\", got {glyph_count}");
        assert!(shaped.ascent > 0.0 && shaped.ascent <= shaped.height);

        let (w, h) = fs.measure("Hello", &style);
        assert!(
            (w - shaped.width).abs() < 0.01 && (h - shaped.height).abs() < 0.01,
            "measure ({w} x {h}) must agree with shape ({} x {})",
            shaped.width,
            shaped.height
        );
    }

    #[test]
    fn splits_and_trims_family_list() {
        assert_eq!(
            split_font_families("\"Source Serif 4\", Georgia, serif"),
            vec!["Source Serif 4", "Georgia", "serif"]
        );
        assert_eq!(split_font_families("Arial"), vec!["Arial"]);
        assert_eq!(
            split_font_families(" 'My Font' , , monospace "),
            vec!["My Font", "monospace"]
        );
    }

    #[test]
    fn recognises_generic_families() {
        assert!(is_real_generic("serif"));
        assert!(is_real_generic("Sans-Serif"));
        assert!(is_real_generic("monospace"));
        assert!(is_pseudo_generic("system-ui"));
        assert!(is_pseudo_generic("UI-Monospace"));
        assert!(!is_real_generic("Source Serif 4"));
        assert!(!is_pseudo_generic("Georgia"));
    }

    #[test]
    fn generic_families_resolve_to_concrete_faces() {
        // Skia textlayout's `matchFamily()` resolves bare generics differently from
        // `matchFamilyStyle()` (e.g. FreeMono instead of DejaVu Sans Mono for `monospace`),
        // so `resolve_family_list` must hand textlayout the concrete family name.
        for stack in ["monospace", "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace"] {
            let resolved = resolve_family_list(stack);
            assert_eq!(resolved.len(), 1, "stack '{stack}' resolved to {resolved:?}");
            let name = &resolved[0];
            // If the system has any monospace font, the generic must have been replaced by
            // the same concrete family `matchFamilyStyle` (fc-match) picks.
            if let Some(tf) = FontMgr::new().match_family_style("monospace", FontStyle::normal()) {
                assert_eq!(name, &tf.family_name(), "stack '{stack}'");
            } else {
                assert_eq!(name, "monospace");
            }
        }
    }

    #[test]
    fn css_stretch_percent_maps_to_skia_width_classes() {
        assert_eq!(width_from_css_percent(100), skia_safe::font_style::Width::NORMAL);
        assert_eq!(
            width_from_css_percent(50),
            skia_safe::font_style::Width::ULTRA_CONDENSED
        );
        assert_eq!(width_from_css_percent(75), skia_safe::font_style::Width::CONDENSED);
        assert_eq!(width_from_css_percent(125), skia_safe::font_style::Width::EXPANDED);
        assert_eq!(
            width_from_css_percent(200),
            skia_safe::font_style::Width::ULTRA_EXPANDED
        );
    }
}
