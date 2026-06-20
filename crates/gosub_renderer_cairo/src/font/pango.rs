use cow_utils::CowUtils;
use gtk4::pango;
use gtk4::pango::Weight;
use gtk4::prelude::FontFamilyExt;
use std::sync::{Arc, OnceLock};

const DEFAULT_FONT_FAMILY: &str = "sans";

// ---------------------------------------------------------------------------
// PangoFontSystem
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Process-wide singleton (required because GTK init must happen on main thread)
// ---------------------------------------------------------------------------

/// Process-wide `PangoFontSystem` singleton.
///
/// Set by [`init`]; read by [`get`].  The `OnceLock` is intentional — GTK's
/// font resolution is tied to the main thread and the result is immutable once
/// resolved, so a static `Arc` is the correct primitive here.
static PANGO_FONT_SYSTEM: OnceLock<Arc<PangoFontSystem>> = OnceLock::new();

/// Initialise the singleton from the GTK main thread.
///
/// Called once at startup (e.g. from `gosub_engine::init_gtk_resources()`).
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

// ---------------------------------------------------------------------------
// Weight mapping
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Backward-compat shim used by gosub_engine::init_gtk_resources
// ---------------------------------------------------------------------------

/// Deprecated entry point kept for ABI compatibility.
/// Prefer calling [`init`] directly.
#[deprecated(since = "0.1.0", note = "Use `gosub_renderer_cairo::font::pango::init()` instead")]
pub fn init_system_ui_font() {
    init();
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

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
