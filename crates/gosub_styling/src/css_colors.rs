use lazy_static::lazy_static;
use regex::Regex;
use std::convert::From;

// Values for this table is taken from https://www.w3.org/TR/CSS21/propidx.html
// Probably not the complete list, but it will do for now

pub struct CssColorEntry {
    pub name: &'static str,
    pub value: &'static str,
}

pub struct RgbColor {
    /// Red component
    pub r: u8,
    /// Green component
    pub g: u8,
    /// Blue component
    pub b: u8,
    /// Alpha component (0 = transparent, 255 = solid)
    pub a: u8,
}

impl RgbColor {
    pub fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        RgbColor { r, g, b, a }
    }
}

impl Default for RgbColor {
    fn default() -> Self {
        RgbColor {
            r: 0,
            g: 0,
            b: 0,
            a: 255,
        }
    }
}

impl From<&str> for RgbColor {
    fn from(value: &str) -> Self {
        if value.is_empty() {
            return RgbColor::default();
        }
        if value.starts_with('#') {
            return parse_hex(value);
        }
        if value.starts_with("rgb(") {
            // Rgb function
        }
        if value.starts_with("rgba(") {
            // Rgba function
        }
        if value.starts_with("hsl(") {
            // HSL function
        }

        return get_hex_color_from_name(value).map_or(RgbColor::default(), parse_hex);
    }
}

fn get_hex_color_from_name(color_name: &str) -> Option<&str> {
    for entry in crate::css_colors::CSS_COLORNAMES.iter() {
        if entry.name == color_name {
            return Some(entry.value);
        }
    }
    None
}

fn parse_hex(value: &str) -> RgbColor {
    // 6 with RRGGBB or 8 hex digits RRGGBBAA
    if Regex::new(r"^#[0-9a-fA-F]{6,8}$").unwrap().is_match(value) {
        let r = i32::from_str_radix(&value[1..3], 16).unwrap();
        let g = i32::from_str_radix(&value[3..5], 16).unwrap();
        let b = i32::from_str_radix(&value[5..7], 16).unwrap();
        let mut a = 255;

        if value.len() == 9 {
            a = i32::from_str_radix(&value[7..9], 16).unwrap();
        }

        return RgbColor::new(r as u8, g as u8, b as u8, a as u8);
    }

    // 3 with RGB or 4 hex digits RGBA
    if Regex::new(r"^#[0-9a-fA-F]{3,4}$").unwrap().is_match(value) {
        let mut r = i32::from_str_radix(&value[1..2], 16).unwrap();
        r = r * 16 + r;
        let mut g = i32::from_str_radix(&value[2..3], 16).unwrap();
        g = g * 16 + g;
        let mut b = i32::from_str_radix(&value[3..4], 16).unwrap();
        b = b * 16 + b;
        let mut a = 255;
        if value.len() == 5 {
            a = i32::from_str_radix(&value[4..5], 16).unwrap();
            a = a * 16 + a;
        }

        return RgbColor::new(r as u8, g as u8, b as u8, a as u8);
    }

    RgbColor::default()
}

lazy_static! {
    pub static ref CSS_COLORNAMES: &'static [CssColorEntry] = &[
        CssColorEntry {
            name: "aliceblue",
            value: "#f0f8ff",
        },
        CssColorEntry {
            name: "antiquewhite",
            value: "#faebd7",
        },
        CssColorEntry {
            name: "aqua",
            value: "#00ffff",
        },
        CssColorEntry {
            name: "aquamarine",
            value: "#7fffd4",
        },
        CssColorEntry {
            name: "azure",
            value: "#f0ffff",
        },
        CssColorEntry {
            name: "beige",
            value: "#f5f5dc",
        },
        CssColorEntry {
            name: "bisque",
            value: "#ffe4c4",
        },
        CssColorEntry {
            name: "black",
            value: "#000000",
        },
        CssColorEntry {
            name: "blanchedalmond",
            value: "#ffebcd",
        },
        CssColorEntry {
            name: "blue",
            value: "#0000ff",
        },
        CssColorEntry {
            name: "blueviolet",
            value: "#8a2be2",
        },
        CssColorEntry {
            name: "brown",
            value: "#a52a2a",
        },
        CssColorEntry {
            name: "burlywood",
            value: "#deb887",
        },
        CssColorEntry {
            name: "cadetblue",
            value: "#5f9ea0",
        },
        CssColorEntry {
            name: "chartreuse",
            value: "#7fff00",
        },
        CssColorEntry {
            name: "chocolate",
            value: "#d2691e",
        },
        CssColorEntry {
            name: "coral",
            value: "#ff7f50",
        },
        CssColorEntry {
            name: "cornflowerblue",
            value: "#6495ed",
        },
        CssColorEntry {
            name: "cornsilk",
            value: "#fff8dc",
        },
        CssColorEntry {
            name: "crimson",
            value: "#dc143c",
        },
        CssColorEntry {
            name: "cyan",
            value: "#00ffff",
        },
        CssColorEntry {
            name: "darkblue",
            value: "#00008b",
        },
        CssColorEntry {
            name: "darkcyan",
            value: "#008b8b",
        },
        CssColorEntry {
            name: "darkgoldenrod",
            value: "#b8860b",
        },
        CssColorEntry {
            name: "darkgray",
            value: "#a9a9a9",
        },
        CssColorEntry {
            name: "darkgreen",
            value: "#006400",
        },
        CssColorEntry {
            name: "darkgrey",
            value: "#a9a9a9",
        },
        CssColorEntry {
            name: "darkkhaki",
            value: "#bdb76b",
        },
        CssColorEntry {
            name: "darkmagenta",
            value: "#8b008b",
        },
        CssColorEntry {
            name: "darkolivegreen",
            value: "#556b2f",
        },
        CssColorEntry {
            name: "darkorange",
            value: "#ff8c00",
        },
        CssColorEntry {
            name: "darkorchid",
            value: "#9932cc",
        },
        CssColorEntry {
            name: "darkred",
            value: "#8b0000",
        },
        CssColorEntry {
            name: "darksalmon",
            value: "#e9967a",
        },
        CssColorEntry {
            name: "darkseagreen",
            value: "#8fbc8f",
        },
        CssColorEntry {
            name: "darkslateblue",
            value: "#483d8b",
        },
        CssColorEntry {
            name: "darkslategray",
            value: "#2f4f4f",
        },
        CssColorEntry {
            name: "darkslategrey",
            value: "#2f4f4f",
        },
        CssColorEntry {
            name: "darkturquoise",
            value: "#00ced1",
        },
        CssColorEntry {
            name: "darkviolet",
            value: "#9400d3",
        },
        CssColorEntry {
            name: "deeppink",
            value: "#ff1493",
        },
        CssColorEntry {
            name: "deepskyblue",
            value: "#00bfff",
        },
        CssColorEntry {
            name: "dimgray",
            value: "#696969",
        },
        CssColorEntry {
            name: "dimgrey",
            value: "#696969",
        },
        CssColorEntry {
            name: "dodgerblue",
            value: "#1e90ff",
        },
        CssColorEntry {
            name: "firebrick",
            value: "#b22222",
        },
        CssColorEntry {
            name: "floralwhite",
            value: "#fffaf0",
        },
        CssColorEntry {
            name: "forestgreen",
            value: "#228b22",
        },
        CssColorEntry {
            name: "fuchsia",
            value: "#ff00ff",
        },
        CssColorEntry {
            name: "gainsboro",
            value: "#dcdcdc",
        },
        CssColorEntry {
            name: "ghostwhite",
            value: "#f8f8ff",
        },
        CssColorEntry {
            name: "gold",
            value: "#ffd700",
        },
        CssColorEntry {
            name: "goldenrod",
            value: "#daa520",
        },
        CssColorEntry {
            name: "gray",
            value: "#808080",
        },
        CssColorEntry {
            name: "green",
            value: "#008000",
        },
        CssColorEntry {
            name: "greenyellow",
            value: "#adff2f",
        },
        CssColorEntry {
            name: "grey",
            value: "#808080",
        },
        CssColorEntry {
            name: "honeydew",
            value: "#f0fff0",
        },
        CssColorEntry {
            name: "hotpink",
            value: "#ff69b4",
        },
        CssColorEntry {
            name: "indianred",
            value: "#cd5c5c",
        },
        CssColorEntry {
            name: "indigo",
            value: "#4b0082",
        },
        CssColorEntry {
            name: "ivory",
            value: "#fffff0",
        },
        CssColorEntry {
            name: "khaki",
            value: "#f0e68c",
        },
        CssColorEntry {
            name: "lavender",
            value: "#e6e6fa",
        },
        CssColorEntry {
            name: "lavenderblush",
            value: "#fff0f5",
        },
        CssColorEntry {
            name: "lawngreen",
            value: "#7cfc00",
        },
        CssColorEntry {
            name: "lemonchiffon",
            value: "#fffacd",
        },
        CssColorEntry {
            name: "lightblue",
            value: "#add8e6",
        },
        CssColorEntry {
            name: "lightcoral",
            value: "#f08080",
        },
        CssColorEntry {
            name: "lightcyan",
            value: "#e0ffff",
        },
        CssColorEntry {
            name: "lightgoldenrodyellow",
            value: "#fafad2",
        },
        CssColorEntry {
            name: "lightgray",
            value: "#d3d3d3",
        },
        CssColorEntry {
            name: "lightgreen",
            value: "#90ee90",
        },
        CssColorEntry {
            name: "lightgrey",
            value: "#d3d3d3",
        },
        CssColorEntry {
            name: "lightpink",
            value: "#ffb6c1",
        },
        CssColorEntry {
            name: "lightsalmon",
            value: "#ffa07a",
        },
        CssColorEntry {
            name: "lightseagreen",
            value: "#20b2aa",
        },
        CssColorEntry {
            name: "lightskyblue",
            value: "#87cefa",
        },
        CssColorEntry {
            name: "lightslategray",
            value: "#778899",
        },
        CssColorEntry {
            name: "lightslategrey",
            value: "#778899",
        },
        CssColorEntry {
            name: "lightsteelblue",
            value: "#b0c4de",
        },
        CssColorEntry {
            name: "lightyellow",
            value: "#ffffe0",
        },
        CssColorEntry {
            name: "lime",
            value: "#00ff00",
        },
        CssColorEntry {
            name: "limegreen",
            value: "#32cd32",
        },
        CssColorEntry {
            name: "linen",
            value: "#faf0e6",
        },
        CssColorEntry {
            name: "magenta",
            value: "#ff00ff",
        },
        CssColorEntry {
            name: "maroon",
            value: "#800000",
        },
        CssColorEntry {
            name: "mediumaquamarine",
            value: "#66cdaa",
        },
        CssColorEntry {
            name: "mediumblue",
            value: "#0000cd",
        },
        CssColorEntry {
            name: "mediumorchid",
            value: "#ba55d3",
        },
        CssColorEntry {
            name: "mediumpurple",
            value: "#9370db",
        },
        CssColorEntry {
            name: "mediumseagreen",
            value: "#3cb371",
        },
        CssColorEntry {
            name: "mediumslateblue",
            value: "#7b68ee",
        },
        CssColorEntry {
            name: "mediumspringgreen",
            value: "#00fa9a",
        },
        CssColorEntry {
            name: "mediumturquoise",
            value: "#48d1cc",
        },
        CssColorEntry {
            name: "mediumvioletred",
            value: "#c71585",
        },
        CssColorEntry {
            name: "midnightblue",
            value: "#191970",
        },
        CssColorEntry {
            name: "mintcream",
            value: "#f5fffa",
        },
        CssColorEntry {
            name: "mistyrose",
            value: "#ffe4e1",
        },
        CssColorEntry {
            name: "moccasin",
            value: "#ffe4b5",
        },
        CssColorEntry {
            name: "navajowhite",
            value: "#ffdead",
        },
        CssColorEntry {
            name: "navy",
            value: "#000080",
        },
        CssColorEntry {
            name: "oldlace",
            value: "#fdf5e6",
        },
        CssColorEntry {
            name: "olive",
            value: "#808000",
        },
        CssColorEntry {
            name: "olivedrab",
            value: "#6b8e23",
        },
        CssColorEntry {
            name: "orange",
            value: "#ffa500",
        },
        CssColorEntry {
            name: "orangered",
            value: "#ff4500",
        },
        CssColorEntry {
            name: "orchid",
            value: "#da70d6",
        },
        CssColorEntry {
            name: "palegoldenrod",
            value: "#eee8aa",
        },
        CssColorEntry {
            name: "palegreen",
            value: "#98fb98",
        },
        CssColorEntry {
            name: "paleturquoise",
            value: "#afeeee",
        },
        CssColorEntry {
            name: "palevioletred",
            value: "#db7093",
        },
        CssColorEntry {
            name: "papayawhip",
            value: "#ffefd5",
        },
        CssColorEntry {
            name: "peachpuff",
            value: "#ffdab9",
        },
        CssColorEntry {
            name: "peru",
            value: "#cd853f",
        },
        CssColorEntry {
            name: "pink",
            value: "#ffc0cb",
        },
        CssColorEntry {
            name: "plum",
            value: "#dda0dd",
        },
        CssColorEntry {
            name: "powderblue",
            value: "#b0e0e6",
        },
        CssColorEntry {
            name: "purple",
            value: "#800080",
        },
        CssColorEntry {
            name: "red",
            value: "#ff0000",
        },
        CssColorEntry {
            name: "rosybrown",
            value: "#bc8f8f",
        },
        CssColorEntry {
            name: "royalblue",
            value: "#4169e1",
        },
        CssColorEntry {
            name: "saddlebrown",
            value: "#8b4513",
        },
        CssColorEntry {
            name: "salmon",
            value: "#fa8072",
        },
        CssColorEntry {
            name: "sandybrown",
            value: "#f4a460",
        },
        CssColorEntry {
            name: "seagreen",
            value: "#2e8b57",
        },
        CssColorEntry {
            name: "seashell",
            value: "#fff5ee",
        },
        CssColorEntry {
            name: "sienna",
            value: "#a0522d",
        },
        CssColorEntry {
            name: "silver",
            value: "#c0c0c0",
        },
        CssColorEntry {
            name: "skyblue",
            value: "#87ceeb",
        },
        CssColorEntry {
            name: "slateblue",
            value: "#6a5acd",
        },
        CssColorEntry {
            name: "slategray",
            value: "#708090",
        },
        CssColorEntry {
            name: "slategrey",
            value: "#708090",
        },
        CssColorEntry {
            name: "snow",
            value: "#fffafa",
        },
        CssColorEntry {
            name: "springgreen",
            value: "#00ff7f",
        },
        CssColorEntry {
            name: "steelblue",
            value: "#4682b4",
        },
        CssColorEntry {
            name: "tan",
            value: "#d2b48c",
        },
        CssColorEntry {
            name: "teal",
            value: "#008080",
        },
        CssColorEntry {
            name: "thistle",
            value: "#d8bfd8",
        },
        CssColorEntry {
            name: "tomato",
            value: "#ff6347",
        },
        CssColorEntry {
            name: "turquoise",
            value: "#40e0d0",
        },
        CssColorEntry {
            name: "violet",
            value: "#ee82ee",
        },
        CssColorEntry {
            name: "wheat",
            value: "#f5deb3",
        },
        CssColorEntry {
            name: "white",
            value: "#ffffff",
        },
        CssColorEntry {
            name: "whitesmoke",
            value: "#f5f5f5",
        },
        CssColorEntry {
            name: "yellow",
            value: "#ffff00",
        },
        CssColorEntry {
            name: "yellowgreen",
            value: "#9acd32",
        },
    ];
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_css_color() {
        let color = super::RgbColor::from("#ff0000");
        assert_eq!(color.r, 255);
        assert_eq!(color.g, 0);
        assert_eq!(color.b, 0);
        assert_eq!(color.a, 255);

        let color = super::RgbColor::from("#f00");
        assert_eq!(color.r, 255);
        assert_eq!(color.g, 0);
        assert_eq!(color.b, 0);
        assert_eq!(color.a, 255);

        let color = super::RgbColor::from("#ff0000ff");
        assert_eq!(color.r, 255);
        assert_eq!(color.g, 0);
        assert_eq!(color.b, 0);
        assert_eq!(color.a, 255);

        let color = super::RgbColor::from("#f00f");
        assert_eq!(color.r, 255);
        assert_eq!(color.g, 0);
        assert_eq!(color.b, 0);
        assert_eq!(color.a, 255);

        let color = super::RgbColor::from("#ff0000");
        assert_eq!(color.r, 255);
        assert_eq!(color.g, 0);
        assert_eq!(color.b, 0);
        assert_eq!(color.a, 255);

        let color = super::RgbColor::from("#f00");
        assert_eq!(color.r, 255);
        assert_eq!(color.g, 0);
        assert_eq!(color.b, 0);
        assert_eq!(color.a, 255);

        let color = super::RgbColor::from("#ff0000ff");
        assert_eq!(color.r, 255);
        assert_eq!(color.g, 0);
        assert_eq!(color.b, 0);
        assert_eq!(color.a, 255);

        let color = super::RgbColor::from("#f00f");
        assert_eq!(color.r, 255);
        assert_eq!(color.g, 0);
        assert_eq!(color.b, 0);
        assert_eq!(color.a, 255);
    }

    #[test]
    fn random_colors() {
        let color = super::RgbColor::from("#1234");
        assert_eq!(color.r, 17);
        assert_eq!(color.g, 34);
        assert_eq!(color.b, 51);
        assert_eq!(color.a, 68);

        let color = super::RgbColor::from("#c2e");
        assert_eq!(color.r, 204);
        assert_eq!(color.g, 34);
        assert_eq!(color.b, 238);
        assert_eq!(color.a, 255);

        let color = super::RgbColor::from("#432636");
        assert_eq!(color.r, 67);
        assert_eq!(color.g, 38);
        assert_eq!(color.b, 54);
        assert_eq!(color.a, 255);

        let color = super::RgbColor::from("#10203040");
        assert_eq!(color.r, 16);
        assert_eq!(color.g, 32);
        assert_eq!(color.b, 48);
        assert_eq!(color.a, 64);
    }

    #[test]
    fn wrong_hex_colors() {
        let color = super::RgbColor::from("#incorrect");
        assert_eq!(color.r, 0);
        assert_eq!(color.g, 0);
        assert_eq!(color.b, 0);
        assert_eq!(color.a, 255);

        let color = super::RgbColor::from("ff0000");
        assert_eq!(color.r, 0);
        assert_eq!(color.g, 0);
        assert_eq!(color.b, 0);
        assert_eq!(color.a, 255);

        let color = super::RgbColor::from("abcd");
        assert_eq!(color.r, 0);
        assert_eq!(color.g, 0);
        assert_eq!(color.b, 0);
        assert_eq!(color.a, 255);
    }
}
