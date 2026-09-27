//! `color-mix()` (css-color-5 section 3).
//!
//! `color-mix(in lch longer hue, red 30%, blue)` mixes one or more colours in the named colour
//! space. The interpolation method is optional and defaults to Oklab. Each colour can carry a
//! percentage from 0% to 100%.
//!
//! The specified value keeps the function, in canonical form: [`canonical`] is what the syntax
//! matcher calls. The computed value is the mixed colour: [`resolve`] is what the computed stage
//! calls, through `fold_color_function`. A mix whose colours or percentages cannot be worked out
//! here (`currentcolor`, a percentage that needs a font size) is left as a function.

use cow_utils::CowUtils;

use super::relative::{canonical_origin, resolve_origin};
use super::space::{self, normalize_hue, Space};
use super::{ColorSyntax, CssColor, PredefinedSpace};
use crate::functions::calc;
use crate::stylesheet::CssValue;

/// Whether `name` is `color-mix`.
#[must_use]
pub(crate) fn is_color_mix(name: &str) -> bool {
    name.eq_ignore_ascii_case("color-mix")
}

/// How hues are interpolated in a polar space (css-color-4 section 13.5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HueMethod {
    Shorter,
    Longer,
    Increasing,
    Decreasing,
}

impl HueMethod {
    fn from_name(name: &str) -> Option<Self> {
        Some(match name.cow_to_ascii_lowercase().as_ref() {
            "shorter" => HueMethod::Shorter,
            "longer" => HueMethod::Longer,
            "increasing" => HueMethod::Increasing,
            "decreasing" => HueMethod::Decreasing,
            _ => return None,
        })
    }

    fn name(self) -> &'static str {
        match self {
            HueMethod::Shorter => "shorter",
            HueMethod::Longer => "longer",
            HueMethod::Increasing => "increasing",
            HueMethod::Decreasing => "decreasing",
        }
    }
}

/// A `<color-interpolation-method>`: the space and, for a polar space, the hue method.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Method {
    space: Space,
    /// The space's name as it serializes. `xyz` is written `xyz-d65`.
    name: &'static str,
    hue: HueMethod,
}

impl Method {
    /// Oklab, used when no method is given.
    const DEFAULT: Method = Method {
        space: Space::Oklab,
        name: "oklab",
        hue: HueMethod::Shorter,
    };

    fn parse(values: &[CssValue]) -> Option<Self> {
        let words: Vec<&str> = values
            .iter()
            .map(|value| match value {
                CssValue::String(word) => Some(word.as_str()),
                _ => None,
            })
            .collect::<Option<_>>()?;
        let [keyword, space, rest @ ..] = words.as_slice() else {
            return None;
        };
        if !keyword.eq_ignore_ascii_case("in") {
            return None;
        }
        let (space, name) = space_named(space)?;
        let hue = match rest {
            [] => HueMethod::Shorter,
            [method, hue] if hue.eq_ignore_ascii_case("hue") && space.hue_index().is_some() => {
                HueMethod::from_name(method)?
            }
            _ => return None,
        };
        Some(Method { space, name, hue })
    }

    /// The method's canonical tokens, or nothing when it is the default. The default hue method,
    /// `shorter`, is left out too.
    fn canonical(self) -> Vec<CssValue> {
        if self == Method::DEFAULT {
            return Vec::new();
        }
        let mut out = vec![
            CssValue::String("in".to_string()),
            CssValue::String(self.name.to_string()),
        ];
        if self.hue != HueMethod::Shorter {
            out.push(CssValue::String(self.hue.name().to_string()));
            out.push(CssValue::String("hue".to_string()));
        }
        out
    }
}

/// The interpolation space a `<color-space>` keyword names, and its serialized name.
fn space_named(name: &str) -> Option<(Space, &'static str)> {
    Some(match name.cow_to_ascii_lowercase().as_ref() {
        "srgb" => (Space::Srgb, "srgb"),
        "srgb-linear" => (Space::SrgbLinear, "srgb-linear"),
        "display-p3" => (Space::DisplayP3, "display-p3"),
        "display-p3-linear" => (Space::DisplayP3Linear, "display-p3-linear"),
        "a98-rgb" => (Space::A98Rgb, "a98-rgb"),
        "prophoto-rgb" => (Space::ProphotoRgb, "prophoto-rgb"),
        "rec2020" => (Space::Rec2020, "rec2020"),
        "lab" => (Space::Lab, "lab"),
        "oklab" => (Space::Oklab, "oklab"),
        "xyz" | "xyz-d65" => (Space::XyzD65, "xyz-d65"),
        "xyz-d50" => (Space::XyzD50, "xyz-d50"),
        "hsl" => (Space::Hsl, "hsl"),
        "hwb" => (Space::Hwb, "hwb"),
        "lch" => (Space::Lch, "lch"),
        "oklch" => (Space::Oklch, "oklch"),
        _ => return None,
    })
}

/// One `<color> && <percentage>?` argument.
struct Item<'a> {
    color: &'a CssValue,
    percentage: Option<&'a CssValue>,
}

/// Split the arguments into the method and the items, or `None` when they are not a valid
/// `color-mix()`.
fn split(args: &[CssValue]) -> Option<(Method, Vec<Item<'_>>)> {
    let mut segments: Vec<&[CssValue]> = args.split(|value| matches!(value, CssValue::Comma)).collect();
    if segments.iter().any(|segment| segment.is_empty()) {
        return None;
    }
    let starts_with_in = matches!(
        segments.first().and_then(|segment| segment.first()),
        Some(CssValue::String(word)) if word.eq_ignore_ascii_case("in")
    );
    let method = if starts_with_in {
        Method::parse(segments.remove(0))?
    } else {
        Method::DEFAULT
    };
    if segments.is_empty() {
        return None;
    }
    let items = segments.into_iter().map(item).collect::<Option<Vec<_>>>()?;
    Some((method, items))
}

fn item(segment: &[CssValue]) -> Option<Item<'_>> {
    match segment {
        [color] => Some(Item {
            color,
            percentage: None,
        }),
        [first, second] if is_percentage(second) => Some(Item {
            color: first,
            percentage: Some(second),
        }),
        [first, second] if is_percentage(first) => Some(Item {
            color: second,
            percentage: Some(first),
        }),
        _ => None,
    }
}

/// Whether a value is a `<percentage [0,100]>`. A math function is only checked for its type.
/// Its range is enforced when it is computed.
fn is_percentage(value: &CssValue) -> bool {
    match value {
        CssValue::Percentage(percentage) => (0.0..=100.0).contains(percentage),
        CssValue::Function(name, args) if calc::is_math_function_name(name) => {
            match calc::math_function_type(name, args, &calc::Units::none()) {
                calc::MathType::Resolved(kinds) => kinds.len() == 1 && kinds[0] == "percentage",
                calc::MathType::Unknown => true,
                calc::MathType::Invalid => false,
            }
        }
        _ => false,
    }
}

// --- parsing ----------------------------------------------------------------------------------

/// The canonical specified value of `color-mix(args)`, or `None` when it is not valid.
///
/// The default method is left out, each percentage follows its colour, and the percentages
/// are shown filled in when all of them are plain percentages. They are left out entirely when
/// every colour has an equal share of 100%.
#[must_use]
pub(crate) fn canonical(args: &[CssValue]) -> Option<CssValue> {
    let (method, items) = split(args)?;
    let colors = items
        .iter()
        .map(|item| canonical_origin(item.color))
        .collect::<Option<Vec<_>>>()?;

    let literal: Option<Vec<Option<f64>>> = items
        .iter()
        .map(|item| match item.percentage {
            None => Some(None),
            Some(CssValue::Percentage(value)) => Some(Some(*value)),
            Some(_) => None,
        })
        .collect();
    let percentages: Vec<Option<CssValue>> = match literal {
        Some(literal) => {
            let filled = fill_omitted(&literal);
            #[expect(clippy::cast_precision_loss, reason = "a handful of colours")]
            let share = 100.0 / filled.len() as f64;
            if filled.iter().all(|value| (value - share).abs() < 1e-9) {
                vec![None; filled.len()]
            } else {
                filled
                    .into_iter()
                    .map(|value| Some(CssValue::Percentage(value)))
                    .collect()
            }
        }
        // A math function stays as written, and so does everything around it.
        None => items.iter().map(|item| item.percentage.cloned()).collect(),
    };

    let mut out = method.canonical();
    for (color, percentage) in colors.into_iter().zip(percentages) {
        if !out.is_empty() {
            out.push(CssValue::Comma);
        }
        out.push(color);
        if let Some(percentage) = percentage {
            out.push(percentage);
        }
    }
    Some(CssValue::Function("color-mix".to_string(), out))
}

/// The percentages with each omitted one set to an equal share of what the given ones leave
/// over (css-values-5 "normalize mix percentages", steps 1 and 2).
fn fill_omitted(percentages: &[Option<f64>]) -> Vec<f64> {
    let specified: f64 = percentages.iter().flatten().sum::<f64>().min(100.0);
    let omitted = percentages.iter().filter(|value| value.is_none()).count();
    #[expect(clippy::cast_precision_loss, reason = "a handful of colours")]
    let share = if omitted == 0 {
        0.0
    } else {
        (100.0 - specified) / omitted as f64
    };
    percentages.iter().map(|value| value.unwrap_or(share)).collect()
}

// --- computing --------------------------------------------------------------------------------

/// A colour in the interpolation space: its components in that space's units and its alpha,
/// `None` where missing.
#[derive(Clone, Copy, Debug)]
struct Point {
    components: [Option<f64>; 3],
    alpha: Option<f64>,
}

/// The colour `color-mix(args)` computes to, or `None` when it cannot be worked out here.
#[must_use]
pub(crate) fn resolve(args: &[CssValue]) -> Option<CssColor> {
    let (method, items) = split(args)?;
    let mut points = Vec::with_capacity(items.len());
    let mut percentages = Vec::with_capacity(items.len());
    for item in &items {
        let color = resolve_origin(item.color)?;
        let mut components = color.in_space(method.space);
        if let Some(hue) = method.space.hue_index() {
            components[hue] = components[hue].map(normalize_hue);
        }
        points.push(Point {
            components,
            alpha: color.alpha(),
        });
        percentages.push(match item.percentage {
            None => None,
            Some(value) => Some(percentage_value(value)?),
        });
    }

    // css-values-5 "normalize mix percentages", with forced normalization.
    let mut percentages = fill_omitted(&percentages);
    let total: f64 = percentages.iter().sum();
    if total > 0.0 {
        for value in &mut percentages {
            *value *= 100.0 / total;
        }
    }
    let leftover = if total < 100.0 { 100.0 - total } else { 0.0 };
    let alpha_mult = 1.0 - leftover / 100.0;

    // Mix from the front: the first two colours, then that result with the third, and so on.
    let mut stack: Vec<(Point, f64)> = points.into_iter().zip(percentages).rev().collect();
    while stack.len() >= 2 {
        let (a, a_share) = stack.pop()?;
        let (b, b_share) = stack.pop()?;
        let combined = a_share + b_share;
        let progress = if combined == 0.0 { 0.5 } else { b_share / combined };
        stack.push((interpolate(a, b, progress, method), combined));
    }
    let (mut point, _) = stack.pop()?;
    point.alpha = point.alpha.map(|alpha| (alpha * alpha_mult).clamp(0.0, 1.0));
    Some(build(method.space, point))
}

/// A percentage argument as a number from 0 to 100, or `None` when it needs something this
/// stage does not know.
fn percentage_value(value: &CssValue) -> Option<f64> {
    let value = match value {
        CssValue::Function(name, args) => calc::evaluate_call(name, args, &calc::Units::none(), true)?,
        other => other.clone(),
    };
    match value {
        CssValue::Percentage(percentage) => Some(percentage.clamp(0.0, 100.0)),
        _ => None,
    }
}

/// Interpolate two colours at `progress` from `a` to `b`, with premultiplied alpha
/// (css-color-4 sections 13.3 to 13.5).
fn interpolate(a: Point, b: Point, progress: f64, method: Method) -> Point {
    let hue_index = method.space.hue_index();

    // A component missing on one side takes the other side's value.
    let fill = |x: Option<f64>, y: Option<f64>| (x.or(y), y.or(x));
    let (a_alpha, b_alpha) = fill(a.alpha, b.alpha);
    let mut a_parts = a.components;
    let mut b_parts = b.components;
    for index in 0..3 {
        (a_parts[index], b_parts[index]) = fill(a.components[index], b.components[index]);
    }

    // Premultiply every component but the hue.
    let premultiply = |parts: &mut [Option<f64>; 3], alpha: Option<f64>| {
        if let Some(alpha) = alpha {
            for (index, part) in parts.iter_mut().enumerate() {
                if Some(index) != hue_index {
                    *part = part.map(|value| value * alpha);
                }
            }
        }
    };
    premultiply(&mut a_parts, a_alpha);
    premultiply(&mut b_parts, b_alpha);

    if let Some(hue) = hue_index {
        if let (Some(first), Some(second)) = (a_parts[hue], b_parts[hue]) {
            let (first, second) = fix_up_hues(first, second, method.hue);
            a_parts[hue] = Some(first);
            b_parts[hue] = Some(second);
        }
    }

    let lerp = |x: Option<f64>, y: Option<f64>| match (x, y) {
        (Some(x), Some(y)) => Some(x + (y - x) * progress),
        _ => None,
    };
    let alpha = lerp(a_alpha, b_alpha);
    let mut components = [
        lerp(a_parts[0], b_parts[0]),
        lerp(a_parts[1], b_parts[1]),
        lerp(a_parts[2], b_parts[2]),
    ];

    // Undo the premultiplication. With an alpha of zero or none there is nothing to undo.
    if let Some(alpha) = alpha.filter(|alpha| *alpha != 0.0) {
        for (index, part) in components.iter_mut().enumerate() {
            if Some(index) != hue_index {
                *part = part.map(|value| value / alpha);
            }
        }
    }
    if let Some(hue) = hue_index {
        components[hue] = components[hue].map(normalize_hue);
    }
    Point { components, alpha }
}

/// Adjust two hues in `[0, 360)` so that plain interpolation between them follows `method`.
fn fix_up_hues(mut first: f64, mut second: f64, method: HueMethod) -> (f64, f64) {
    let delta = second - first;
    match method {
        HueMethod::Shorter => {
            if delta > 180.0 {
                first += 360.0;
            } else if delta < -180.0 {
                second += 360.0;
            }
        }
        HueMethod::Longer => {
            if 0.0 < delta && delta < 180.0 {
                first += 360.0;
            } else if -180.0 < delta && delta <= 0.0 {
                second += 360.0;
            }
        }
        HueMethod::Increasing => {
            if second < first {
                second += 360.0;
            }
        }
        HueMethod::Decreasing => {
            if first < second {
                first += 360.0;
            }
        }
    }
    (first, second)
}

/// The mixed colour as a value in the interpolation space.
///
/// A mix in sRGB, HSL or HWB computes to `color(srgb ...)`. An HSL or HWB result that still has
/// a missing component keeps its own notation, since converting it would drop the `none`.
fn build(space: Space, point: Point) -> CssColor {
    let srgb = ColorSyntax::Predefined(PredefinedSpace::Srgb);
    let syntax = match space {
        Space::Srgb => srgb,
        Space::SrgbLinear => ColorSyntax::Predefined(PredefinedSpace::SrgbLinear),
        Space::DisplayP3 => ColorSyntax::Predefined(PredefinedSpace::DisplayP3),
        Space::DisplayP3Linear => ColorSyntax::Predefined(PredefinedSpace::DisplayP3Linear),
        Space::A98Rgb => ColorSyntax::Predefined(PredefinedSpace::A98Rgb),
        Space::ProphotoRgb => ColorSyntax::Predefined(PredefinedSpace::ProphotoRgb),
        Space::Rec2020 => ColorSyntax::Predefined(PredefinedSpace::Rec2020),
        Space::XyzD50 => ColorSyntax::Predefined(PredefinedSpace::XyzD50),
        Space::XyzD65 => ColorSyntax::Predefined(PredefinedSpace::XyzD65),
        Space::Lab => ColorSyntax::Lab,
        Space::Lch => ColorSyntax::Lch,
        Space::Oklab => ColorSyntax::Oklab,
        Space::Oklch => ColorSyntax::Oklch,
        Space::Hsl | Space::Hwb => {
            let complete = point.components.iter().all(Option::is_some) && point.alpha.is_some();
            if complete {
                let values = point.components.map(|c| c.unwrap_or(0.0));
                let converted = space::convert(space, Space::Srgb, values);
                return CssColor::from_parts(srgb, converted.map(Some), point.alpha, false);
            }
            if space == Space::Hsl {
                ColorSyntax::Hsl
            } else {
                ColorSyntax::Hwb
            }
        }
    };
    CssColor::from_parts(syntax, point.components, point.alpha, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokenizer::NumberKind;

    fn word(text: &str) -> CssValue {
        CssValue::String(text.to_string())
    }

    fn args(text: &[&str]) -> Vec<CssValue> {
        text.iter()
            .map(|token| match *token {
                "," => CssValue::Comma,
                t if t.ends_with('%') => CssValue::Percentage(t.trim_end_matches('%').parse().unwrap_or(0.0)),
                t => t
                    .parse::<f64>()
                    .map_or_else(|_| word(t), |number| CssValue::Number(number, NumberKind::Number)),
            })
            .collect()
    }

    fn mixed(tokens: &[&str]) -> String {
        resolve(&args(tokens))
            .map(|color| color.to_string())
            .unwrap_or_default()
    }

    #[test]
    fn two_colours_mix_halfway_by_default() {
        assert_eq!(mixed(&["in", "srgb", ",", "red", ",", "blue"]), "color(srgb 0.5 0 0.5)");
    }

    #[test]
    fn percentages_weight_the_mix_and_a_short_sum_lowers_alpha() {
        assert_eq!(
            mixed(&["in", "srgb", ",", "red", "10%", ",", "blue", "50%"]),
            "color(srgb 0.16666667 0 0.83333333 / 0.6)"
        );
    }

    #[test]
    fn transparent_does_not_darken_the_mix() {
        // Premultiplied: transparent black adds no colour, only transparency.
        assert_eq!(
            mixed(&["in", "srgb", ",", "red", ",", "transparent"]),
            "color(srgb 1 0 0 / 0.5)"
        );
    }

    #[test]
    fn hue_methods_choose_the_arc() {
        assert_eq!(fix_up_hues(40.0, 60.0, HueMethod::Shorter), (40.0, 60.0));
        assert_eq!(fix_up_hues(50.0, 330.0, HueMethod::Shorter), (410.0, 330.0));
        assert_eq!(fix_up_hues(40.0, 60.0, HueMethod::Longer), (400.0, 60.0));
        assert_eq!(fix_up_hues(60.0, 40.0, HueMethod::Increasing), (60.0, 400.0));
        assert_eq!(fix_up_hues(40.0, 60.0, HueMethod::Decreasing), (400.0, 60.0));
    }

    #[test]
    fn more_than_two_colours_mix_in_order() {
        assert_eq!(
            mixed(&["in", "srgb", ",", "red", ",", "lime", ",", "blue"]),
            "color(srgb 0.33333333 0.33333333 0.33333333)"
        );
    }

    #[test]
    fn a_component_missing_on_both_sides_stays_missing() {
        let none = CssColor::from_parts(ColorSyntax::Lab, [Some(10.0), None, Some(30.0)], Some(1.0), false);
        let other = CssColor::from_parts(ColorSyntax::Lab, [Some(50.0), None, Some(70.0)], Some(1.0), false);
        let tokens = vec![
            word("in"),
            word("lab"),
            CssValue::Comma,
            CssValue::Color(none),
            CssValue::Comma,
            CssValue::Color(other),
        ];
        assert_eq!(
            resolve(&tokens).map(|c| c.to_string()).unwrap_or_default(),
            "lab(30 none 50)"
        );
    }

    #[test]
    fn the_specified_form_drops_defaults_and_fills_percentages() {
        let value = canonical(&args(&["in", "oklab", ",", "25%", "red", ",", "blue"])).expect("valid");
        assert_eq!(value.to_string(), "color-mix(red 25%, blue 75%)");
        let value = canonical(&args(&["in", "hsl", "shorter", "hue", ",", "red", "50%", ",", "blue"])).expect("valid");
        assert_eq!(value.to_string(), "color-mix(in hsl, red, blue)");
    }

    #[test]
    fn invalid_forms_are_rejected() {
        for tokens in [
            &["in", "hsl", "hue", ",", "red", ",", "blue"][..],
            &["in", "srgb", "longer", "hue", ",", "red", ",", "blue"],
            &["in", "srgb", ",", "red", "150%", ",", "blue"],
            &["in", "srgb", ",", "red", ",", "blue", "blue"],
            &["red", ",", "blue", ",", "in", "srgb"],
        ] {
            assert_eq!(canonical(&args(tokens)), None, "{tokens:?}");
        }
    }
}
