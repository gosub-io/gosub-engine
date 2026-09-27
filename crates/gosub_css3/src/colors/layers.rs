//! `color-layers()` (css-color-6 section 3).
//!
//! `color-layers(multiply, red, rgb(0 0 255 / 50%))` composites a list of colours with the
//! source-over operator and an optional blend mode. The first colour is the top layer, as with
//! background layers. With no blend mode, `normal` is used.
//!
//! The specified value keeps the function, and the syntax matcher calls [`canonical`]. The
//! computed value is the composited colour. The computed stage calls [`resolve`], through
//! `fold_color_function`. When a layer cannot be resolved here (`currentcolor`, a system colour),
//! the function is left as written.
//!
//! Compositing and blending follow Compositing and Blending Level 1, in gamma-encoded sRGB.
//! A layer outside the sRGB gamut is clipped to it first. The result computes to
//! `color(srgb ...)`.

use cow_utils::CowUtils;

use super::relative::{canonical_origin, resolve_origin};
use super::space::Space;
use super::{ColorSyntax, CssColor, PredefinedSpace};
use crate::stylesheet::CssValue;

/// Whether `name` is `color-layers`.
#[must_use]
pub(crate) fn is_color_layers(name: &str) -> bool {
    name.eq_ignore_ascii_case("color-layers")
}

/// A `<blend-mode>` (Compositing and Blending Level 1, section 5.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BlendMode {
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
}

impl BlendMode {
    fn from_name(name: &str) -> Option<Self> {
        Some(match name.cow_to_ascii_lowercase().as_ref() {
            "normal" => BlendMode::Normal,
            "multiply" => BlendMode::Multiply,
            "screen" => BlendMode::Screen,
            "overlay" => BlendMode::Overlay,
            "darken" => BlendMode::Darken,
            "lighten" => BlendMode::Lighten,
            "color-dodge" => BlendMode::ColorDodge,
            "color-burn" => BlendMode::ColorBurn,
            "hard-light" => BlendMode::HardLight,
            "soft-light" => BlendMode::SoftLight,
            "difference" => BlendMode::Difference,
            "exclusion" => BlendMode::Exclusion,
            "hue" => BlendMode::Hue,
            "saturation" => BlendMode::Saturation,
            "color" => BlendMode::Color,
            "luminosity" => BlendMode::Luminosity,
            _ => return None,
        })
    }

    fn name(self) -> &'static str {
        match self {
            BlendMode::Normal => "normal",
            BlendMode::Multiply => "multiply",
            BlendMode::Screen => "screen",
            BlendMode::Overlay => "overlay",
            BlendMode::Darken => "darken",
            BlendMode::Lighten => "lighten",
            BlendMode::ColorDodge => "color-dodge",
            BlendMode::ColorBurn => "color-burn",
            BlendMode::HardLight => "hard-light",
            BlendMode::SoftLight => "soft-light",
            BlendMode::Difference => "difference",
            BlendMode::Exclusion => "exclusion",
            BlendMode::Hue => "hue",
            BlendMode::Saturation => "saturation",
            BlendMode::Color => "color",
            BlendMode::Luminosity => "luminosity",
        }
    }

    /// The blended colour B(Cb, Cs) of a backdrop and a source, both sRGB in `[0, 1]`.
    fn blend(self, backdrop: [f64; 3], source: [f64; 3]) -> [f64; 3] {
        let each = |f: fn(f64, f64) -> f64| {
            [
                f(backdrop[0], source[0]),
                f(backdrop[1], source[1]),
                f(backdrop[2], source[2]),
            ]
        };
        match self {
            BlendMode::Normal => source,
            BlendMode::Multiply => each(multiply),
            BlendMode::Screen => each(screen),
            BlendMode::Overlay => each(|cb, cs| hard_light(cs, cb)),
            BlendMode::Darken => each(f64::min),
            BlendMode::Lighten => each(f64::max),
            BlendMode::ColorDodge => each(color_dodge),
            BlendMode::ColorBurn => each(color_burn),
            BlendMode::HardLight => each(hard_light),
            BlendMode::SoftLight => each(soft_light),
            BlendMode::Difference => each(|cb, cs| (cb - cs).abs()),
            BlendMode::Exclusion => each(|cb, cs| cb + cs - 2.0 * cb * cs),
            BlendMode::Hue => set_lum(set_sat(source, sat(backdrop)), lum(backdrop)),
            BlendMode::Saturation => set_lum(set_sat(backdrop, sat(source)), lum(backdrop)),
            BlendMode::Color => set_lum(source, lum(backdrop)),
            BlendMode::Luminosity => set_lum(backdrop, lum(source)),
        }
    }
}

fn multiply(cb: f64, cs: f64) -> f64 {
    cb * cs
}

fn screen(cb: f64, cs: f64) -> f64 {
    cb + cs - cb * cs
}

fn hard_light(cb: f64, cs: f64) -> f64 {
    if cs <= 0.5 {
        multiply(cb, 2.0 * cs)
    } else {
        screen(cb, 2.0 * cs - 1.0)
    }
}

fn color_dodge(cb: f64, cs: f64) -> f64 {
    if cb == 0.0 {
        0.0
    } else if cs >= 1.0 {
        1.0
    } else {
        (cb / (1.0 - cs)).min(1.0)
    }
}

fn color_burn(cb: f64, cs: f64) -> f64 {
    if cb >= 1.0 {
        1.0
    } else if cs <= 0.0 {
        0.0
    } else {
        1.0 - ((1.0 - cb) / cs).min(1.0)
    }
}

fn soft_light(cb: f64, cs: f64) -> f64 {
    if cs <= 0.5 {
        cb - (1.0 - 2.0 * cs) * cb * (1.0 - cb)
    } else {
        let d = if cb <= 0.25 {
            ((16.0 * cb - 12.0) * cb + 4.0) * cb
        } else {
            cb.sqrt()
        };
        cb + (2.0 * cs - 1.0) * (d - cb)
    }
}

// The non-separable blend modes (Compositing and Blending Level 1, section 5.9).

fn lum(c: [f64; 3]) -> f64 {
    0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]
}

fn clip_color(c: [f64; 3]) -> [f64; 3] {
    let l = lum(c);
    let n = c[0].min(c[1]).min(c[2]);
    let x = c[0].max(c[1]).max(c[2]);
    let mut out = c;
    if n < 0.0 {
        out = out.map(|v| l + (v - l) * l / (l - n));
    }
    if x > 1.0 {
        out = out.map(|v| l + (v - l) * (1.0 - l) / (x - l));
    }
    out
}

fn set_lum(c: [f64; 3], l: f64) -> [f64; 3] {
    let d = l - lum(c);
    clip_color(c.map(|v| v + d))
}

fn sat(c: [f64; 3]) -> f64 {
    c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2])
}

fn set_sat(c: [f64; 3], s: f64) -> [f64; 3] {
    let max = c[0].max(c[1]).max(c[2]);
    let min = c[0].min(c[1]).min(c[2]);
    if max <= min {
        return [0.0; 3];
    }
    c.map(|v| {
        if v >= max {
            s
        } else if v <= min {
            0.0
        } else {
            (v - min) * s / (max - min)
        }
    })
}

/// Split the arguments into the blend mode and the layers, top layer first. `None` when they are
/// not a valid `color-layers()`.
fn split(args: &[CssValue]) -> Option<(BlendMode, Vec<&CssValue>)> {
    let mut segments: Vec<&[CssValue]> = args.split(|value| matches!(value, CssValue::Comma)).collect();
    let mut mode = BlendMode::Normal;
    if let Some([CssValue::String(word)]) = segments.first() {
        if let Some(named) = BlendMode::from_name(word) {
            mode = named;
            segments.remove(0);
        }
    }
    if segments.is_empty() {
        return None;
    }
    let layers = segments
        .into_iter()
        .map(|segment| match segment {
            [color] => Some(color),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()?;
    Some((mode, layers))
}

/// The canonical specified value of `color-layers(args)`, or `None` when it is not valid.
/// A `normal` blend mode is left out.
#[must_use]
pub(crate) fn canonical(args: &[CssValue]) -> Option<CssValue> {
    let (mode, layers) = split(args)?;
    let mut out = Vec::with_capacity(args.len());
    if mode != BlendMode::Normal {
        out.push(CssValue::String(mode.name().to_string()));
    }
    for layer in layers {
        if !out.is_empty() {
            out.push(CssValue::Comma);
        }
        out.push(canonical_origin(layer)?);
    }
    Some(CssValue::Function("color-layers".to_string(), out))
}

/// The colour `color-layers(args)` computes to, or `None` when it cannot be worked out here.
#[must_use]
pub(crate) fn resolve(args: &[CssValue]) -> Option<CssColor> {
    let (mode, layers) = split(args)?;
    let mut colors = Vec::with_capacity(layers.len());
    for layer in layers {
        let color = resolve_origin(layer)?;
        let rgb = color.in_space(Space::Srgb).map(|c| c.unwrap_or(0.0).clamp(0.0, 1.0));
        colors.push((rgb, color.alpha().unwrap_or(0.0).clamp(0.0, 1.0)));
    }

    // Start from the bottom layer and put each layer above it over what is there so far.
    let (mut backdrop, mut backdrop_alpha) = colors.pop()?;
    while let Some((source, source_alpha)) = colors.pop() {
        (backdrop, backdrop_alpha) = source_over(mode, source, source_alpha, backdrop, backdrop_alpha);
    }
    Some(CssColor::from_parts(
        ColorSyntax::Predefined(PredefinedSpace::Srgb),
        backdrop.map(Some),
        Some(backdrop_alpha),
        false,
    ))
}

/// One source layer blended and composited over a backdrop (Compositing and Blending Level 1,
/// sections 5.1 and 9.1.4). Colours are not premultiplied on the way in or out.
fn source_over(
    mode: BlendMode,
    source: [f64; 3],
    source_alpha: f64,
    backdrop: [f64; 3],
    backdrop_alpha: f64,
) -> ([f64; 3], f64) {
    let blended = mode.blend(backdrop, source);
    let alpha = source_alpha + backdrop_alpha * (1.0 - source_alpha);
    if alpha <= 0.0 {
        return ([0.0; 3], 0.0);
    }
    let mut out = [0.0; 3];
    for index in 0..3 {
        let mixed = (1.0 - backdrop_alpha) * source[index] + backdrop_alpha * blended[index];
        let premultiplied = source_alpha * mixed + (1.0 - source_alpha) * backdrop_alpha * backdrop[index];
        out[index] = premultiplied / alpha;
    }
    (out, alpha)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(text: &str) -> CssValue {
        CssValue::String(text.to_string())
    }

    fn list(tokens: &[&str]) -> Vec<CssValue> {
        tokens
            .iter()
            .map(|token| if *token == "," { CssValue::Comma } else { word(token) })
            .collect()
    }

    fn resolved(tokens: &[&str]) -> String {
        resolve(&list(tokens))
            .map(|color| color.to_string())
            .unwrap_or_default()
    }

    fn translucent(r: f64, g: f64, b: f64, a: f64) -> CssValue {
        CssValue::Color(CssColor::from_parts(
            ColorSyntax::Predefined(PredefinedSpace::Srgb),
            [Some(r), Some(g), Some(b)],
            Some(a),
            false,
        ))
    }

    #[test]
    fn an_opaque_top_layer_hides_the_rest() {
        assert_eq!(resolved(&["red", ",", "blue"]), "color(srgb 1 0 0)");
        assert_eq!(resolved(&["red"]), "color(srgb 1 0 0)");
    }

    #[test]
    fn a_translucent_layer_is_composited_over_the_next() {
        let args = vec![translucent(1.0, 0.0, 0.0, 0.5), CssValue::Comma, word("blue")];
        assert_eq!(
            resolve(&args).map(|c| c.to_string()).unwrap_or_default(),
            "color(srgb 0.5 0 0.5)"
        );
        // Two half-transparent layers leave a quarter of the backdrop showing.
        let args = vec![
            translucent(1.0, 0.0, 0.0, 0.5),
            CssValue::Comma,
            translucent(0.0, 0.0, 1.0, 0.5),
        ];
        assert_eq!(
            resolve(&args).map(|c| c.to_string()).unwrap_or_default(),
            "color(srgb 0.66666667 0 0.33333333 / 0.75)"
        );
    }

    #[test]
    fn blend_modes_combine_opaque_layers() {
        assert_eq!(resolved(&["multiply", ",", "red", ",", "yellow"]), "color(srgb 1 0 0)");
        assert_eq!(resolved(&["screen", ",", "red", ",", "blue"]), "color(srgb 1 0 1)");
        assert_eq!(resolved(&["difference", ",", "white", ",", "red"]), "color(srgb 0 1 1)");
        assert_eq!(resolved(&["luminosity", ",", "black", ",", "red"]), "color(srgb 0 0 0)");
    }

    #[test]
    fn the_specified_form_leaves_out_normal() {
        let value = canonical(&list(&["normal", ",", "Red", ",", "blue"])).expect("valid");
        assert_eq!(value.to_string(), "color-layers(red, blue)");
        let value = canonical(&list(&["multiply", ",", "red"])).expect("valid");
        assert_eq!(value.to_string(), "color-layers(multiply, red)");
    }

    #[test]
    fn invalid_forms_are_rejected() {
        for tokens in [
            &["normal"][..],
            &["red", "blue"],
            &["plus-lighter", ",", "red", ",", "blue"],
            &["multiply", ",", "multiply", ",", "red"],
        ] {
            assert_eq!(canonical(&list(tokens)), None, "{tokens:?}");
        }
        assert_eq!(canonical(&[]), None);
    }
}
