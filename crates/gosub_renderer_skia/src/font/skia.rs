use gosub_interface::font::{FontError, FontStyle as CssFontStyle};
use gosub_interface::font_system::{FontSystem, TextStyle as GosubTextStyle};
use gosub_render_pipeline::common::font::{FontAlignment, FontInfo};
use parking_lot::Mutex;
use skia_safe::textlayout::{
    FontCollection, Paragraph, ParagraphBuilder, ParagraphStyle, TextAlign, TextDecoration, TextDirection, TextStyle,
    TypefaceFontProvider,
};
use skia_safe::{FontMgr, FontStyle, Paint};
use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::OnceLock;

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
fn build_web_font_mgr() -> Option<FontMgr> {
    let reg = registry().lock();
    if reg.fonts.is_empty() {
        return None;
    }
    let fm = FontMgr::new();
    let mut provider = TypefaceFontProvider::new();
    let mut any = false;
    for (family, bytes) in reg.fonts.iter() {
        if let Some(tf) = fm.new_from_data(bytes, None) {
            provider.register_typeface(tf, Some(family.as_str()));
            any = true;
        }
    }
    any.then(|| provider.into())
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
            if is_real_generic(&name) || resolves_to_real(&name) {
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

/// A [`FontSystem`] backed by Skia's `skia_safe` text layout.
///
/// Opaque / backend-coupled like Pango: it measures (and the Skia rasterizer draws) through the
/// same thread-local [`FontCollection`], so measurement matches what Skia paints. Lives in the
/// Skia backend crate for the same reason Pango lives in the Cairo crate — it's tied to its
/// renderer and isn't a portable glyph emitter.
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

    fn measure(&mut self, text: &str, style: &GosubTextStyle) -> (f32, f32) {
        if text.is_empty() {
            return (0.0, 0.0);
        }
        with_font_collection(|fc| {
            let paragraph_style = ParagraphStyle::new();
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
            // Pass the pruned family list so Skia's FontCollection reaches the real generic instead
            // of letting an unavailable leading family capture the platform default.
            ts.set_font_families(&resolve_family_list(&style.family));
            ts.set_font_style(FontStyle::new(
                skia_safe::font_style::Weight::from(style.weight.0 as i32),
                skia_safe::font_style::Width::NORMAL,
                match style.style {
                    CssFontStyle::Normal => skia_safe::font_style::Slant::Upright,
                    CssFontStyle::Italic => skia_safe::font_style::Slant::Italic,
                    CssFontStyle::Oblique => skia_safe::font_style::Slant::Oblique,
                },
            ));
            builder.push_style(&ts);
            builder.add_text(text);

            let mut paragraph = builder.build();
            // `None` = no wrap; use a large finite advance (Skia dislikes INFINITY).
            paragraph.layout(style.max_width.unwrap_or(1.0e9));
            (paragraph.longest_line(), paragraph.height())
        })
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Build and lay out a Skia `Paragraph` for `text`, drawn with `paint`, wrapped/aligned within
/// `layout_width`. This is the single text engine for the Skia backend: the same `textlayout`
/// machinery used by [`SkiaFontSystem::measure`], so draw metrics match the layout metrics. It
/// honours the CSS features carried on [`FontInfo`] — text alignment, absolute line-height,
/// `underline`/`line-through`, weight/width/slant — which the previous hand-rolled `draw_str`
/// path could not. The caller paints the returned paragraph at the text box's top-left.
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
    ts.set_font_families(&resolve_family_list(&font_info.family));
    ts.set_font_style(FontStyle::new(
        skia_safe::font_style::Weight::from(font_info.weight),
        skia_safe::font_style::Width::from((font_info.width / 100).clamp(1, 9)),
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
}
