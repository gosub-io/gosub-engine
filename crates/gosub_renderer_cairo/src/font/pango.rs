use cow_utils::CowUtils;
use gosub_interface::font::{FontError, FontStyle};
use gosub_interface::font_system::{FontSystem, TextStyle};
use gtk4::pango;
use gtk4::pango::Weight;
use gtk4::prelude::FontFamilyExt;
use std::any::Any;
use std::sync::{Arc, OnceLock};

const DEFAULT_FONT_FAMILY: &str = "sans";

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
pub struct PangoFontSystem {
    system_ui_font: Option<String>,
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
        Self { system_ui_font: None }
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
    /// Measure `text` by building a Pango layout on a throwaway 1×1 surface (no pixels are
    /// drawn — we only read `pixel_size()`). Reuses the same font-description path as the
    /// rasterizer so measurement matches what Cairo will actually paint.
    fn measure_inner(&self, text: &str, style: &TextStyle) -> Option<(f32, f32)> {
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
        font_desc.set_size((style.size as f64 * style.display_scale as f64 * pango::SCALE as f64 * (72.0 / 96.0)) as i32);
        font_desc.set_weight(to_pango_weight(style.weight.0 as usize));
        if style.style != FontStyle::Normal {
            font_desc.set_style(pango::Style::Italic);
        }
        layout.set_font_description(Some(&font_desc));
        layout.set_text(text);
        layout.set_wrap(pango::WrapMode::Word);
        match style.max_width {
            Some(w) => layout.set_width((w * style.display_scale) as i32 * pango::SCALE),
            None => layout.set_width(-1),
        }

        let (w, h) = layout.pixel_size();
        Some((w as f32, h as f32))
    }
}

/// Pango as a swappable [`FontSystem`].
///
/// Pango is an *opaque* engine: it shapes and draws from a family name via pango/cairo and never
/// exposes raw font bytes or neutral glyph runs. The slim trait fits it well — only `measure` and
/// `register_font` are meaningful; engine-native shaping/drawing stays in the Cairo rasterizer
/// (which draws through Pango directly), and there are no inherent `resolve`/`shape` methods here.
///
/// Note: Pango uses its own natural line height (matching how the Cairo rasterizer draws), so
/// `TextStyle::line_height` is intentionally not applied during measurement.
impl FontSystem for PangoFontSystem {
    fn register_font(&mut self, _data: Vec<u8>, _family_override: Option<&str>) -> Result<(), FontError> {
        // Pango discovers fonts via fontconfig; injecting @font-face bytes at runtime would
        // require writing a fontconfig configuration and is out of scope here.
        log::warn!("PangoFontSystem::register_font is unsupported; the font was ignored");
        Ok(())
    }

    fn measure(&mut self, text: &str, style: &TextStyle) -> (f32, f32) {
        if text.is_empty() {
            return (0.0, 0.0);
        }
        self.measure_inner(text, style)
            .unwrap_or_else(|| (text.chars().count() as f32 * style.size * 0.5, style.size * 1.2))
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
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
