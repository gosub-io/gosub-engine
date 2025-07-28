use std::convert::From;
use std::fmt::Debug;
use std::str::FromStr;

use colors_transform::Color;
use colors_transform::{AlphaColor, Hsl, Rgb};
use lazy_static::lazy_static;
// Values for this table is taken from https://www.w3.org/TR/CSS21/propidx.html
// Probably not the complete list, but it will do for now

/// A list of CSS color names
pub struct CssColorEntry {
    pub name: &'static str,
    pub value: &'static str,
}

/// A RGB color with alpha channel
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RgbColor {
    /// Red component
    pub r: f32,
    /// Green component
    pub g: f32,
    /// Blue component
    pub b: f32,
    /// Alpha component (0 = transparent, 255 = solid)
    pub a: f32,
}

impl RgbColor {
    /// Create a new color with r,g,b and alpha values
    #[must_use] 
    pub fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        RgbColor { r, g, b, a }
    }
}

impl Default for RgbColor {
    fn default() -> Self {
        // Default full alpha (solid) with black color
        RgbColor {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 255.0,
        }
    }
}

impl From<&str> for RgbColor {
    fn from(value: &str) -> Self {
        if value.is_empty() {
            return RgbColor::default();
        }
        if value == "currentcolor" {
            // @todo: implement currentcolor
            return RgbColor::default();
        }

        if value.starts_with('#') {
            return parse_hex(value);
        }
        if value.starts_with("rgb(") {
            // Rgb function
            let rgb = Rgb::from_str(value);
            if rgb.is_err() {
                return RgbColor::default();
            }
            let rgb = rgb.unwrap();
            return RgbColor::new(rgb.get_red(), rgb.get_green(), rgb.get_blue(), 255.0);
        }
        if value.starts_with("rgba(") {
            // Rgba function
            let rgb = Rgb::from_str(value);
            if rgb.is_err() {
                return RgbColor::default();
            }
            let rgb = rgb.unwrap();
            return RgbColor::new(rgb.get_red(), rgb.get_green(), rgb.get_blue(), rgb.get_alpha());
        }
        if value.starts_with("hsl(") {
            let hsl = Hsl::from_str(value);
            if hsl.is_err() {
                return RgbColor::default();
            }
            let rgb: Rgb = hsl.unwrap().to_rgb();
            return RgbColor::new(rgb.get_red(), rgb.get_green(), rgb.get_blue(), 255.0);
        }
        if value.starts_with("hsla(") {
            // @TODO: hsla() does not work properly
            // HSLA function
            let hsl = Hsl::from_str(value);
            if hsl.is_err() {
                return RgbColor::default();
            }
            let rgb: Rgb = hsl.unwrap().to_rgb();
            return RgbColor::new(rgb.get_red(), rgb.get_green(), rgb.get_blue(), rgb.get_alpha());
        }

        get_hex_color_from_name(value).map_or(RgbColor::default(), parse_hex)
    }
}

fn get_hex_color_from_name(color_name: &str) -> Option<&str> {
    Some(
        crate::colors::CSS_COLORNAMES
            .iter()
            .find(|entry| entry.name == color_name)?
            .value,
    )
}

fn is_hex(value: &str) -> bool {
    // Check if the input is empty or doesn't start with '#'
    if value.is_empty() || !value.starts_with('#') {
        return false;
    }

    // Check if all characters after '#' are hexadecimal digits
    value.chars().skip(1).all(|c| c.is_ascii_hexdigit())
}

fn parse_hex(value: &str) -> RgbColor {
    const R: usize = 0;
    const G: usize = 1;
    const B: usize = 2;
    const A: usize = 3;
    const DEFAULT_A_VALUE: f32 = 255.0;

    if !is_hex(value) {
        return RgbColor::default();
    }

    // 3 hex digits (RGB)
    if value.len() == 4 {
        let hex_size = 1;
        let number_array = convert_from_hex_str_to_vec_of_ints(value, hex_size);

        let r = number_array[R];
        let g = number_array[G];
        let b = number_array[B];
        return RgbColor::new(
            (r * 16 + r) as f32,
            (g * 16 + g) as f32,
            (b * 16 + b) as f32,
            DEFAULT_A_VALUE,
        );
    }

    // 4 hex digits (RGBA)
    if value.len() == 5 {
        let hex_size = 1;
        let number_array = convert_from_hex_str_to_vec_of_ints(value, hex_size);

        let r = number_array[R];
        let g = number_array[G];
        let b = number_array[B];
        let a = number_array[A];

        return RgbColor::new(
            (r * 16 + r) as f32,
            (g * 16 + g) as f32,
            (b * 16 + b) as f32,
            (a * 16 + a) as f32,
        );
    }

    // 6 hex digits (RRGGBB)
    if value.len() == 7 {
        let hex_size = 2;
        let number_array = convert_from_hex_str_to_vec_of_ints(value, hex_size);
        let r = number_array[R];
        let g = number_array[G];
        let b = number_array[B];

        return RgbColor::new(r as f32, g as f32, b as f32, DEFAULT_A_VALUE);
    }

    // 8 hex digits (RRGGBBAA)
    if value.len() == 9 {
        let hex_size = 2;
        let number_array = convert_from_hex_str_to_vec_of_ints(value, hex_size);

        let r = number_array[R];
        let g = number_array[G];
        let b = number_array[B];
        let a = number_array[A];

        return RgbColor::new(r as f32, g as f32, b as f32, a as f32);
    }

    RgbColor::default()
}

fn convert_from_hex_str_to_vec_of_ints(hex_value: &str, hex_size: usize) -> Vec<i32> {
    const HEX_RADIX: u32 = 16;
    const LINES_TO_SKIP: usize = 1;
    const EXPECT_MESSAGE: &str = "is_hex has failed us";
    // Get the individual chars from the hex then convert from hex -> decimal
    match hex_size {
        // if each hex char is only 1 char long
        1 => {
            hex_value
                .chars()
                .skip(LINES_TO_SKIP) // Skip the # at the front
                .map(|char| i32::from_str_radix(char.to_string().as_str(), HEX_RADIX).expect(EXPECT_MESSAGE)) // Can parse with an unwrap because is_hex made sure it's digits
                .collect::<Vec<i32>>()
        }
        // if each hex char is 2 char long
        2 => {
            // If we're doing a hex value without an `a` value
            let size_without_a = 7;

            let hex_vec = if hex_value.len() == size_without_a {
                vec![&hex_value[1..3], &hex_value[3..5], &hex_value[5..7]]
            } else {
                vec![&hex_value[1..3], &hex_value[3..5], &hex_value[5..7], &hex_value[7..9]]
            };

            hex_vec
                .iter()
                .map(|str| i32::from_str_radix(str, HEX_RADIX).expect(EXPECT_MESSAGE))
                .collect::<Vec<i32>>()
        }
        _ => {
            vec![]
        }
    }
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
        CssColorEntry {
            name: "rebeccapurple",
            value: "#663399",
        },
    ];
}

#[must_use] 
pub fn is_system_color(name: &str) -> bool {
    for entry in &CSS_SYSTEM_COLOR_NAMES {
        if entry == &name {
            return true;
        }
    }
    false
}

#[must_use] 
pub fn is_named_color(name: &str) -> bool {
    for entry in *CSS_COLORNAMES {
        if entry.name == name {
            return true;
        }
    }
    false
}

pub const CSS_SYSTEM_COLOR_NAMES: [&str; 42] = [
    "AccentColor",
    "AccentColorText",
    "ActiveText",
    "ButtonBorder",
    "ButtonFace",
    "ButtonText",
    "Canvas",
    "CanvasText",
    "Field",
    "FieldText",
    "GrayText",
    "Highlight",
    "HighlightText",
    "LinkText",
    "Mark",
    "MarkText",
    "SelectedItem",
    "SelectedItemText",
    "VisitedText",
    "ActiveBorder",
    "ActiveCaption",
    "AppWorkspace",
    "Background",
    "ButtonHighlight",
    "ButtonShadow",
    "CaptionText",
    "InactiveBorder",
    "InactiveCaption",
    "InactiveCaptionText",
    "InfoBackground",
    "InfoText",
    "Menu",
    "MenuText",
    "Scrollbar",
    "ThreeDDarkShadow",
    "ThreeDFace",
    "ThreeDHighlight",
    "ThreeDLightShadow",
    "ThreeDShadow",
    "Window",
    "WindowFrame",
    "WindowText",
];

#[cfg(test)]
mod tests {
    use crate::colors::{convert_from_hex_str_to_vec_of_ints, is_hex};

    #[test]
    fn test_is_hex_good() {
        // Given a good hex value
        let good_hex = "#fffafa";
        // When we see if it is a legit hex value
        let result = is_hex(good_hex);
        // Then we should get true back
        let expected_result = true;
        assert_eq!(result, expected_result);
    }
    #[test]
    fn test_is_hex_bad_no_pound() {
        // Given a bad hex value
        let bad_hex = "hana";
        // When we see if it is a legit hex value
        let result = is_hex(bad_hex);
        // Then we should get false back
        let expected_result = false;
        assert_eq!(result, expected_result);
    }

    #[test]
    fn test_is_hex_bad_not_digit() {
        // Given a bad hex value with a pound
        let bad_hex = "#hana";
        // When we see if it is a legit hex value
        let result = is_hex(bad_hex);
        // Then we should get false back
        let expected_result = false;
        assert_eq!(result, expected_result);
    }

    #[test]
    fn test_is_hex_bad_empty() {
        // Given an empty hex value
        let bad_hex = "";
        // When we see if it is a legit hex value
        let result = is_hex(bad_hex);
        // Then we should get false back
        let expected_result = false;
        assert_eq!(result, expected_result);
    }

    #[test]
    fn convert_hex_test() {
        // Given a valid hex str of length 3
        let hex_str = "#c5f";
        // When we convert to its individual parts
        let conversion = convert_from_hex_str_to_vec_of_ints(hex_str, 1);
        // Then we should get an expected Vec
        let expected_vec = vec![12, 5, 15];
        assert_eq!(expected_vec, conversion);
    }

    #[test]
    fn convert_hex_test_4_digit() {
        // Given a valid hex str of length 4
        let hex_str = "#abcd";
        // When we convert to its individual parts
        let conversion = convert_from_hex_str_to_vec_of_ints(hex_str, 1);
        // Then we should get an expected Vec
        let expected_vec = vec![10, 11, 12, 13];
        assert_eq!(expected_vec, conversion);
    }

    #[test]
    fn test_css_color() {
        let color = super::RgbColor::from("#ff0000");
        assert_eq!(color.r, 255.0);
        assert_eq!(color.g, 0.0);
        assert_eq!(color.b, 0.0);
        assert_eq!(color.a, 255.0);

        let color = super::RgbColor::from("#f00");
        assert_eq!(color.r, 255.0);
        assert_eq!(color.g, 0.0);
        assert_eq!(color.b, 0.0);
        assert_eq!(color.a, 255.0);

        let color = super::RgbColor::from("#ff0000ff");
        assert_eq!(color.r, 255.0);
        assert_eq!(color.g, 0.0);
        assert_eq!(color.b, 0.0);
        assert_eq!(color.a, 255.0);

        let color = super::RgbColor::from("#f00f");
        assert_eq!(color.r, 255.0);
        assert_eq!(color.g, 0.0);
        assert_eq!(color.b, 0.0);
        assert_eq!(color.a, 255.0);

        let color = super::RgbColor::from("#ff0000");
        assert_eq!(color.r, 255.0);
        assert_eq!(color.g, 0.0);
        assert_eq!(color.b, 0.0);
        assert_eq!(color.a, 255.0);

        let color = super::RgbColor::from("#f00");
        assert_eq!(color.r, 255.0);
        assert_eq!(color.g, 0.0);
        assert_eq!(color.b, 0.0);
        assert_eq!(color.a, 255.0);

        let color = super::RgbColor::from("#ff0000ff");
        assert_eq!(color.r, 255.0);
        assert_eq!(color.g, 0.0);
        assert_eq!(color.b, 0.0);
        assert_eq!(color.a, 255.0);

        let color = super::RgbColor::from("#f00f");
        assert_eq!(color.r, 255.0);
        assert_eq!(color.g, 0.0);
        assert_eq!(color.b, 0.0);
        assert_eq!(color.a, 255.0);
    }

    #[test]
    fn random_colors() {
        let color = super::RgbColor::from("#1234");
        assert_eq!(color.r, 17.0);
        assert_eq!(color.g, 34.0);
        assert_eq!(color.b, 51.0);
        assert_eq!(color.a, 68.0);

        let color = super::RgbColor::from("#c2e");
        assert_eq!(color.r, 204.0);
        assert_eq!(color.g, 34.0);
        assert_eq!(color.b, 238.0);
        assert_eq!(color.a, 255.0);

        let color = super::RgbColor::from("#432636");
        assert_eq!(color.r, 67.0);
        assert_eq!(color.g, 38.0);
        assert_eq!(color.b, 54.0);
        assert_eq!(color.a, 255.0);

        let color = super::RgbColor::from("#10203040");
        assert_eq!(color.r, 16.0);
        assert_eq!(color.g, 32.0);
        assert_eq!(color.b, 48.0);
        assert_eq!(color.a, 64.0);
    }

    #[test]
    fn wrong_hex_colors() {
        let color = super::RgbColor::from("#incorrect");
        assert_eq!(color.r, 0.0);
        assert_eq!(color.g, 0.0);
        assert_eq!(color.b, 0.0);
        assert_eq!(color.a, 255.0);

        let color = super::RgbColor::from("ff0000");
        assert_eq!(color.r, 0.0);
        assert_eq!(color.g, 0.0);
        assert_eq!(color.b, 0.0);
        assert_eq!(color.a, 255.0);

        let color = super::RgbColor::from("abcd");
        assert_eq!(color.r, 0.0);
        assert_eq!(color.g, 0.0);
        assert_eq!(color.b, 0.0);
        assert_eq!(color.a, 255.0);
    }

    #[test]
    fn color_names() {
        let color = super::RgbColor::from("red");
        assert_eq!(color.r, 255.0);
        assert_eq!(color.g, 0.0);
        assert_eq!(color.b, 0.0);
        assert_eq!(color.a, 255.0);

        let color = super::RgbColor::from("green");
        assert_eq!(color.r, 0.0);
        assert_eq!(color.g, 128.0);
        assert_eq!(color.b, 0.0);
        assert_eq!(color.a, 255.0);

        let color = super::RgbColor::from("blue");
        assert_eq!(color.r, 0.0);
        assert_eq!(color.g, 0.0);
        assert_eq!(color.b, 255.0);
        assert_eq!(color.a, 255.0);

        let color = super::RgbColor::from("rebeccapurple");
        assert_eq!(color.r, 0x66 as f32);
        assert_eq!(color.g, 0x33 as f32);
        assert_eq!(color.b, 0x99 as f32);
        assert_eq!(color.a, 255.0);
    }

    #[test]
    fn rgb_func_colors() {
        let color = super::RgbColor::from("rgb(10, 20, 30)");
        assert_eq!(color.r, 10.0);
        assert_eq!(color.g, 20.0);
        assert_eq!(color.b, 30.0);
        assert_eq!(color.a, 255.0);

        // invalid color
        let color = super::RgbColor::from("rgb(10)");
        assert_eq!(color.r, 0.0);
        assert_eq!(color.g, 0.0);
        assert_eq!(color.b, 0.0);
        assert_eq!(color.a, 255.0);
    }

    #[test]
    fn hsl_func_colors() {
        let color = super::RgbColor::from("hsl(10, 20%, 30%)");
        assert_eq!(color.r, 91.8);
        assert_eq!(color.g, 66.3);
        assert_eq!(color.b, 61.2);
        assert_eq!(color.a, 255.0);

        // invalid color
        let color = super::RgbColor::from("hsl(10)");
        assert_eq!(color.r, 0.0);
        assert_eq!(color.g, 0.0);
        assert_eq!(color.b, 0.0);
        assert_eq!(color.a, 255.0);
    }
}
