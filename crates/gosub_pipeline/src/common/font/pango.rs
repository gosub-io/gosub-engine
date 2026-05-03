use cow_utils::CowUtils;
use gtk4::gio::{Settings, SettingsSchemaSource};
use gtk4::pango;
use gtk4::pango::Weight;
use gtk4::prelude::{FontFamilyExt, SettingsExt};

const DEFAULT_FONT_FAMILY: &str = "sans";

pub fn find_available_font(families: &str, ctx: &pango::Context) -> String {
    let available_fonts: Vec<String> = ctx
        .list_families()
        .iter()
        .map(|f| f.name().cow_to_ascii_lowercase().into_owned())
        .collect();

    for font in families.split(',') {
        let font_name = font.trim().trim_matches(|c| c == '"' || c == '\'').to_string();

        // system-ui is a special keyword resolved via the desktop environment
        if font_name.eq_ignore_ascii_case("system-ui") {
            let system_font = get_system_ui_font();
            if available_fonts.contains(&system_font.to_ascii_lowercase()) {
                return system_font;
            }
            continue;
        }

        if available_fonts.contains(&font_name.cow_to_ascii_lowercase().into_owned()) {
            return font_name;
        }
    }

    DEFAULT_FONT_FAMILY.to_string()
}

pub fn to_pango_weight(w: usize) -> Weight {
    match w {
        100 => Weight::Thin,
        200 => Weight::Ultralight,
        300 => Weight::Light,
        350 => Weight::Semilight,
        380 => Weight::Book,
        400 => Weight::Normal,
        500 => Weight::Medium,
        600 => Weight::Semibold,
        700 => Weight::Bold,
        800 => Weight::Ultrabold,
        900 => Weight::Heavy,
        1000 => Weight::Ultraheavy,
        _ => {
            // Weight::__Unknown is not a stable public API; map to the nearest standard weight.
            if w < 150 {
                Weight::Thin
            } else if w < 250 {
                Weight::Ultralight
            } else if w < 325 {
                Weight::Light
            } else if w < 365 {
                Weight::Semilight
            } else if w < 390 {
                Weight::Book
            } else if w < 450 {
                Weight::Normal
            } else if w < 550 {
                Weight::Medium
            } else if w < 650 {
                Weight::Semibold
            } else if w < 750 {
                Weight::Bold
            } else if w < 850 {
                Weight::Ultrabold
            } else if w < 950 {
                Weight::Heavy
            } else {
                Weight::Ultraheavy
            }
        }
    }
}

/// Returns the font family as defined by the gnome settings, with the size suffix stripped.
/// Falls back to DEFAULT_FONT_FAMILY on non-GNOME platforms where the schema is absent.
fn get_system_ui_font() -> String {
    // Settings::new() panics when the schema is not installed, so check first.
    let schema_available = SettingsSchemaSource::default()
        .and_then(|src| src.lookup("org.gnome.desktop.interface", true))
        .is_some();
    if !schema_available {
        return DEFAULT_FONT_FAMILY.to_string();
    }

    let settings = Settings::new("org.gnome.desktop.interface");
    let full_name = settings.string("font-name").to_string();
    // GSettings returns "Family Name <size>" — strip the trailing size token.
    match full_name.rsplit_once(' ') {
        Some((family, size_token)) if size_token.parse::<f32>().is_ok() => family.to_string(),
        _ => full_name,
    }
}
