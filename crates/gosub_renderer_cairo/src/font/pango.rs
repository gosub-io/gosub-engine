use cow_utils::CowUtils;
use gtk4::pango;
use gtk4::pango::Weight;
use gtk4::prelude::FontFamilyExt;
use std::sync::OnceLock;

const DEFAULT_FONT_FAMILY: &str = "sans";

/// Cached system-ui font family. Must be populated from the GTK main thread before any
/// background rendering begins — call `init_system_ui_font()` once at startup.
static SYSTEM_UI_FONT: OnceLock<Option<String>> = OnceLock::new();

/// Resolve and cache the system-ui font. Call once from the GTK main thread at startup.
pub fn init_system_ui_font() {
    SYSTEM_UI_FONT.get_or_init(get_system_ui_font_from_gsettings);
}

pub fn find_available_font(families: &str, ctx: &pango::Context) -> String {
    let available_fonts: Vec<String> = ctx
        .list_families()
        .iter()
        .map(|f| f.name().cow_to_ascii_lowercase().into_owned())
        .collect();

    for font in families.split(',') {
        let font_name = font.trim().trim_matches(|c| c == '"' || c == '\'').to_string();

        if font_name.eq_ignore_ascii_case("system-ui") {
            if let Some(Some(system_font)) = SYSTEM_UI_FONT.get() {
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

/// Read the GNOME system font from GSettings. Must be called from the GTK main thread.
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

pub fn to_pango_weight(weight: usize) -> Weight {
    match weight {
        0..=149 => Weight::Thin,         // 100
        150..=249 => Weight::Ultralight, // 200
        250..=324 => Weight::Light,      // 300
        325..=374 => Weight::Semilight,  // 350
        375..=449 => Weight::Normal,     // 400 — CSS "normal" weight lands here
        450..=549 => Weight::Medium,     // 500
        550..=649 => Weight::Semibold,   // 600
        650..=749 => Weight::Bold,       // 700
        750..=849 => Weight::Ultrabold,  // 800
        _ => Weight::Heavy,              // 900
    }
}
