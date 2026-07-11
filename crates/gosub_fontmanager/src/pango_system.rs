//! `PangoFontSystem` — fontconfig lookup + Pango/HarfBuzz shaping (the Linux desktop stack).
//!
//! Lives here (rather than in the Cairo renderer crate) because a font system is
//! renderer-independent: it resolves, shapes, and measures; any glyph-painting backend can
//! consume its output.

use cow_utils::CowUtils;
use gosub_interface::font::{FontBlob, FontError, FontStyle};
use gosub_interface::font_system::{
    FontQuery, FontSystem, ResolvedFont, RunMetrics, ShapedGlyph, ShapedRun, ShapedText, TextAlign, TextStyle,
};
use gtk4::pango;
use gtk4::pango::Weight;
use gtk4::prelude::{FontExt, FontFamilyExt};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::ffi::c_int;
use std::sync::{Arc, OnceLock};

const DEFAULT_FONT_FAMILY: &str = "sans";

/// Serialises every direct fontconfig call in this module. Mutating the process-global config
/// (`FcConfigAppFontAddFile`/`FcConfigBuildFonts`) while another thread matches against it
/// (`FcFontMatch`) segfaults — fontconfig's documented thread safety does not cover concurrent
/// mutation of the current config.
static FONTCONFIG_LOCK: Mutex<()> = Mutex::new(());

/// Register an in-memory `@font-face` font so Pango (via fontconfig) can discover it.
///
/// The bytes are written to a uniquely-named file in the temp dir — intentionally left on
/// disk for the process lifetime, since fontconfig references it by path — and added to the
/// process-global fontconfig config with `FcConfigAppFontAddFile`. fontconfig reads the
/// font's own family name from its `name` table (so the family CSS asked for, e.g.
/// "Source Serif 4", is what becomes available); `family_override` is informational.
///
/// Because the font is added to the *process-global* config (not a per-thread Pango font
/// map), any Pango `FcFontMap` built afterwards on any thread sees it — provided it is
/// registered before that font map is first built (the engine registers web fonts right
/// after the document is set, before the first layout).
fn register_font_via_fontconfig(data: &[u8], family_override: Option<&str>) -> Result<(), FontError> {
    use fontconfig_sys::{FcConfigAppFontAddFile, FcConfigBuildFonts, FcConfigGetCurrent};
    use std::io::Write as _;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);

    let safe: String = family_override
        .unwrap_or("webfont")
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let mut path = std::env::temp_dir();
    path.push(format!("gosub-webfont-{}-{safe}-{n}.ttf", std::process::id()));
    {
        let mut file =
            std::fs::File::create(&path).map_err(|e| FontError::InvalidFont(format!("temp font file: {e}")))?;
        file.write_all(data)
            .map_err(|e| FontError::InvalidFont(format!("write font: {e}")))?;
    }

    let path_str = path
        .to_str()
        .ok_or_else(|| FontError::InvalidFont("non-UTF-8 font path".to_string()))?;
    let c_path = std::ffi::CString::new(path_str).map_err(|e| FontError::InvalidFont(format!("font path: {e}")))?;

    let _guard = FONTCONFIG_LOCK.lock();

    #[allow(unsafe_code)] // fontconfig has no safe Rust binding for app-font registration
    // SAFETY: `FcConfigGetCurrent` returns the process-global config (auto-initialised, not
    // null in practice — checked anyway). `FcConfigAppFontAddFile` reads the NUL-terminated
    // `c_path` (valid for the call, not retained by fontconfig) and copies what it needs;
    // `FcConfigBuildFonts` rebuilds the font set. No Rust aliasing/lifetime invariants apply.
    let added = unsafe {
        let config = FcConfigGetCurrent();
        if config.is_null() {
            return Err(FontError::InvalidFont("fontconfig not initialised".to_string()));
        }
        let added = FcConfigAppFontAddFile(config, c_path.as_ptr().cast::<u8>());
        if added != 0 {
            FcConfigBuildFonts(config);
        }
        added
    };

    if added == 0 {
        return Err(FontError::InvalidFont(format!(
            "FcConfigAppFontAddFile failed for {}",
            path.display()
        )));
    }

    log::debug!(
        "Registered web font '{}' via fontconfig ({})",
        family_override.unwrap_or("<unnamed>"),
        path.display()
    );
    Ok(())
}

// fontconfig font matching (the lookup half of `resolve`)

/// The face fontconfig selects for a query: its real family name plus where its bytes live.
struct FontconfigMatch {
    family: String,
    path: String,
    index: u32,
}

/// CSS font-weight (100–900) → fontconfig's own (non-linear) weight scale.
fn to_fc_weight(w: u16) -> c_int {
    use fontconfig_sys::constants as fc;
    match w {
        0..=149 => fc::FC_WEIGHT_THIN,
        150..=249 => fc::FC_WEIGHT_EXTRALIGHT,
        250..=324 => fc::FC_WEIGHT_LIGHT,
        325..=449 => fc::FC_WEIGHT_REGULAR,
        450..=549 => fc::FC_WEIGHT_MEDIUM,
        550..=649 => fc::FC_WEIGHT_DEMIBOLD,
        650..=749 => fc::FC_WEIGHT_BOLD,
        750..=849 => fc::FC_WEIGHT_EXTRABOLD,
        _ => fc::FC_WEIGHT_BLACK,
    }
}

fn to_fc_slant(s: FontStyle) -> c_int {
    use fontconfig_sys::constants as fc;
    match s {
        FontStyle::Normal => fc::FC_SLANT_ROMAN,
        FontStyle::Italic => fc::FC_SLANT_ITALIC,
        FontStyle::Oblique => fc::FC_SLANT_OBLIQUE,
    }
}

/// CSS font-stretch ratio (1.0 = normal) → fontconfig width percent (100 = normal).
fn to_fc_width(ratio: f32) -> c_int {
    (ratio * 100.0).round() as c_int
}

/// Match `families` (already mapped to fontconfig names, in priority order) against the
/// process-global fontconfig config — the same database Pango itself resolves from — and return
/// the winning face. fontconfig always matches *something* after `FcDefaultSubstitute`, so this
/// only errors when fontconfig is unavailable or the matched pattern lacks a file path.
fn fontconfig_match(families: &[&str], weight: c_int, slant: c_int, width: c_int) -> Result<FontconfigMatch, FontError> {
    use fontconfig_sys::constants::{FC_FAMILY, FC_FILE, FC_INDEX, FC_SLANT, FC_WEIGHT, FC_WIDTH};
    use fontconfig_sys::{
        FcConfigGetCurrent, FcConfigSubstitute, FcDefaultSubstitute, FcFontMatch, FcMatchPattern, FcPatternAddInteger,
        FcPatternAddString, FcPatternCreate, FcPatternDestroy, FcPatternGetInteger, FcPatternGetString, FcResultMatch,
    };

    let c_families: Vec<std::ffi::CString> = families
        .iter()
        .filter_map(|f| std::ffi::CString::new(*f).ok())
        .collect();

    let _guard = FONTCONFIG_LOCK.lock();

    #[allow(unsafe_code)] // fontconfig has no safe Rust binding for font matching
    // SAFETY: `FcConfigGetCurrent` returns the process-global config (checked for null). The
    // pattern is created, filled, matched, and destroyed within this scope; the strings read out
    // of the matched pattern point into it, so they are copied to owned `String`s *before*
    // `FcPatternDestroy(matched)`. All pointers passed in are valid for the duration of each call.
    unsafe {
        let config = FcConfigGetCurrent();
        if config.is_null() {
            return Err(FontError::FontNotFound("fontconfig not initialised".to_string()));
        }
        let pat = FcPatternCreate();
        if pat.is_null() {
            return Err(FontError::FontNotFound("FcPatternCreate failed".to_string()));
        }
        for fam in &c_families {
            FcPatternAddString(pat, FC_FAMILY.as_ptr(), fam.as_ptr().cast::<u8>());
        }
        FcPatternAddInteger(pat, FC_WEIGHT.as_ptr(), weight);
        FcPatternAddInteger(pat, FC_SLANT.as_ptr(), slant);
        FcPatternAddInteger(pat, FC_WIDTH.as_ptr(), width);
        FcConfigSubstitute(config, pat, FcMatchPattern);
        FcDefaultSubstitute(pat);

        let mut result = FcResultMatch;
        let matched = FcFontMatch(config, pat, &mut result);
        FcPatternDestroy(pat);
        if matched.is_null() || result != FcResultMatch {
            if !matched.is_null() {
                FcPatternDestroy(matched);
            }
            return Err(FontError::FontNotFound(families.join(", ")));
        }

        let mut file_ptr: *mut u8 = std::ptr::null_mut();
        let mut family_ptr: *mut u8 = std::ptr::null_mut();
        let mut index: c_int = 0;
        let file_ok = FcPatternGetString(matched, FC_FILE.as_ptr(), 0, &mut file_ptr) == FcResultMatch
            && !file_ptr.is_null();
        let family_ok = FcPatternGetString(matched, FC_FAMILY.as_ptr(), 0, &mut family_ptr) == FcResultMatch
            && !family_ptr.is_null();
        let _ = FcPatternGetInteger(matched, FC_INDEX.as_ptr(), 0, &mut index);

        let out = file_ok.then(|| FontconfigMatch {
            family: if family_ok {
                std::ffi::CStr::from_ptr(family_ptr.cast()).to_string_lossy().into_owned()
            } else {
                families.first().copied().unwrap_or(DEFAULT_FONT_FAMILY).to_string()
            },
            path: std::ffi::CStr::from_ptr(file_ptr.cast()).to_string_lossy().into_owned(),
            index: index.max(0) as u32,
        });
        FcPatternDestroy(matched);

        out.ok_or_else(|| FontError::FontNotFound(families.join(", ")))
    }
}

/// Map a CSS generic font-family keyword to the Pango/fontconfig alias that resolves it.
/// Returns `None` for concrete family names (which must be looked up in `list_families()`).
fn pango_generic_family(name: &str) -> Option<&'static str> {
    match name.cow_to_ascii_lowercase().as_ref() {
        "serif" | "ui-serif" => Some("serif"),
        "sans-serif" | "ui-sans-serif" => Some("sans"),
        "monospace" | "ui-monospace" => Some("monospace"),
        "cursive" => Some("cursive"),
        "fantasy" => Some("fantasy"),
        _ => None,
    }
}

// PangoFontSystem

/// Font-system state for the Cairo/Pango backend.
///
/// Holds the cached `system-ui` family name, which must be resolved from the
/// GTK main thread before any background rendering.  After [`init_from_gtk_thread`]
/// is called the struct is read-only and can be shared freely behind an [`Arc`].
///
/// Obtain a shared instance via [`get`] (which returns the process-wide singleton
/// initialised by [`init`]) or construct an independent instance with [`new`] and
/// call [`PangoFontSystem::init_from_gtk_thread`] yourself.
/// Font file bytes keyed by `(path, ttc index)`.
type BlobCache = HashMap<(String, u32), Arc<Vec<u8>>>;

pub struct PangoFontSystem {
    system_ui_font: Option<String>,
    /// Cached font file bytes. Shaping resolves a font per glyph run, and re-reading e.g.
    /// DejaVu Sans from disk for every text run would hurt; interior mutability keeps the
    /// read-only-after-init sharing contract of the struct intact.
    blob_cache: Mutex<BlobCache>,
}

impl std::fmt::Debug for PangoFontSystem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PangoFontSystem")
            .field("system_ui_font", &self.system_ui_font)
            .finish()
    }
}

impl PangoFontSystem {
    pub fn new() -> Self {
        Self {
            system_ui_font: None,
            blob_cache: Mutex::new(HashMap::new()),
        }
    }

    /// Resolve and cache the system-ui font via GSettings.
    ///
    /// **Must** be called from the GTK main thread before any background rendering
    /// begins.  Calling it a second time is a no-op.
    pub fn init_from_gtk_thread(&mut self) {
        if self.system_ui_font.is_none() {
            self.system_ui_font = get_system_ui_font_from_gsettings();
        }
    }

    /// Walk `families` (comma-separated CSS `font-family` value) and return the
    /// first family name that Pango knows about, falling back to `"sans"`.
    pub fn find_available_font(&self, families: &str, ctx: &pango::Context) -> String {
        let available_fonts: Vec<String> = ctx
            .list_families()
            .iter()
            .map(|f| f.name().cow_to_ascii_lowercase().into_owned())
            .collect();

        for font in families.split(',') {
            let font_name = font.trim().trim_matches(|c| c == '"' || c == '\'').to_string();

            if font_name.eq_ignore_ascii_case("system-ui") {
                if let Some(ref system_font) = self.system_ui_font {
                    return system_font.clone();
                }
                continue;
            }

            // Generic CSS families resolve through Pango/fontconfig aliases ("serif",
            // "sans", "monospace") and never appear in `list_families()`, so map them
            // explicitly. Without this the generic at the end of a list (e.g. the `serif`
            // in `"Source Serif 4", Georgia, serif`) is skipped and the heading wrongly
            // falls back to the default sans family.
            if let Some(generic) = pango_generic_family(&font_name) {
                return generic.to_string();
            }

            let normalized = font_name.cow_to_ascii_lowercase();
            if available_fonts.contains(&normalized.into_owned()) {
                return font_name;
            }
        }

        DEFAULT_FONT_FAMILY.to_string()
    }
}

impl Default for PangoFontSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl PangoFontSystem {
    /// Map a CSS family list onto the names fontconfig understands: `system-ui` becomes the
    /// GSettings-resolved desktop font (or is skipped when unknown), CSS generics become their
    /// fontconfig aliases, concrete names pass through. Never returns an empty list.
    fn fc_family_names<'a>(&'a self, families: &[&'a str]) -> Vec<&'a str> {
        let mut out = Vec::new();
        for name in families {
            if name.eq_ignore_ascii_case("system-ui") {
                if let Some(ref system_font) = self.system_ui_font {
                    out.push(system_font.as_str());
                }
                continue;
            }
            out.push(pango_generic_family(name).unwrap_or(name));
        }
        if out.is_empty() {
            out.push(DEFAULT_FONT_FAMILY);
        }
        out
    }

    /// Font file bytes for a fontconfig match, served from the cache when possible.
    fn blob_for_path(&self, path: &str, index: u32) -> Result<FontBlob, FontError> {
        use std::collections::hash_map::Entry;

        let mut cache = self.blob_cache.lock();
        let data = match cache.entry((path.to_string(), index)) {
            Entry::Occupied(e) => Arc::clone(e.get()),
            Entry::Vacant(e) => {
                let bytes =
                    std::fs::read(path).map_err(|err| FontError::InvalidFont(format!("read {path}: {err}")))?;
                Arc::clone(e.insert(Arc::new(bytes)))
            }
        };
        Ok(FontBlob::new(data, index))
    }

    /// Build a laid-out Pango layout for `text` in `style` on a throwaway 1×1 surface (no pixels
    /// are drawn). The shared front half of `measure` and `shape`, and the same font-description
    /// path as the rasterizer — so measuring, shaping, and painting all see identical fonts and
    /// line breaking.
    fn build_layout(&self, text: &str, style: &TextStyle) -> Option<pango::Layout> {
        use pangocairo::functions::{context_set_resolution, create_layout};

        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 1, 1).ok()?;
        let cr = cairo::Context::new(&surface).ok()?;
        let layout = create_layout(&cr);
        // 96 DPI matches the browser/CSS convention, same as the rasterizer.
        context_set_resolution(&layout.context(), 96.0);

        let family = self.find_available_font(&style.family, &layout.context());
        let mut font_desc = pango::FontDescription::new();
        font_desc.set_family(&family);
        // CSS px → pt (× 72/96), then to Pango units (× SCALE).
        font_desc
            .set_size((style.size as f64 * style.display_scale as f64 * pango::SCALE as f64 * (72.0 / 96.0)) as i32);
        font_desc.set_weight(to_pango_weight(style.weight.0 as usize));
        if style.style != FontStyle::Normal {
            font_desc.set_style(pango::Style::Italic);
        }
        layout.set_font_description(Some(&font_desc));
        layout.set_text(text);
        layout.set_wrap(pango::WrapMode::Word);
        match style.align {
            TextAlign::Start => {} // pango's default (left for LTR)
            TextAlign::Center => layout.set_alignment(pango::Alignment::Center),
            TextAlign::End => layout.set_alignment(pango::Alignment::Right),
            TextAlign::Justify => layout.set_justify(true),
        }
        match style.max_width {
            Some(w) => {
                // Pango width is in Pango units (CSS px × display_scale × SCALE). Compute in
                // f64 and guard against i32 overflow: a very large / unbounded max_width is
                // treated as "no wrap limit" (-1), which is what an effectively-infinite
                // width means anyway. Without this, large widths panic in debug (overflow)
                // and silently wrap in release.
                let units = w as f64 * style.display_scale as f64 * pango::SCALE as f64;
                layout.set_width(if units >= i32::MAX as f64 { -1 } else { units as i32 });
            }
            None => layout.set_width(-1),
        }

        Some(layout)
    }

    /// Measure `text` by reading the pixel size of its laid-out Pango layout.
    fn measure_inner(&self, text: &str, style: &TextStyle) -> Option<(f32, f32)> {
        let layout = self.build_layout(text, style)?;
        let (w, h) = layout.pixel_size();
        Some((w as f32, h as f32))
    }

    /// Walk a laid-out Pango layout and export its glyph runs in the neutral [`ShapedText`] form.
    ///
    /// Glyph IDs are FreeType glyph indices into the run's font file; positions are pixels with
    /// `y` on the baseline, per the [`ShapedGlyph`] contract. Each run's font is the one Pango
    /// actually chose (mid-string fallback included), routed back through fontconfig to obtain
    /// its bytes — same database, so the description round-trip lands on the same file.
    fn runs_from_layout(&mut self, layout: &pango::Layout, style: &TextStyle) -> ShapedText {
        let scale = pango::SCALE as f32;
        let (px_w, px_h) = layout.pixel_size();
        let ascent = layout.baseline() as f32 / scale;
        let line_count = layout.line_count().max(1) as f32;

        let mut runs: Vec<ShapedRun> = Vec::new();
        let mut iter = layout.iter();
        loop {
            if let Some(run) = iter.run_readonly() {
                let baseline = iter.baseline() as f32 / scale;
                let (_, logical) = iter.run_extents();
                let run_x = logical.x() as f32 / scale;

                let glyph_string = run.glyph_string();
                let infos = glyph_string.glyph_info();
                let mut glyphs = Vec::with_capacity(infos.len());
                let mut pen_x = 0.0f32;
                for info in infos {
                    let geometry = info.geometry();
                    glyphs.push(ShapedGlyph {
                        id: info.glyph(),
                        x: run_x + pen_x + geometry.x_offset() as f32 / scale,
                        y: baseline + geometry.y_offset() as f32 / scale,
                    });
                    pen_x += geometry.width() as f32 / scale;
                }

                if !glyphs.is_empty() {
                    let pango_font = run.item().analysis().font();
                    // Pango metrics are y-up (underline below the baseline is negative); our
                    // convention is positive-down, so the positions flip sign.
                    let fm = pango_font.metrics(None);
                    let metrics = RunMetrics {
                        underline_offset: -fm.underline_position() as f32 / scale,
                        underline_size: fm.underline_thickness() as f32 / scale,
                        strikethrough_offset: -fm.strikethrough_position() as f32 / scale,
                        strikethrough_size: fm.strikethrough_thickness() as f32 / scale,
                    };
                    let description = pango_font.describe();
                    let family = description
                        .family()
                        .map(|f| f.to_string())
                        .unwrap_or_else(|| style.family.clone());
                    let families = [family.as_str()];
                    let query = FontQuery {
                        families: &families,
                        style: style.style,
                        weight: style.weight,
                        stretch: style.stretch,
                    };
                    if let Ok(font) = self.resolve(&query) {
                        runs.push(ShapedRun {
                            font,
                            font_size: style.size,
                            x: run_x,
                            baseline,
                            width: pen_x,
                            metrics,
                            glyphs,
                        });
                    }
                }
            }
            if !iter.next_run() {
                break;
            }
        }

        ShapedText {
            runs,
            width: px_w as f32,
            height: px_h as f32,
            line_height: px_h as f32 / line_count,
            ascent,
        }
    }
}

/// Pango as a swappable [`FontSystem`].
///
/// Pango bundles the three jobs the trait names: fontconfig does the lookup (`resolve` queries it
/// directly — the same database Pango picks fonts from), Pango/HarfBuzz do the shaping (`shape`
/// exports the `PangoLayout` glyph runs in neutral form), and `measure` reads the same layout's
/// pixel size. The Cairo rasterizer still draws through Pango natively; the glyph runs exist so
/// any [`ShapedText`]-painting backend can consume this font system too.
///
/// Note: Pango uses its own natural line height (matching how the Cairo rasterizer draws), so
/// `TextStyle::line_height` is intentionally not applied during measurement or shaping.
impl FontSystem for PangoFontSystem {
    fn register_font(&mut self, data: Vec<u8>, family_override: Option<&str>) -> Result<(), FontError> {
        register_font_via_fontconfig(&data, family_override)
    }

    fn resolve(&mut self, query: &FontQuery<'_>) -> Result<ResolvedFont, FontError> {
        let names = self.fc_family_names(query.families);
        let matched = fontconfig_match(
            &names,
            to_fc_weight(query.weight.0),
            to_fc_slant(query.style),
            to_fc_width(query.stretch.0),
        )?;
        let blob = self.blob_for_path(&matched.path, matched.index)?;
        Ok(ResolvedFont {
            family: matched.family,
            style: query.style,
            weight: query.weight,
            stretch: query.stretch,
            blob,
        })
    }

    fn families(&mut self) -> Vec<String> {
        // A throwaway pangocairo context (same construction as `build_layout`) reads the
        // default font map — the fontconfig database, including web fonts registered before
        // the font map was first built.
        use pangocairo::functions::create_layout;
        let Ok(surface) = cairo::ImageSurface::create(cairo::Format::ARgb32, 1, 1) else {
            return Vec::new();
        };
        let Ok(cr) = cairo::Context::new(&surface) else {
            return Vec::new();
        };
        let layout = create_layout(&cr);
        let mut out: Vec<String> = layout
            .context()
            .list_families()
            .iter()
            .map(|f| f.name().to_string())
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    fn shape(&mut self, text: &str, style: &TextStyle) -> ShapedText {
        if text.is_empty() {
            return ShapedText::empty();
        }
        let Some(layout) = self.build_layout(text, style) else {
            return ShapedText::empty();
        };
        self.runs_from_layout(&layout, style)
    }

    fn measure(&mut self, text: &str, style: &TextStyle) -> (f32, f32) {
        if text.is_empty() {
            return (0.0, 0.0);
        }
        self.measure_inner(text, style)
            .unwrap_or_else(|| (text.chars().count() as f32 * style.size * 0.5, style.size * 1.2))
    }
}

// Process-wide singleton (required because GTK init must happen on main thread)

/// Process-wide `PangoFontSystem` singleton.
///
/// Set by [`init`]; read by [`get`].  The `OnceLock` is intentional — GTK's
/// font resolution is tied to the main thread and the result is immutable once
/// resolved, so a static `Arc` is the correct primitive here.
static PANGO_FONT_SYSTEM: OnceLock<Arc<PangoFontSystem>> = OnceLock::new();

/// Initialise the singleton from the GTK main thread.
///
/// Called once at startup (e.g. from `crate::init_gtk_resources()`).
/// Subsequent calls are silently ignored.
pub fn init() {
    PANGO_FONT_SYSTEM.get_or_init(|| {
        let mut fs = PangoFontSystem::new();
        fs.init_from_gtk_thread();
        Arc::new(fs)
    });
}

/// Return the process-wide font system, initialising it without GTK-thread
/// resolution if it hasn't been set yet (fallback path for headless tests).
pub fn get() -> Arc<PangoFontSystem> {
    Arc::clone(PANGO_FONT_SYSTEM.get_or_init(|| Arc::new(PangoFontSystem::new())))
}

// Weight mapping

pub fn to_pango_weight(weight: usize) -> Weight {
    match weight {
        0..=149 => Weight::Thin,         // 100
        150..=249 => Weight::Ultralight, // 200
        250..=324 => Weight::Light,      // 300
        325..=374 => Weight::Semilight,  // 350
        375..=449 => Weight::Normal,     // 400
        450..=549 => Weight::Medium,     // 500
        550..=649 => Weight::Semibold,   // 600
        650..=749 => Weight::Bold,       // 700
        750..=849 => Weight::Ultrabold,  // 800
        _ => Weight::Heavy,              // 900
    }
}

// Backward-compat shim used by crate::init_gtk_resources

/// Deprecated entry point kept for ABI compatibility.
/// Prefer calling [`init`] directly.
#[deprecated(since = "0.1.0", note = "Use `gosub_renderer_cairo::font::pango::init()` instead")]
pub fn init_system_ui_font() {
    init();
}

// Internal helpers

fn get_system_ui_font_from_gsettings() -> Option<String> {
    use gtk4::gio::{Settings, SettingsSchemaSource};
    use gtk4::prelude::SettingsExt;

    let schema_source = SettingsSchemaSource::default()?;
    let schema = schema_source.lookup("org.gnome.desktop.interface", true)?;

    if schema.has_key("font-name") {
        let settings = Settings::new("org.gnome.desktop.interface");
        let font_name: String = settings.string("font-name").into();
        return Some(
            font_name
                .split_whitespace()
                .next()
                .unwrap_or(DEFAULT_FONT_FAMILY)
                .to_string(),
        );
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Exercises the full registration path end-to-end: temp-file write plus the fontconfig
    /// FFI (`FcConfigGetCurrent` / `FcConfigAppFontAddFile` / `FcConfigBuildFonts`). Uses the
    /// always-available bundled Roboto bytes. A misdeclared FFI signature would crash here;
    /// a clean `Ok(())` confirms the unsafe boundary is sound at runtime.
    #[test]
    fn registers_font_via_fontconfig() {
        let res = register_font_via_fontconfig(gosub_shared::ROBOTO_FONT, Some("Gosub Roboto Test"));
        assert!(res.is_ok(), "fontconfig registration failed: {res:?}");
    }

    /// `families()` reads the default Pango font map (fontconfig): non-empty on any machine
    /// with fonts, sorted, de-duplicated.
    #[test]
    fn families_lists_fontconfig_families_sorted() {
        let mut fs = PangoFontSystem::new();
        let families = fs.families();
        assert!(!families.is_empty(), "fontconfig families must be listed");
        assert!(families.windows(2).all(|w| w[0] < w[1]), "must be sorted and deduped");
    }

    /// End-to-end resolve + shape through fontconfig and Pango: the resolved font must carry its
    /// file bytes, shaping must produce glyph runs, and the shape bounding box must agree with
    /// `measure` (both read the same `PangoLayout`).
    #[test]
    fn resolves_and_shapes_via_fontconfig() {
        let mut fs = PangoFontSystem::new();

        let query = FontQuery::new(&["sans-serif"]);
        let resolved = fs.resolve(&query).expect("sans-serif must resolve via fontconfig");
        assert!(!resolved.blob.as_u8().is_empty(), "resolved font must carry file bytes");

        let style = TextStyle::new("sans-serif", 16.0);
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
}
