use cow_utils::CowUtils;
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

/// The CSSOM serialization of an sRGB colour.
///
/// css-color-4 says a resolved sRGB colour serializes through the legacy comma form - `rgb()`
/// when it is opaque, `rgba()` when it is not - whatever notation the author used to write it.
/// `#f00`, `rgb(255 0 0)` and `red` all come back as `rgb(255, 0, 0)`, which is what both
/// `getComputedStyle` and a round-trip through `element.style` are required to report.
impl std::fmt::Display for RgbColor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (r, g, b) = (self.r.round() as u8, self.g.round() as u8, self.b.round() as u8);
        let alpha = self.a / 255.0;
        if alpha >= 1.0 {
            return write!(f, "rgb({r}, {g}, {b})");
        }
        // Alpha is held as one of 256 steps, so three decimals is always enough to tell two
        // apart (1/255 is 0.0039) and never emits the noise `{}` on an f32 would - `128/255`
        // is 0.50196078 in full, and every browser writes it `0.502`.
        let alpha = format!("{:.3}", alpha.max(0.0));
        let alpha = alpha.trim_end_matches('0').trim_end_matches('.');
        write!(f, "rgba({r}, {g}, {b}, {alpha})")
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
    /// Lossy conversion: anything that is not a colour becomes opaque black. Prefer
    /// [`RgbColor::try_from_str`], which reports that case instead of inventing a colour.
    fn from(value: &str) -> Self {
        RgbColor::try_from_str(value).unwrap_or_default()
    }
}

impl RgbColor {
    /// Parses a CSS `<color>` from its textual form, or `None` when the string is not a colour.
    ///
    /// Reporting the failure matters: a declaration whose value is invalid at computed-value
    /// time must be dropped, so the property keeps its inherited or initial value. Answering
    /// with a colour instead turns every keyword that reaches a colour slot into an opaque
    /// black paint - `background: none` and `background: no-repeat` (the shorthand's non-colour
    /// components) both filled their box with black.
    #[must_use]
    pub fn try_from_str(value: &str) -> Option<Self> {
        if value.is_empty() {
            return None;
        }
        // CSS defines it as rgba(0, 0, 0, 0), and the named-colour table does not carry it.
        if value.eq_ignore_ascii_case("transparent") {
            return Some(RgbColor::new(0.0, 0.0, 0.0, 0.0));
        }
        if value.eq_ignore_ascii_case("currentcolor") {
            // @todo: implement currentcolor - it resolves to the element's own `color`, which
            // is not reachable from here. Black keeps the pre-existing behaviour; returning
            // `None` instead would silently drop every `currentcolor` declaration.
            return Some(RgbColor::default());
        }

        if value.starts_with('#') {
            return try_parse_hex(value);
        }
        if value.starts_with("rgb(") {
            // Rgb function
            let rgb = Rgb::from_str(value).ok()?;
            return Some(RgbColor::new(rgb.get_red(), rgb.get_green(), rgb.get_blue(), 255.0));
        }
        if value.starts_with("rgba(") {
            // Rgba function - alpha from colors_transform is in 0..1 range; scale to 0..255
            let rgb = Rgb::from_str(value).ok()?;
            return Some(RgbColor::new(
                rgb.get_red(),
                rgb.get_green(),
                rgb.get_blue(),
                rgb.get_alpha() * 255.0,
            ));
        }
        if value.starts_with("hsl(") {
            let hsl = Hsl::from_str(value).ok()?;
            let rgb: Rgb = hsl.to_rgb();
            return Some(RgbColor::new(rgb.get_red(), rgb.get_green(), rgb.get_blue(), 255.0));
        }
        if value.starts_with("hsla(") {
            // hsla() - alpha from colors_transform is in 0..1 range; scale to 0..255
            let hsl = Hsl::from_str(value).ok()?;
            let rgb: Rgb = hsl.to_rgb();
            return Some(RgbColor::new(
                rgb.get_red(),
                rgb.get_green(),
                rgb.get_blue(),
                rgb.get_alpha() * 255.0,
            ));
        }

        // Modern CSS Color Level 4 functions stored as unparsed strings.
        if value.starts_with("oklch(") {
            return parse_oklch_str(value);
        }
        if value.starts_with("oklab(") {
            return parse_oklab_str(value);
        }

        named_color_hex(value).and_then(try_parse_hex)
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

/// Parses `#rgb`, `#rgba`, `#rrggbb` and `#rrggbbaa`, or `None` when the string is not one of
/// those - a malformed hex value is an invalid declaration, not black.
fn try_parse_hex(value: &str) -> Option<RgbColor> {
    const R: usize = 0;
    const G: usize = 1;
    const B: usize = 2;
    const A: usize = 3;
    const DEFAULT_A_VALUE: f32 = 255.0;

    if !is_hex(value) {
        return None;
    }

    // 3 hex digits (RGB)
    if value.len() == 4 {
        let hex_size = 1;
        let number_array = convert_from_hex_str_to_vec_of_ints(value, hex_size);

        let r = number_array[R];
        let g = number_array[G];
        let b = number_array[B];
        return Some(RgbColor::new(
            (r * 16 + r) as f32,
            (g * 16 + g) as f32,
            (b * 16 + b) as f32,
            DEFAULT_A_VALUE,
        ));
    }

    // 4 hex digits (RGBA)
    if value.len() == 5 {
        let hex_size = 1;
        let number_array = convert_from_hex_str_to_vec_of_ints(value, hex_size);

        let r = number_array[R];
        let g = number_array[G];
        let b = number_array[B];
        let a = number_array[A];

        return Some(RgbColor::new(
            (r * 16 + r) as f32,
            (g * 16 + g) as f32,
            (b * 16 + b) as f32,
            (a * 16 + a) as f32,
        ));
    }

    // 6 hex digits (RRGGBB)
    if value.len() == 7 {
        let hex_size = 2;
        let number_array = convert_from_hex_str_to_vec_of_ints(value, hex_size);
        let r = number_array[R];
        let g = number_array[G];
        let b = number_array[B];

        return Some(RgbColor::new(r as f32, g as f32, b as f32, DEFAULT_A_VALUE));
    }

    // 8 hex digits (RRGGBBAA)
    if value.len() == 9 {
        let hex_size = 2;
        let number_array = convert_from_hex_str_to_vec_of_ints(value, hex_size);

        let r = number_array[R];
        let g = number_array[G];
        let b = number_array[B];
        let a = number_array[A];

        return Some(RgbColor::new(r as f32, g as f32, b as f32, a as f32));
    }

    None
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

    #[test]
    fn non_colours_are_reported_rather_than_blackened() {
        // These reach a colour slot through the `background` shorthand, whose non-colour
        // components used to parse as opaque black and fill the element's box.
        for keyword in ["none", "no-repeat", "center", "inherit", "", "notacolour"] {
            assert_eq!(
                super::RgbColor::try_from_str(keyword),
                None,
                "{keyword} is not a colour"
            );
        }
    }

    #[test]
    fn malformed_values_are_reported() {
        for value in ["#incorrect", "ff0000", "abcd", "#12345", "rgb(bogus)", "hsl(nope)"] {
            assert_eq!(super::RgbColor::try_from_str(value), None, "{value} is not a colour");
        }
    }

    #[test]
    fn transparent_is_a_colour_with_zero_alpha() {
        let color = super::RgbColor::try_from_str("transparent").expect("transparent is a colour");
        assert_eq!((color.r, color.g, color.b, color.a), (0.0, 0.0, 0.0, 0.0));
        // CSS keywords are ASCII case-insensitive.
        assert_eq!(super::RgbColor::try_from_str("TRANSPARENT"), Some(color));
    }

    #[test]
    fn colours_still_parse() {
        assert_eq!(
            super::RgbColor::try_from_str("#ff0000"),
            Some(super::RgbColor::new(255.0, 0.0, 0.0, 255.0))
        );
        assert_eq!(
            super::RgbColor::try_from_str("red"),
            Some(super::RgbColor::new(255.0, 0.0, 0.0, 255.0))
        );
        assert_eq!(
            super::RgbColor::try_from_str("rgb(255, 0, 0)"),
            Some(super::RgbColor::new(255.0, 0.0, 0.0, 255.0))
        );
    }

    #[test]
    fn the_lossy_conversion_keeps_its_black_default() {
        // `From<&str>` is unchanged for callers that have nowhere to report a failure.
        assert_eq!(super::RgbColor::from("none"), super::RgbColor::default());
    }
}

/// Convert an HWB colour to sRGB (css-color-4 §7.2).
///
/// The hue is the fully saturated colour at that angle, and whiteness and blackness say how much
/// of it is replaced by white and by black. The two are normalized when they would leave nothing
/// of the hue at all: `hwb(0 60% 60%)` is grey, not a negative amount of red.
#[must_use]
pub fn hwb_to_srgb(hue: f32, white: f32, black: f32) -> (f32, f32, f32) {
    let (mut white, mut black) = (white, black);
    let total = white + black;
    if total > 1.0 {
        white /= total;
        black /= total;
    }
    let (r, g, b) = hsl_to_srgb(hue, 1.0, 0.5);
    let mix = |channel: f32| (channel / 255.0).mul_add(1.0 - white - black, white) * 255.0;
    (mix(r), mix(g), mix(b))
}

pub fn hsl_to_srgb(h: f32, s: f32, l: f32) -> (f32, f32, f32) {
    let h = h.rem_euclid(360.0) / 360.0;
    if s <= 0.0 {
        let v = l * 255.0;
        return (v, v, v);
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let hue = |mut t: f32| -> f32 {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        let c = if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 1.0 / 2.0 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        };
        c * 255.0
    };
    (hue(h + 1.0 / 3.0), hue(h), hue(h - 1.0 / 3.0))
}

/// Convert a CIE Lab colour to sRGB (css-color-4 §10.3, via XYZ).
///
/// Lab is defined against the D50 white point, so the matrix below folds the Bradford adaptation
/// to D65 into the XYZ-to-linear-sRGB conversion. The result is gamma-encoded and scaled to the
/// 0-255 channels the rest of the engine paints with; a colour outside the sRGB gamut comes back
/// clipped, which is what a display can show of it.
#[must_use]
pub fn lab_to_srgb(lightness: f32, a: f32, b: f32) -> (f32, f32, f32) {
    const KAPPA: f32 = 24389.0 / 27.0;
    const EPSILON: f32 = 216.0 / 24389.0;
    // The D50 white point, which Lab is measured against.
    const WHITE: [f32; 3] = [0.964_295_7, 1.0, 0.825_104_6];

    let fy = (lightness + 16.0) / 116.0;
    let fx = a / 500.0 + fy;
    let fz = fy - b / 200.0;
    let cube = |f: f32| {
        let cubed = f * f * f;
        if cubed > EPSILON {
            cubed
        } else {
            f.mul_add(116.0, -16.0) / KAPPA
        }
    };
    let x = cube(fx) * WHITE[0];
    let y = if lightness > KAPPA * EPSILON {
        fy * fy * fy
    } else {
        lightness / KAPPA
    } * WHITE[1];
    let z = cube(fz) * WHITE[2];

    // XYZ (D50) straight to linear sRGB.
    let r = 3.134_136 * x - 1.617_386_3 * y - 0.490_661_95 * z;
    let g = -0.978_795_5 * x + 1.916_140_4 * y + 0.033_417_27 * z;
    let bl = 0.071_955_38 * x - 0.228_976_83 * y + 1.405_386 * z;

    let encode = |channel: f32| {
        let channel = channel.clamp(0.0, 1.0);
        let encoded = if channel <= 0.003_130_8 {
            channel * 12.92
        } else {
            1.055 * channel.powf(1.0 / 2.4) - 0.055
        };
        encoded * 255.0
    };
    (encode(r), encode(g), encode(bl))
}

/// Convert a CIE LCH colour to sRGB. LCH is Lab in polar form: the hue is an angle and the
/// chroma is how far the colour sits from the neutral axis.
#[must_use]
pub fn lch_to_srgb(lightness: f32, chroma: f32, hue_deg: f32) -> (f32, f32, f32) {
    let hue = hue_deg.to_radians();
    lab_to_srgb(lightness, chroma * hue.cos(), chroma * hue.sin())
}

/// How a colour was written, which is also what decides how it serializes.
///
/// css-color-4 §15 does not serialize every colour the same way. A colour written as a keyword,
/// a hex triple or `rgb()` is an sRGB colour and comes back through `rgb()`; one written
/// `hsl()` or `hwb()` does too, but only while every component is present; and the rest keep the
/// notation they were written in, because no other notation can say what they say.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorSyntax {
    /// A keyword, a hex triple, `rgb()` or `rgba()`. Components are 0-255.
    Rgb,
    /// `hsl()` or `hsla()`: hue in degrees, saturation and lightness as percentages.
    Hsl,
    /// `hwb()`: hue in degrees, whiteness and blackness as percentages.
    Hwb,
    /// `lab()`: lightness, and the two opponent axes.
    Lab,
    /// `lch()`: lightness, chroma, hue in degrees.
    Lch,
    /// `oklab()`.
    Oklab,
    /// `oklch()`.
    Oklch,
    /// `color(<space> ...)`: components are 0-1 in the named space.
    Predefined(PredefinedSpace),
}

/// The colour spaces `color()` can name (css-color-4 §10).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PredefinedSpace {
    Srgb,
    SrgbLinear,
    DisplayP3,
    A98Rgb,
    ProphotoRgb,
    Rec2020,
    XyzD50,
    XyzD65,
}

impl PredefinedSpace {
    /// The space's name as `color()` spells it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            PredefinedSpace::Srgb => "srgb",
            PredefinedSpace::SrgbLinear => "srgb-linear",
            PredefinedSpace::DisplayP3 => "display-p3",
            PredefinedSpace::A98Rgb => "a98-rgb",
            PredefinedSpace::ProphotoRgb => "prophoto-rgb",
            PredefinedSpace::Rec2020 => "rec2020",
            PredefinedSpace::XyzD50 => "xyz-d50",
            PredefinedSpace::XyzD65 => "xyz-d65",
        }
    }

    /// The space a `color()` keyword names, or `None` when it names none of them.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        let space = match name.cow_to_ascii_lowercase().as_ref() {
            "srgb" => PredefinedSpace::Srgb,
            "srgb-linear" => PredefinedSpace::SrgbLinear,
            "display-p3" => PredefinedSpace::DisplayP3,
            "a98-rgb" => PredefinedSpace::A98Rgb,
            "prophoto-rgb" => PredefinedSpace::ProphotoRgb,
            "rec2020" => PredefinedSpace::Rec2020,
            // `xyz` is a synonym for `xyz-d65`, and serializes as the name it was given.
            "xyz-d50" => PredefinedSpace::XyzD50,
            "xyz" | "xyz-d65" => PredefinedSpace::XyzD65,
            _ => return None,
        };
        Some(space)
    }
}

/// A CSS colour: the notation it was written in, its three components, and its alpha.
///
/// A component is `None` when it was written `none`. css-color-4 §12.2 calls that a *missing*
/// component, and it is not the same as zero: it says the colour has nothing to contribute on
/// that axis, which matters when the colour is interpolated. sRGB has no way to write it, which
/// is why a colour that has one keeps the notation it came in.
///
/// Storing the components in the notation's own units, rather than converting to sRGB up front,
/// is the point of the type. A converted colour cannot say which space it was in, cannot say a
/// component was missing, and cannot be given back the way it was written - and all three are
/// things the CSSOM is required to report.
#[derive(Clone, Copy, Debug)]
pub struct CssColor {
    pub syntax: ColorSyntax,
    /// The three components, in the units of `syntax`.
    ///
    /// Held at the precision they were parsed with. Narrowing them to `f32` costs a digit that
    /// the CSSOM reports: `128/255` is `0.50196078`, and an `f32` says `0.50196081`.
    pub components: [Option<f64>; 3],
    /// Alpha, 0 to 1.
    pub alpha: Option<f64>,
    /// Whether this is a computed value rather than a specified one.
    ///
    /// The two serialize differently in one place: an `hsl()` or `hwb()` colour that has to keep
    /// its own notation writes its saturation and lightness as bare numbers when specified and
    /// as percentages once computed. `element.style.color = "hsl(120 80% none)"` reads back
    /// `hsl(120 80 none)`, while `getComputedStyle` reports `hsl(120 80% none)`.
    pub computed: bool,
}

/// Two colours are the same when they describe the same colour, whatever notation each was
/// written in. Deriving this would make `red` and `#f00` different values.
impl PartialEq for CssColor {
    fn eq(&self, other: &Self) -> bool {
        let (a, b) = (self.to_rgb(), other.to_rgb());
        (a.r - b.r).abs() < 0.5 && (a.g - b.g).abs() < 0.5 && (a.b - b.b).abs() < 0.5 && (a.a - b.a).abs() < 0.5
    }
}

impl CssColor {
    /// An sRGB colour from 0-255 channels and a 0-255 alpha, which is how [`RgbColor`] holds it.
    #[must_use]
    pub fn srgb(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self {
            syntax: ColorSyntax::Rgb,
            components: [Some(f64::from(r)), Some(f64::from(g)), Some(f64::from(b))],
            alpha: Some(f64::from(a) / 255.0),
            computed: false,
        }
    }

    /// Whether this colour serializes in the notation it was written in, rather than through
    /// the legacy sRGB triple. Only such a colour can show a `calc()` a component was written
    /// with, so only such a colour has to keep one unresolved.
    #[must_use]
    pub fn keeps_its_notation(&self) -> bool {
        match self.syntax {
            ColorSyntax::Rgb => false,
            ColorSyntax::Hsl | ColorSyntax::Hwb => self.has_missing(),
            _ => true,
        }
    }

    /// Whether any component or the alpha was written `none`.
    #[must_use]
    pub fn has_missing(&self) -> bool {
        self.alpha.is_none() || self.components.iter().any(Option::is_none)
    }

    /// The colour as sRGB, for everything downstream that paints rather than serializes.
    ///
    /// A missing component resolves to zero here, which is what css-color-4 §12.2 asks for when
    /// a colour has to be used rather than carried: the value is not being interpolated any more,
    /// so there is nothing left for "missing" to mean.
    #[must_use]
    pub fn to_rgb(&self) -> RgbColor {
        // Narrowed here, at the boundary with the painting triple, rather than on the way in.
        #[expect(clippy::cast_possible_truncation, reason = "a colour channel fits an f32")]
        let [first, second, third] = self.components.map(|c| c.unwrap_or(0.0) as f32);
        // A missing alpha is zero, like any other missing component: `none` is not "absent",
        // which would be opaque, but "nothing to contribute" (css-color-4 §12.2).
        #[expect(clippy::cast_possible_truncation, reason = "alpha is a fraction")]
        let alpha = self.alpha.unwrap_or(0.0) as f32 * 255.0;
        let (r, g, b) = match self.syntax {
            ColorSyntax::Rgb => (first, second, third),
            ColorSyntax::Hsl => hsl_to_srgb(first, second / 100.0, third / 100.0),
            ColorSyntax::Hwb => hwb_to_srgb(first, second / 100.0, third / 100.0),
            ColorSyntax::Oklab => oklab_to_srgb(first, second, third),
            ColorSyntax::Oklch => oklch_to_srgb(first, second, third),
            ColorSyntax::Lab => lab_to_srgb(first, second, third),
            ColorSyntax::Lch => lch_to_srgb(first, second, third),
            // Every predefined space is read as though it were sRGB. Converting between them is
            // a matrix apiece and nothing downstream asks for it yet.
            ColorSyntax::Predefined(_) => (first * 255.0, second * 255.0, third * 255.0),
        };
        RgbColor::new(r, g, b, alpha)
    }
}

impl From<RgbColor> for CssColor {
    fn from(color: RgbColor) -> Self {
        CssColor::srgb(color.r, color.g, color.b, color.a)
    }
}

/// A component as css-color-4 writes it: up to eight decimals, with nothing trailing.
fn component(value: Option<f64>) -> String {
    let Some(value) = value else {
        return "none".to_string();
    };
    let text = format!("{value:.8}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text.is_empty() || text == "-0" {
        "0".to_string()
    } else {
        text.to_string()
    }
}

impl std::fmt::Display for CssColor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // An sRGB colour, and an HSL or HWB one with nothing missing, go out through the legacy
        // comma form - which is what every browser reports and what the CSSOM requires, whatever
        // notation the author used (css-color-4 §15.2).
        let legacy = match self.syntax {
            ColorSyntax::Rgb => true,
            ColorSyntax::Hsl | ColorSyntax::Hwb => !self.has_missing(),
            _ => false,
        };
        if legacy {
            // An sRGB colour that has a missing component cannot say so through `rgb()`, which
            // predates the idea. Its *computed* value moves to the modern notation to keep it,
            // where the specified value still goes out as the legacy triple with the missing
            // component read as zero.
            if self.computed && self.syntax == ColorSyntax::Rgb && self.has_missing() {
                let channel = |c: Option<f64>| c.map(|v| v / 255.0);
                let srgb = CssColor {
                    syntax: ColorSyntax::Predefined(PredefinedSpace::Srgb),
                    components: self.components.map(channel),
                    alpha: self.alpha,
                    computed: true,
                };
                return write!(f, "{srgb}");
            }
            return write!(f, "{}", self.to_rgb());
        }

        let [first, second, third] = self.components;
        let (name, second, third) = match self.syntax {
            // Handled above, where it goes out as the legacy triple. Writing it again here
            // rather than declaring it unreachable keeps the match total.
            ColorSyntax::Rgb => return write!(f, "{}", self.to_rgb()),
            // Saturation, lightness, whiteness and blackness are percentages, and say so.
            ColorSyntax::Hsl => ("hsl", self.axis(second), self.axis(third)),
            ColorSyntax::Hwb => ("hwb", self.axis(second), self.axis(third)),
            ColorSyntax::Lab => ("lab", component(second), component(third)),
            ColorSyntax::Lch => ("lch", component(second), component(third)),
            ColorSyntax::Oklab => ("oklab", component(second), component(third)),
            ColorSyntax::Oklch => ("oklch", component(second), component(third)),
            ColorSyntax::Predefined(space) => {
                let alpha = alpha_suffix(self.alpha);
                return write!(
                    f,
                    "color({} {} {} {}{alpha})",
                    space.name(),
                    component(first),
                    component(second),
                    component(third)
                );
            }
        };
        write!(
            f,
            "{name}({} {second} {third}{})",
            component(first),
            alpha_suffix(self.alpha)
        )
    }
}

impl CssColor {
    /// One of the two percentage axes of `hsl()` or `hwb()`, written the way this value's stage
    /// writes it: bare when specified, with a percent sign once computed.
    fn axis(&self, value: Option<f64>) -> String {
        match (value, self.computed) {
            (Some(_), true) => format!("{}%", component(value)),
            (Some(_), false) => component(value),
            (None, _) => "none".to_string(),
        }
    }
}

/// The ` / alpha` a modern colour notation ends with, left off when the colour is opaque.
fn alpha_suffix(alpha: Option<f64>) -> String {
    match alpha {
        Some(alpha) if (alpha - 1.0).abs() < f64::EPSILON => String::new(),
        Some(alpha) => format!(" / {}", component(Some(alpha))),
        None => " / none".to_string(),
    }
}
