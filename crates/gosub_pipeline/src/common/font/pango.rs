use gtk4::gio::Settings;
use gtk4::pango;
use gtk4::pango::Weight;
use gtk4::prelude::{FontFamilyExt, SettingsExt};

const DEFAULT_FONT_FAMILY: &str = "sans";

pub fn find_available_font(families: &str, ctx: &pango::Context) -> String {
    let available_fonts: Vec<String> = ctx
        .list_families()
        .iter()
        .map(|f| f.name().to_ascii_lowercase())
        .collect();

    for font in families.split(',') {
        let font_name = font.trim().replace('"', "");

        // system-ui is a special keyword resolved via the desktop environment
        if font_name.eq_ignore_ascii_case("system-ui") {
            let system_font = get_system_ui_font();
            if available_fonts.contains(&system_font.to_ascii_lowercase()) {
                return system_font;
            }
            continue;
        }

        if available_fonts.contains(&font_name.to_ascii_lowercase()) {
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
        _ => Weight::__Unknown(w as i32),
    }
}

/// Returns the font as defined by the gnome settings. Other platforms like windows and osx will
/// deal with this differently.
fn get_system_ui_font() -> String {
    let settings = Settings::new("org.gnome.desktop.interface");
    settings.string("font-name").to_string()
}
