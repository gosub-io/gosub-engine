use std::convert::From;
use std::fmt::Debug;
use std::str::FromStr;

use colors_transform::Color;
use colors_transform::{AlphaColor, Hsl, Rgb};

// The named-color table lives in gosub_shared so the render pipeline can resolve
// the same names without depending on this crate; re-exported here for existing users.
pub use gosub_shared::css_colors::{
    is_named_color, is_system_color, named_color_hex, CssColorEntry, CSS_COLORNAMES, CSS_SYSTEM_COLOR_NAMES,
};

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
            let Ok(rgb) = Rgb::from_str(value) else {
                return RgbColor::default();
            };
            return RgbColor::new(rgb.get_red(), rgb.get_green(), rgb.get_blue(), 255.0);
        }
        if value.starts_with("rgba(") {
            // Rgba function - alpha from colors_transform is in 0..1 range; scale to 0..255
            let Ok(rgb) = Rgb::from_str(value) else {
                return RgbColor::default();
            };
            return RgbColor::new(rgb.get_red(), rgb.get_green(), rgb.get_blue(), rgb.get_alpha() * 255.0);
        }
        if value.starts_with("hsl(") {
            let Ok(hsl) = Hsl::from_str(value) else {
                return RgbColor::default();
            };
            let rgb: Rgb = hsl.to_rgb();
            return RgbColor::new(rgb.get_red(), rgb.get_green(), rgb.get_blue(), 255.0);
        }
        if value.starts_with("hsla(") {
            // hsla() - alpha from colors_transform is in 0..1 range; scale to 0..255
            let Ok(hsl) = Hsl::from_str(value) else {
                return RgbColor::default();
            };
            let rgb: Rgb = hsl.to_rgb();
            return RgbColor::new(rgb.get_red(), rgb.get_green(), rgb.get_blue(), rgb.get_alpha() * 255.0);
        }

        // Modern CSS Color Level 4 functions stored as unparsed strings.
        if value.starts_with("oklch(") {
            if let Some(c) = parse_oklch_str(value) {
                return c;
            }
        }
        if value.starts_with("oklab(") {
            if let Some(c) = parse_oklab_str(value) {
                return c;
            }
        }

        named_color_hex(value).map_or(RgbColor::default(), parse_hex)
    }
}

/// Parse `oklch(L C H [/ alpha])` from a raw CSS string into an `RgbColor`.
fn parse_oklch_str(s: &str) -> Option<RgbColor> {
    let inner = s.strip_prefix("oklch(")?.strip_suffix(')')?;
    let nums = parse_space_nums(inner);
    if nums.len() < 3 {
        return None;
    }
    let (r, g, b) = oklch_to_srgb(nums[0], nums[1], nums[2]);
    let a = nums.get(3).copied().unwrap_or(1.0) * 255.0;
    Some(RgbColor::new(r, g, b, a))
}

/// Parse `oklab(L a b [/ alpha])` from a raw CSS string into an `RgbColor`.
fn parse_oklab_str(s: &str) -> Option<RgbColor> {
    let inner = s.strip_prefix("oklab(")?.strip_suffix(')')?;
    let nums = parse_space_nums(inner);
    if nums.len() < 3 {
        return None;
    }
    let (r, g, b) = oklab_to_srgb(nums[0], nums[1], nums[2]);
    let a = nums.get(3).copied().unwrap_or(1.0) * 255.0;
    Some(RgbColor::new(r, g, b, a))
}

/// Extract whitespace-/slash-separated floats from a CSS function argument string.
/// Strips trailing `%` and skips non-numeric tokens (like the `/` slash).
fn parse_space_nums(s: &str) -> Vec<f32> {
    s.split(|c: char| c.is_ascii_whitespace() || c == '/')
        .filter_map(|tok| {
            let tok = tok.trim().trim_end_matches('%');
            tok.parse::<f32>().ok()
        })
        .collect()
}

/// Convert an oklch(L C H) triplet to an sRGB [r,g,b] triplet in the 0.0–255.0 range.
/// L: 0.0–1.0 lightness, C: 0.0–0.37+ chroma, H: hue in degrees.
pub fn oklch_to_srgb(l: f32, c: f32, h_deg: f32) -> (f32, f32, f32) {
    // oklch → oklab
    let h = h_deg * std::f32::consts::PI / 180.0;
    let a = c * h.cos();
    let b = c * h.sin();

    // oklab → linear sRGB (M2 and M1 matrices from the Oklab specification)
    let l_ = l + 0.396_337_78 * a + 0.215_803_76 * b;
    let m_ = l - 0.105_561_35 * a - 0.063_854_17 * b;
    let s_ = l - 0.089_484_18 * a - 1.291_485_5 * b;

    let l_c = l_ * l_ * l_;
    let m_c = m_ * m_ * m_;
    let s_c = s_ * s_ * s_;

    let r_lin = 4.076_741_7 * l_c - 3.307_711_6 * m_c + 0.230_97 * s_c;
    let g_lin = -1.268_438 * l_c + 2.609_757_4 * m_c - 0.341_319_4 * s_c;
    let b_lin = -0.004_196_1 * l_c - 0.703_418_6 * m_c + 1.707_614_7 * s_c;

    // linear sRGB → gamma-corrected sRGB
    let gamma = |x: f32| -> f32 {
        if x <= 0.003_130_8 {
            12.92 * x
        } else {
            1.055 * x.powf(1.0 / 2.4) - 0.055
        }
    };

    (
        gamma(r_lin).clamp(0.0, 1.0) * 255.0,
        gamma(g_lin).clamp(0.0, 1.0) * 255.0,
        gamma(b_lin).clamp(0.0, 1.0) * 255.0,
    )
}

/// Convert an oklab(L a b) triplet to an sRGB [r,g,b] triplet in the 0.0–255.0 range.
pub fn oklab_to_srgb(l: f32, a: f32, b: f32) -> (f32, f32, f32) {
    let l_ = l + 0.396_337_78 * a + 0.215_803_76 * b;
    let m_ = l - 0.105_561_35 * a - 0.063_854_17 * b;
    let s_ = l - 0.089_484_18 * a - 1.291_485_5 * b;

    let l_c = l_ * l_ * l_;
    let m_c = m_ * m_ * m_;
    let s_c = s_ * s_ * s_;

    let r_lin = 4.076_741_7 * l_c - 3.307_711_6 * m_c + 0.230_97 * s_c;
    let g_lin = -1.268_438 * l_c + 2.609_757_4 * m_c - 0.341_319_4 * s_c;
    let b_lin = -0.004_196_1 * l_c - 0.703_418_6 * m_c + 1.707_614_7 * s_c;

    let gamma = |x: f32| -> f32 {
        if x <= 0.003_130_8 {
            12.92 * x
        } else {
            1.055 * x.powf(1.0 / 2.4) - 0.055
        }
    };

    (
        gamma(r_lin).clamp(0.0, 1.0) * 255.0,
        gamma(g_lin).clamp(0.0, 1.0) * 255.0,
        gamma(b_lin).clamp(0.0, 1.0) * 255.0,
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
    // Get the individual chars from the hex then convert from hex -> decimal
    match hex_size {
        // if each hex char is only 1 char long
        1 => {
            hex_value
                .chars()
                .skip(LINES_TO_SKIP) // Skip the # at the front
                .map(|char| i32::from_str_radix(char.to_string().as_str(), HEX_RADIX).unwrap_or(0)) // is_hex() above guarantees digits
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
                .map(|str| i32::from_str_radix(str, HEX_RADIX).unwrap_or(0))
                .collect::<Vec<i32>>()
        }
        _ => {
            vec![]
        }
    }
}

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
