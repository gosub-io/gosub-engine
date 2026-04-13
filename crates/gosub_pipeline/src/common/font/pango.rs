use pangocairo::pango;
use pangocairo::pango::prelude::FontFamilyExt;
use pangocairo::pango::Weight;

const DEFAULT_FONT_FAMILY: &str = "sans";

pub fn find_available_font(families: &str, ctx: &pango::Context) -> String {
    let available_fonts: Vec<String> = ctx
        .list_families()
        .iter()
        .map(|f: &pango::FontFamily| f.name().to_ascii_lowercase())
        .collect();

    for font in families.split(',') {
        // system-ui is a special font handled by the OS; fall back to a known name.
        if font.trim() == "system-ui" {
            return "sans".into();
        }

        let font_name = font.trim().replace('"', "");
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
