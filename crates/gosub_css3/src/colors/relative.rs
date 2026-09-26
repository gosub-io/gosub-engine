//! Relative colour syntax (css-color-5 §4): `rgb(from <color> r g calc(b / 2))`.
//!
//! A colour function whose first argument is `from` takes an *origin* colour, converts it into
//! the function's own space, and exposes that colour's components as keywords: `r g b` for
//! `rgb()`, `h s l` for `hsl()`, `l c h` for `lch()`, `x y z` for an XYZ `color()`, and `alpha`
//! in all of them. Each component of the new colour is then a number, a percentage, `none`, one
//! of those keywords, or a math function over them.
//!
//! * **Parsing** checks the function and puts it in canonical form, but keeps it as written:
//!   the specified value serializes as the function, keywords included (css-color-5 §4.1 and the
//!   "serializing relative colors" section). The syntax matcher calls [`canonical`].
//! * **Computing** resolves the origin, converts it and evaluates each component. The computed
//!   stage calls [`resolve`], through `fold_color_function`.
//!
//! Two origins cannot be resolved here and are left as written: `currentcolor`, because the
//! computed stage does not pass in the element's `color`, and system colours, whose values live
//! in the render pipeline.

use cow_utils::CowUtils;

use super::space::{self, normalize_hue, Analogy, Space};
use super::{is_named_color, is_system_color, ColorSyntax, CssColor, PredefinedSpace, RgbColor};
use crate::functions::calc;
use crate::matcher::property_definitions::get_css_definitions;
use crate::stylesheet::{clamp_alpha, color_component, color_hue, fold_color_function, is_color_function, CssValue};
use crate::tokenizer::NumberKind;

/// Whether a colour function's arguments are the relative form: they start with `from`.
#[must_use]
pub(crate) fn is_relative(args: &[CssValue]) -> bool {
    matches!(args.first(), Some(CssValue::String(word)) if word.eq_ignore_ascii_case("from"))
}

/// What a component slot holds, which decides the units a literal is read in.
#[derive(Clone, Copy, PartialEq)]
enum Slot {
    /// A number, or a percentage of `scale`.
    Plain(f64),
    /// A hue: a number of degrees or an angle.
    Hue,
}

/// The target notation of a relative colour: its syntax, its component keywords and what each
/// slot takes.
struct Target {
    syntax: ColorSyntax,
    keywords: [&'static str; 3],
    slots: [Slot; 3],
}

impl Target {
    /// The target a colour function names, given the space keyword for `color()`.
    fn of(name: &str, space: Option<PredefinedSpace>) -> Option<Self> {
        let (syntax, keywords, slots) = match name {
            "rgb" | "rgba" => (
                ColorSyntax::Rgb,
                ["r", "g", "b"],
                [Slot::Plain(255.0), Slot::Plain(255.0), Slot::Plain(255.0)],
            ),
            "hsl" | "hsla" => (
                ColorSyntax::Hsl,
                ["h", "s", "l"],
                [Slot::Hue, Slot::Plain(100.0), Slot::Plain(100.0)],
            ),
            "hwb" => (
                ColorSyntax::Hwb,
                ["h", "w", "b"],
                [Slot::Hue, Slot::Plain(100.0), Slot::Plain(100.0)],
            ),
            "lab" => (
                ColorSyntax::Lab,
                ["l", "a", "b"],
                [Slot::Plain(100.0), Slot::Plain(125.0), Slot::Plain(125.0)],
            ),
            "lch" => (
                ColorSyntax::Lch,
                ["l", "c", "h"],
                [Slot::Plain(100.0), Slot::Plain(150.0), Slot::Hue],
            ),
            "oklab" => (
                ColorSyntax::Oklab,
                ["l", "a", "b"],
                [Slot::Plain(1.0), Slot::Plain(0.4), Slot::Plain(0.4)],
            ),
            "oklch" => (
                ColorSyntax::Oklch,
                ["l", "c", "h"],
                [Slot::Plain(1.0), Slot::Plain(0.4), Slot::Hue],
            ),
            // `alpha()` stays in the origin's space, which is only known once the origin is
            // resolved; all it names is the alpha channel.
            "alpha" => (
                ColorSyntax::Rgb,
                ["alpha", "alpha", "alpha"],
                [Slot::Plain(1.0), Slot::Plain(1.0), Slot::Plain(1.0)],
            ),
            "color" => {
                let space = space?;
                let keywords = if space.is_xyz() {
                    ["x", "y", "z"]
                } else {
                    ["r", "g", "b"]
                };
                (
                    ColorSyntax::Predefined(space),
                    keywords,
                    [Slot::Plain(1.0), Slot::Plain(1.0), Slot::Plain(1.0)],
                )
            }
            _ => return None,
        };
        Some(Self {
            syntax,
            keywords,
            slots,
        })
    }

    /// Whether `word` is one of this target's keywords, `alpha` included.
    fn is_keyword(&self, word: &str) -> bool {
        word.eq_ignore_ascii_case("alpha") || self.keywords.iter().any(|keyword| word.eq_ignore_ascii_case(keyword))
    }
}

/// The pieces of a relative colour function: the origin, the target, the three components and
/// the alpha, if one was written.
struct Parts<'a> {
    name: String,
    origin: &'a CssValue,
    space: Option<(PredefinedSpace, &'a CssValue)>,
    target: Target,
    /// The three components, or `None` for `alpha()`, which has none.
    components: Option<[&'a CssValue; 3]>,
    alpha: Option<&'a CssValue>,
}

fn split<'a>(name: &str, args: &'a [CssValue]) -> Option<Parts<'a>> {
    if !is_relative(args) {
        return None;
    }
    let name = name.cow_to_ascii_lowercase().into_owned();
    if !is_color_function(&name) && name != "alpha" {
        return None;
    }
    let origin = args.get(1)?;
    let (space, rest) = if name == "color" {
        let word = args.get(2)?;
        let CssValue::String(word_text) = word else {
            return None;
        };
        (Some((PredefinedSpace::from_name(word_text)?, word)), &args[3..])
    } else {
        (None, &args[2..])
    };
    let target = Target::of(&name, space.map(|(space, _)| space))?;

    // The relative form is the modern one: no commas, and the alpha after a solidus.
    let solidus = rest
        .iter()
        .position(|value| matches!(value, CssValue::String(word) if word == "/"));
    let (components, alpha) = match solidus {
        Some(at) => {
            if rest.len() != at + 2 {
                return None;
            }
            (&rest[..at], Some(&rest[at + 1]))
        }
        None => (rest, None),
    };
    let components = if name == "alpha" {
        // `alpha()` requires an alpha.
        if !components.is_empty() || alpha.is_none() {
            return None;
        }
        None
    } else {
        let [first, second, third] = components else {
            return None;
        };
        Some([first, second, third])
    };
    Some(Parts {
        name,
        origin,
        space,
        target,
        components,
        alpha,
    })
}

// --- parsing ----------------------------------------------------------------------------------

/// The canonical specified value of a relative colour function, or `None` when it is not one.
///
/// Canonical means the function's modern name (`rgba` is `rgb`, `hsla` is `hsl`), keywords in
/// lowercase, an origin in its own canonical form, and `xyz` spelled `xyz-d65` - the two name
/// the same space, and css-color-4 serializes the longer one.
#[must_use]
pub(crate) fn canonical(name: &str, args: &[CssValue]) -> Option<CssValue> {
    let parts = split(name, args)?;
    let name = match parts.name.as_str() {
        "rgba" => "rgb".to_string(),
        "hsla" => "hsl".to_string(),
        other => other.to_string(),
    };

    let mut out = Vec::with_capacity(args.len());
    out.push(CssValue::String("from".to_string()));
    out.push(canonical_origin(parts.origin)?);
    if let Some((space, _)) = parts.space {
        out.push(CssValue::String(space.name().to_string()));
    }
    for (value, slot) in parts.components.iter().flatten().zip(parts.target.slots) {
        out.push(canonical_component(value, slot, &parts.target)?);
    }
    if let Some(alpha) = parts.alpha {
        out.push(CssValue::String("/".to_string()));
        out.push(canonical_component(alpha, Slot::Plain(1.0), &parts.target)?);
    }
    Some(CssValue::Function(name, out))
}

/// The origin, checked against `<color>` and put in its canonical form.
fn canonical_origin(origin: &CssValue) -> Option<CssValue> {
    // A CSS-wide keyword is valid as a whole declaration, never as a colour inside one.
    if let CssValue::String(word) = origin {
        let word = word.cow_to_ascii_lowercase();
        if matches!(
            word.as_ref(),
            "inherit" | "initial" | "unset" | "revert" | "revert-layer" | "default"
        ) {
            return None;
        }
    }
    // `<color>` is the whole grammar of the `color` property, so matching the origin against
    // that property matches it against the type, including nested relative colours and
    // `light-dark()`.
    let color = get_css_definitions().find_property("color")?;
    let mut values = color.canonical(std::slice::from_ref(origin))?;
    if values.len() != 1 {
        return None;
    }
    // A colour keyword is ASCII case-insensitive and serializes in lowercase.
    Some(match values.remove(0) {
        CssValue::String(word) => CssValue::String(word.cow_to_ascii_lowercase().into_owned()),
        other => other,
    })
}

/// One component, checked for its slot and put in canonical form.
fn canonical_component(value: &CssValue, slot: Slot, target: &Target) -> Option<CssValue> {
    match value {
        CssValue::String(word) if word.eq_ignore_ascii_case("none") => Some(CssValue::String("none".to_string())),
        CssValue::String(word) if target.is_keyword(word) => {
            Some(CssValue::String(word.cow_to_ascii_lowercase().into_owned()))
        }
        CssValue::Number(..) | CssValue::Zero => Some(value.clone()),
        // A hue is a number or an angle; `hsl(from red 10% s l)` is not a colour.
        CssValue::Percentage(_) if slot != Slot::Hue => Some(value.clone()),
        CssValue::Unit(_, unit) if slot == Slot::Hue && is_angle(unit) => Some(value.clone()),
        // The tree-counting functions (css-values-5 §8) are integers, which fit any slot, and
        // take no arguments.
        CssValue::Function(name, args) if is_tree_counting(name) && args.is_empty() => Some(value.clone()),
        CssValue::Function(name, args) if calc::is_math_function_name(name) => {
            // The keywords stand for numbers, so an expression over them is typed as though
            // they were numbers. The expression itself is kept as written.
            let typed = substitute(args, target, &|_| 1.0);
            let accepted = match calc::math_function_type(name, &typed, &calc::Units::none()) {
                calc::MathType::Resolved(kinds) => kinds.len() == 1 && slot_accepts(slot, kinds[0]),
                // Accept a function that cannot be evaluated yet.
                calc::MathType::Unknown => true,
                calc::MathType::Invalid => false,
            };
            accepted.then(|| CssValue::Function(name.clone(), lowercase_keywords(args, target)))
        }
        _ => None,
    }
}

fn is_tree_counting(name: &str) -> bool {
    name.eq_ignore_ascii_case("sibling-index") || name.eq_ignore_ascii_case("sibling-count")
}

fn is_angle(unit: &str) -> bool {
    matches!(unit.cow_to_ascii_lowercase().as_ref(), "deg" | "grad" | "rad" | "turn")
}

fn slot_accepts(slot: Slot, kind: &str) -> bool {
    match slot {
        Slot::Plain(_) => kind == "number" || kind == "percentage",
        Slot::Hue => kind == "number" || kind == "angle",
    }
}

/// `args` with every channel keyword replaced by the number `value` gives for it.
fn substitute(args: &[CssValue], target: &Target, value: &dyn Fn(&str) -> f64) -> Vec<CssValue> {
    args.iter()
        .map(|arg| match arg {
            CssValue::String(word) if target.is_keyword(word) => {
                CssValue::Number(value(&word.cow_to_ascii_lowercase()), NumberKind::Number)
            }
            CssValue::Function(name, inner) => CssValue::Function(name.clone(), substitute(inner, target, value)),
            other => other.clone(),
        })
        .collect()
}

fn lowercase_keywords(args: &[CssValue], target: &Target) -> Vec<CssValue> {
    args.iter()
        .map(|arg| match arg {
            CssValue::String(word) if target.is_keyword(word) => {
                CssValue::String(word.cow_to_ascii_lowercase().into_owned())
            }
            CssValue::Function(name, inner) => CssValue::Function(name.clone(), lowercase_keywords(inner, target)),
            other => other.clone(),
        })
        .collect()
}

// --- computing --------------------------------------------------------------------------------

/// The colour a relative colour function computes to, or `None` when it cannot be worked out
/// here (see the module documentation) or is not a relative colour at all.
#[must_use]
pub(crate) fn resolve(name: &str, args: &[CssValue]) -> Option<CssColor> {
    let parts = split(name, args)?;
    let origin = resolve_origin(parts.origin)?;
    let Some(written) = parts.components else {
        return with_alpha(origin, parts.alpha, &parts.target);
    };
    let target = &parts.target;
    let (channels, origin_alpha) = channels_of(&origin, target.syntax);

    let lookup = |word: &str| -> Option<f64> {
        if word == "alpha" {
            return origin_alpha;
        }
        let index = target.keywords.iter().position(|keyword| *keyword == word)?;
        channels[index]
    };

    let mut components = [None; 3];
    for (index, (value, slot)) in written.iter().zip(target.slots).enumerate() {
        components[index] = evaluate(value, slot, target, &lookup)?;
    }
    let alpha = match parts.alpha {
        Some(value) => evaluate(value, Slot::Plain(1.0), target, &lookup)?,
        None => origin_alpha,
    };
    Some(build(target.syntax, components, alpha))
}

/// `alpha(from <color> / <alpha>)`: the origin, unchanged but for its alpha, in its own space.
///
/// The sRGB notations have a computed form of `color(srgb ...)` here as they do for any other
/// relative colour, since the result is one.
///
/// `None` when the alpha cannot be evaluated here, in which case the function is left as
/// written, like any other relative colour that cannot be resolved yet.
fn with_alpha(origin: CssColor, alpha: Option<&CssValue>, target: &Target) -> Option<CssColor> {
    let origin_alpha = origin.alpha();
    let lookup = |_: &str| origin_alpha;
    let alpha = match alpha {
        Some(value) => evaluate(value, Slot::Plain(1.0), target, &lookup)?,
        None => origin_alpha,
    };
    let alpha = alpha.map(clamp_alpha);
    Some(match origin.syntax {
        ColorSyntax::Rgb | ColorSyntax::Hsl | ColorSyntax::Hwb => {
            let srgb = ColorSyntax::Predefined(PredefinedSpace::Srgb);
            let components = if origin.syntax == ColorSyntax::Rgb {
                origin.components().map(|c| c.map(|v| v / 255.0))
            } else {
                space::convert(origin.syntax.space(), Space::Srgb, origin.space_components()).map(Some)
            };
            CssColor::from_parts(srgb, components, alpha, false)
        }
        syntax => CssColor::from_parts(syntax, origin.components(), alpha, false),
    })
}

/// The origin as a colour, or `None` when it can only be resolved later (see the module docs).
fn resolve_origin(origin: &CssValue) -> Option<CssColor> {
    match origin {
        CssValue::Color(color) => Some(*color),
        CssValue::String(word) => {
            let lower = word.cow_to_ascii_lowercase();
            if lower == "currentcolor" || is_system_color(&lower) {
                return None;
            }
            if lower != "transparent" && !is_named_color(&lower) {
                return None;
            }
            RgbColor::try_from_str(&lower).map(CssColor::from)
        }
        CssValue::Function(name, args) => fold_color_function(name, args, true),
        _ => None,
    }
}

/// The origin's components in the target's space and units, `None` for a missing one, and its
/// alpha.
///
/// A component missing in the origin stays missing when the origin is already in the target's
/// space. Across spaces a missing component is read as zero (css-color-4 §12.2), and only the
/// hue of an achromatic result can come out missing: `lch(from black l c h)` has no hue.
fn channels_of(origin: &CssColor, target: ColorSyntax) -> ([Option<f64>; 3], Option<f64>) {
    let (from, to) = (origin.syntax.space(), target.space());
    // `rgb()` counts its channels in 255ths, every other sRGB notation in ones.
    let origin_scale = if origin.syntax == ColorSyntax::Rgb { 255.0 } else { 1.0 };
    let target_scale = if target == ColorSyntax::Rgb { 255.0 } else { 1.0 };

    let mut channels = if from == to {
        origin
            .components()
            .map(|component| component.map(|value| value / origin_scale))
    } else {
        let mut converted =
            space::convert(from, to, origin.space_components()).map(|value| (!value.is_nan()).then_some(value));
        // A component missing in the origin stays missing in the analogous component of the
        // target (css-color-5 §4, via css-color-4 §12.2), such as lightness in both spaces.
        let (from_kinds, to_kinds) = (from.component_kinds(), to.component_kinds());
        for (index, component) in origin.components().iter().enumerate() {
            let kind = from_kinds[index];
            if component.is_none() && kind != Analogy::None {
                for (slot, target_kind) in to_kinds.iter().enumerate() {
                    if *target_kind == kind {
                        converted[slot] = None;
                    }
                }
            }
        }
        converted
    };
    for channel in &mut channels {
        *channel = channel.map(|value| value * target_scale);
    }
    if let Some(hue) = to.hue_index() {
        channels[hue] = channels[hue].map(normalize_hue);
    }
    (channels, origin.alpha())
}

/// One component of the result: `Some(None)` for `none` or a keyword whose channel is missing,
/// `None` when the value cannot be evaluated.
fn evaluate(
    value: &CssValue,
    slot: Slot,
    target: &Target,
    lookup: &dyn Fn(&str) -> Option<f64>,
) -> Option<Option<f64>> {
    match value {
        CssValue::String(word) if word.eq_ignore_ascii_case("none") => Some(None),
        CssValue::String(word) if target.is_keyword(word) => Some(lookup(&word.cow_to_ascii_lowercase())),
        CssValue::Function(name, args) if calc::is_math_function_name(name) => {
            // Inside an expression a missing channel is read as zero.
            let args = substitute(args, target, &|word| lookup(word).unwrap_or(0.0));
            let reduced = calc::evaluate_call(name, &args, &calc::Units::none(), true)?;
            literal(&reduced, slot)
        }
        _ => literal(value, slot),
    }
}

fn literal(value: &CssValue, slot: Slot) -> Option<Option<f64>> {
    match slot {
        Slot::Plain(scale) => color_component(value, scale),
        Slot::Hue => color_hue(value),
    }
}

/// The resolved colour, in the form its computed value takes.
///
/// css-color-5 has the sRGB notations resolve to `color(srgb ...)`, because a relative colour
/// can fall outside sRGB and the legacy notations cannot express that. An `hsl()` or `hwb()`
/// result with a missing component keeps its notation, because converting would drop the
/// missing component.
fn build(syntax: ColorSyntax, mut components: [Option<f64>; 3], alpha: Option<f64>) -> CssColor {
    let alpha = alpha.map(clamp_alpha);
    if let Some(hue) = syntax.space().hue_index() {
        components[hue] = components[hue].map(normalize_hue);
    }
    match syntax {
        ColorSyntax::Lab | ColorSyntax::Lch => components[0] = components[0].map(|l| l.clamp(0.0, 100.0)),
        ColorSyntax::Oklab | ColorSyntax::Oklch => components[0] = components[0].map(|l| l.clamp(0.0, 1.0)),
        _ => {}
    }
    if matches!(syntax, ColorSyntax::Lch | ColorSyntax::Oklch) {
        components[1] = components[1].map(|chroma| chroma.max(0.0));
    }

    let srgb = ColorSyntax::Predefined(PredefinedSpace::Srgb);
    let color = match syntax {
        ColorSyntax::Rgb => CssColor::from_parts(srgb, components.map(|c| c.map(|v| v / 255.0)), alpha, false),
        ColorSyntax::Hsl | ColorSyntax::Hwb if components.iter().all(Option::is_some) && alpha.is_some() => {
            let values = components.map(|c| c.unwrap_or(0.0));
            let converted = space::convert(syntax.space(), Space::Srgb, values);
            CssColor::from_parts(srgb, converted.map(Some), alpha, false)
        }
        _ => CssColor::from_parts(syntax, components, alpha, false),
    };
    color
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(text: &str) -> CssValue {
        CssValue::String(text.to_string())
    }

    fn number(value: f64) -> CssValue {
        CssValue::Number(value, NumberKind::Number)
    }

    fn resolved(name: &str, args: &[CssValue]) -> String {
        resolve(name, args).map(|color| color.to_string()).unwrap_or_default()
    }

    #[test]
    fn an_unmodified_rgb_origin_resolves_to_srgb() {
        let args = [word("from"), word("rebeccapurple"), word("r"), word("g"), word("b")];
        assert_eq!(resolved("rgb", &args), "color(srgb 0.4 0.2 0.6)");
    }

    #[test]
    fn keywords_are_the_origin_in_the_target_space() {
        // rebeccapurple is hsl(270 50% 40%), so swapping s and l gives hsl(270 40% 50%).
        let args = [word("from"), word("rebeccapurple"), word("h"), word("l"), word("s")];
        assert_eq!(resolved("hsl", &args), "color(srgb 0.5 0.3 0.7)");
    }

    #[test]
    fn a_math_function_sees_the_channels_as_numbers() {
        let half_red = CssValue::Function("calc".to_string(), vec![word("r"), word("*"), number(0.5)]);
        let args = [word("from"), word("rebeccapurple"), half_red, word("g"), word("b")];
        assert_eq!(resolved("rgb", &args), "color(srgb 0.2 0.2 0.6)");
    }

    #[test]
    fn none_and_a_missing_origin_channel_stay_missing() {
        let args = [word("from"), word("rebeccapurple"), word("r"), word("g"), word("none")];
        assert_eq!(resolved("rgb", &args), "color(srgb 0.4 0.2 none)");

        let origin = CssColor::from_parts(ColorSyntax::Lab, [Some(25.0), None, Some(50.0)], Some(1.0), false);
        let args = [word("from"), CssValue::Color(origin), word("l"), word("a"), word("b")];
        assert_eq!(resolved("lab", &args), "lab(25 none 50)");
    }

    #[test]
    fn an_achromatic_colour_has_no_hue_in_a_polar_space() {
        let black = CssColor::from_parts(
            ColorSyntax::Predefined(PredefinedSpace::DisplayP3),
            [Some(0.0), Some(0.0), Some(0.0)],
            Some(1.0),
            false,
        );
        let args = [word("from"), CssValue::Color(black), word("l"), word("c"), word("h")];
        assert_eq!(resolved("lch", &args), "lch(0 0 none)");
    }

    #[test]
    fn color_function_targets_name_their_space() {
        let args = [
            word("from"),
            word("rebeccapurple"),
            word("xyz"),
            word("x"),
            word("y"),
            word("z"),
        ];
        let color = resolve("color", &args).expect("xyz is a colour space");
        assert_eq!(color.syntax, ColorSyntax::Predefined(PredefinedSpace::XyzD65));
    }

    #[test]
    fn out_of_gamut_results_are_kept() {
        let p3_green = CssColor::from_parts(
            ColorSyntax::Predefined(PredefinedSpace::DisplayP3),
            [Some(0.0), Some(1.0), Some(0.0)],
            Some(1.0),
            false,
        );
        let args = [word("from"), CssValue::Color(p3_green), word("r"), word("g"), word("b")];
        let color = resolve("rgb", &args).expect("resolves");
        let [r, g, b] = color.components().map(Option::unwrap_or_default);
        assert!((r + 0.5116).abs() < 1e-4 && (g - 1.01827).abs() < 1e-4 && (b + 0.31067).abs() < 1e-4);
    }

    #[test]
    fn the_wrong_keywords_are_rejected() {
        // `h` is not an `rgb()` channel, and the legacy comma form has no relative version.
        let args = [word("from"), word("red"), word("h"), word("g"), word("b")];
        assert_eq!(canonical("rgb", &args), None);
        let commas = [
            word("from"),
            word("red"),
            word("r"),
            CssValue::Comma,
            word("g"),
            CssValue::Comma,
            word("b"),
        ];
        assert_eq!(canonical("rgb", &commas), None);
    }

    #[test]
    fn the_specified_form_is_canonical_but_unresolved() {
        let args = [word("from"), word("RebeccaPurple"), word("R"), word("g"), word("b")];
        let value = canonical("rgba", &args).expect("valid");
        assert_eq!(value.to_string(), "rgb(from rebeccapurple r g b)");
    }
}
