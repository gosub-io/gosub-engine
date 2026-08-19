use crate::common::document::node::{AttrMap, ElementData, Node, NodeType};
use crate::common::document::style::{
    intern, lookup, BorderStyle, Display, FontWeight, NodeStyle, StyleProperty, TextAlign, TextWrap, Unit, Value,
};
use crate::painter::commands::color::Color;
use crate::painter::commands::gradient::{ColorStop, Gradient, LinearGradient, Tiling};
use cow_utils::CowUtils;
use gosub_interface::config::HasDocument;
use gosub_interface::css3::{CssOrigin, CssProperty, CssPropertyMap, CssSystem, CssValue};
use gosub_interface::document::Document as _;
use gosub_interface::node::NodeType as GosubNodeType;
use gosub_shared::node::NodeId;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;

// ── Bridge: CssProperty → Value ──────────────────────────────────────────────

/// `None` when the property carries no usable value (e.g. `CssValue::None`).
fn css_property_to_value<S: CssSystem>(p: &S::Property, prop: &StyleProperty) -> Option<Value> {
    match prop {
        // ── Color properties ───────────────────────────────────────────────
        StyleProperty::Color
        | StyleProperty::BackgroundColor
        | StyleProperty::BorderTopColor
        | StyleProperty::BorderRightColor
        | StyleProperty::BorderBottomColor
        | StyleProperty::BorderLeftColor => {
            if let Some(s) = p.as_string() {
                if let Some((r, g, b, a)) = css_system_color(s) {
                    return Some(Value::Color(r, g, b, a));
                }
            }
            // parse_color returns 0..255 range - matches Value::Color(u8, u8, u8, u8)
            let (r, g, b, a) = p.parse_color()?;
            Some(Value::Color(r as u8, g as u8, b as u8, a as u8))
        }

        // ── Display ────────────────────────────────────────────────────────
        StyleProperty::Display => {
            let s = p.as_string()?;
            let d = match s {
                "block" => Display::Block,
                "inline" => Display::Inline,
                "inline-block" => Display::InlineBlock,
                "none" => Display::None,
                "flex" => Display::Flex,
                "inline-flex" => Display::InlineFlex,
                "grid" => Display::Grid,
                "inline-grid" => Display::InlineGrid,
                "table" => Display::Table,
                "table-caption" => Display::TableCaption,
                "table-cell" => Display::TableCell,
                "table-footer-group" => Display::TableFooterGroup,
                "table-header-group" => Display::TableHeaderGroup,
                "table-row" => Display::TableRow,
                "table-row-group" => Display::TableRowGroup,
                "table-column" => Display::TableColumn,
                "table-column-group" => Display::TableColumnGroup,
                _ => Display::Block,
            };
            Some(Value::Display(d))
        }

        // ── FontWeight ─────────────────────────────────────────────────────
        StyleProperty::FontWeight => {
            let fw = if let Some(n) = p.as_number() {
                FontWeight::Number(n)
            } else {
                match p.as_string()? {
                    "bold" => FontWeight::Bold,
                    "bolder" => FontWeight::Bolder,
                    "lighter" => FontWeight::Lighter,
                    _ => FontWeight::Normal,
                }
            };
            Some(Value::FontWeight(fw))
        }

        // ── TextAlign ──────────────────────────────────────────────────────
        StyleProperty::TextAlign => {
            let ta = match p.as_string()? {
                "left" => TextAlign::Left,
                "right" => TextAlign::Right,
                // `-webkit-center` is what the HTML rendering spec's UA sheet
                // uses for `<caption>`; treat it as plain center.
                "center" | "-webkit-center" => TextAlign::Center,
                "justify" => TextAlign::Justify,
                "start" => TextAlign::Start,
                "end" => TextAlign::End,
                "match-parent" => TextAlign::MatchParent,
                "initial" => TextAlign::Initial,
                "inherit" => TextAlign::Inherit,
                "revert" => TextAlign::Revert,
                "unset" => TextAlign::Unset,
                _ => TextAlign::Left,
            };
            Some(Value::TextAlign(ta))
        }

        // ── TextWrap ───────────────────────────────────────────────────────
        StyleProperty::TextWrap => {
            let tw = match p.as_string()? {
                "nowrap" => TextWrap::NoWrap,
                "balance" => TextWrap::Balance,
                "pretty" => TextWrap::Pretty,
                "stable" => TextWrap::Stable,
                "initial" => TextWrap::Initial,
                "inherit" => TextWrap::Inherit,
                "revert" => TextWrap::Revert,
                "revert-layer" => TextWrap::RevertLayer,
                "unset" => TextWrap::Unset,
                _ => TextWrap::Wrap,
            };
            Some(Value::TextWrap(tw))
        }

        // ── Border styles ──────────────────────────────────────────────────
        StyleProperty::BorderTopStyle
        | StyleProperty::BorderRightStyle
        | StyleProperty::BorderBottomStyle
        | StyleProperty::BorderLeftStyle => {
            let s = p.as_string()?;
            Some(Value::BorderStyle(str_to_border_style(s)))
        }

        // ── Numeric properties ─────────────────────────────────────────────
        StyleProperty::FlexGrow
        | StyleProperty::FlexShrink
        | StyleProperty::AspectRatio
        | StyleProperty::ScrollbarWidth => Some(Value::Number(p.as_number()?)),

        // ── line-height: unitless number is a multiplier, not pixels ───────
        StyleProperty::LineHeight => {
            if let Some((v, unit)) = p.as_unit() {
                // `em` needs the element's font-size, unknown here - defer to `get_style`
                // (`unit_to_px` would resolve it against a hardcoded 16px).
                if unit == "em" {
                    return Some(Value::Unit(v, Unit::Em));
                }
                Some(Value::Unit(p.unit_to_px(), Unit::Px))
            } else if let Some(pct) = p.as_percentage() {
                // Percentages resolve against the element's own font-size, i.e. exactly `em`
                // semantics - and `get_style` resolves `em` at the declaring element, giving the
                // spec's inherit-as-computed-px behaviour for free.
                Some(Value::Unit(pct / 100.0, Unit::Em))
            } else if let Some(n) = p.as_number() {
                Some(Value::Number(n))
            } else {
                Some(Value::Keyword(intern(p.as_string()?)))
            }
        }

        // ── font-family: single string or comma-separated list ─────────────
        StyleProperty::FontFamily => {
            if let Some(s) = p.as_string() {
                return Some(Value::Keyword(intern(s)));
            }
            if let Some(list) = p.as_list() {
                // The list is flat: `DejaVu Sans` arrives as two identifier tokens, and `Comma`
                // separates alternative families. Rejoin adjacent tokens with a space so the name
                // survives intact instead of splitting into "DejaVu, Sans", which matches no font.
                let mut names = String::new();
                let mut need_space = false;
                for v in list {
                    if v.is_comma() {
                        names.push_str(", ");
                        need_space = false;
                        continue;
                    }
                    let Some(s) = v.as_string() else { continue };
                    if need_space {
                        names.push(' ');
                    }
                    names.push_str(s);
                    need_space = true;
                }
                if !names.is_empty() {
                    return Some(Value::Keyword(intern(&names)));
                }
            }
            None
        }

        // ── z-index: an integer (stacking order) or the `auto` keyword ─────
        StyleProperty::ZIndex => {
            if let Some(n) = p.as_number() {
                Some(Value::Number(n))
            } else {
                Some(Value::Keyword(intern(p.as_string()?)))
            }
        }

        // ── Grid track lists: `repeat(3, 1fr)`, `210px 1fr`, `auto`, … ─────
        // Stored as a `Function` (repeat/minmax) or a `List` - neither of which `as_string()`
        // returns - and a bare `1fr` is a `Unit`, so the default branch would drop or mis-type
        // them. Re-serialize to canonical CSS text for the layouter's `parse_grid_template`.
        StyleProperty::GridTemplateColumns
        | StyleProperty::GridTemplateRows
        | StyleProperty::GridAutoColumns
        | StyleProperty::GridAutoRows => {
            let s = if let Some(str) = p.as_string() {
                str.to_string()
            } else if let Some((name, args)) = p.as_function() {
                format!("{name}({})", join_grid_args::<S>(args))
            } else if let Some(list) = p.as_list() {
                list.iter().map(grid_value_to_string::<S>).collect::<Vec<_>>().join(" ")
            } else if let Some((val, unit)) = p.as_unit() {
                format!("{val}{unit}")
            } else {
                let pct = p.as_percentage()?;
                format!("{pct}%")
            };
            Some(Value::Keyword(intern(&s)))
        }

        // ── border-spacing: one length (both axes) or two (horizontal vertical) ──
        StyleProperty::BorderSpacingX | StyleProperty::BorderSpacingY => {
            if let Some(list) = p.as_list() {
                let lengths: Vec<f32> = list
                    .iter()
                    .filter_map(|v| {
                        if v.as_unit().is_some() {
                            Some(v.unit_to_px())
                        } else {
                            // Bare `0` is a valid length.
                            v.as_number()
                        }
                    })
                    .collect();
                let px = match (prop, lengths.as_slice()) {
                    (StyleProperty::BorderSpacingY, [_, y, ..]) => *y,
                    (_, [x, ..]) => *x,
                    _ => return None,
                };
                return Some(Value::Unit(px, Unit::Px));
            }
            if p.as_unit().is_some() {
                return Some(Value::Unit(p.unit_to_px(), Unit::Px));
            }
            p.as_number().map(|n| Value::Unit(n, Unit::Px))
        }

        // ── Default: unit-based or keyword ────────────────────────────────
        _ => {
            if let Some((v, unit)) = p.as_unit() {
                // Font-relative units must scale with the *element's* font-size, which we
                // don't know here. Express them as `em` (with an approximate factor for the
                // ones that aren't already font-multiples) and let `get_style` resolve them
                // against the computed font-size. Absolute and viewport units resolve to px
                // immediately. The factors are coarse stand-ins for real font metrics:
                // `ch` ≈ width of "0", `ex` ≈ x-height, `lh` ≈ line box.
                let value = match unit {
                    "em" => Value::Unit(v, Unit::Em),
                    // 0.55em, not the spec's 0.5em fallback: real proportional fonts sit nearer
                    // 0.52-0.6em, so 0.5em makes `ch` widths (`max-width: 17ch`) over-wrap.
                    "ch" => Value::Unit(v * 0.55, Unit::Em),
                    "ex" => Value::Unit(v * 0.5, Unit::Em),
                    "ic" => Value::Unit(v, Unit::Em),
                    "lh" => Value::Unit(v * 1.4, Unit::Em),
                    // `rem` is root-relative (always 16px here) and everything else is
                    // absolute/viewport - resolve straight to px, no element context needed.
                    _ => Value::Unit(p.unit_to_px(), Unit::Px),
                };
                Some(value)
            } else if let Some(pct) = p.as_percentage() {
                Some(Value::Unit(pct, Unit::Percent))
            } else if let Some(n) = p.as_number() {
                Some(Value::Unit(n, Unit::Px))
            } else {
                Some(Value::Keyword(intern(p.as_string()?)))
            }
        }
    }
}

/// Serializes one grid track-list value back to canonical CSS text (`1fr`, `minmax(100px, 1fr)`,
/// …), reconstructing a `grid-template-*` string the layouter can parse.
fn grid_value_to_string<S: CssSystem>(v: &S::Value) -> String {
    if let Some(s) = v.as_string() {
        return s.to_string();
    }
    if let Some((val, unit)) = v.as_unit() {
        return format!("{val}{unit}");
    }
    if let Some(pct) = v.as_percentage() {
        return format!("{pct}%");
    }
    if v.is_comma() {
        return ",".to_string();
    }
    if let Some((name, args)) = v.as_function() {
        return format!("{name}({})", join_grid_args::<S>(args));
    }
    if let Some(list) = v.as_list() {
        return list.iter().map(grid_value_to_string::<S>).collect::<Vec<_>>().join(" ");
    }
    if let Some(n) = v.as_number() {
        return format!("{n}");
    }
    String::new()
}

/// Joins grid function args (`repeat(3, 1fr)`), rendering commas as `, ` and the rest
/// space-separated.
fn join_grid_args<S: CssSystem>(args: &[S::Value]) -> String {
    let mut out = String::new();
    for arg in args {
        if arg.is_comma() {
            out.push_str(", ");
        } else {
            if !out.is_empty() && !out.ends_with(' ') {
                out.push(' ');
            }
            out.push_str(&grid_value_to_string::<S>(arg));
        }
    }
    out.trim().to_string()
}

/// Recursively search a CSS value tree for the first `url(...)` token and return its
/// (unresolved) target, stripping any quotes. Used for `background-image`.
fn css_value_url<S: CssSystem>(v: &S::Value) -> Option<String> {
    if let Some((name, args)) = v.as_function() {
        if name.eq_ignore_ascii_case("url") {
            if let Some(s) = args.iter().find_map(|a| a.as_string()) {
                return Some(s.trim_matches(['"', '\'']).to_string());
            }
        }
    }
    if let Some(list) = v.as_list() {
        return list.iter().find_map(css_value_url::<S>);
    }
    None
}

/// First `url(...)` in a property value - handles both the `background-image` longhand (a bare
/// `url()` function) and the `background` shorthand (a list like `[url(...), no-repeat]`).
fn css_property_url<S: CssSystem>(p: &S::Property) -> Option<String> {
    if let Some((name, args)) = p.as_function() {
        if name.eq_ignore_ascii_case("url") {
            if let Some(s) = args.iter().find_map(|a| a.as_string()) {
                return Some(s.trim_matches(['"', '\'']).to_string());
            }
        }
    }
    if let Some(list) = p.as_list() {
        return list.iter().find_map(css_value_url::<S>);
    }
    None
}

/// First colour token of a `background` shorthand (`#fff url(...) no-repeat`), components 0..=255.
fn css_property_bg_color<S: CssSystem>(p: &S::Property) -> Option<(u8, u8, u8, u8)> {
    // Single-value shorthand: a bare `<color>` (hex/function collapse to a concrete colour at
    // parse time; a named/system colour arrives as a string).
    if let Some(s) = p.as_string() {
        if let Some(c) = css_system_color(s) {
            return Some(c);
        }
    }
    if let Some((r, g, b, a)) = p.parse_color() {
        return Some((r as u8, g as u8, b as u8, a as u8));
    }
    // Multi-token shorthand: pick the first token that is a concrete colour.
    if let Some(list) = p.as_list() {
        for v in list {
            if let Some((r, g, b, a)) = v.as_color() {
                return Some((r as u8, g as u8, b as u8, a as u8));
            }
            if let Some(c) = v.as_string().and_then(css_system_color) {
                return Some(c);
            }
        }
    }
    None
}

// ── Gradient parsing ──────────────────────────────────────────────────────────

/// Parses `linear-gradient(...)` args: an optional leading direction (`to <side>[ <side>]` or an
/// `<angle>`) then two or more stops. Positionless stops are spread evenly between neighbours.
fn parse_linear_gradient<S: CssSystem>(args: &[S::Value]) -> Option<Gradient> {
    let mut groups: Vec<Vec<&S::Value>> = Vec::new();
    let mut current: Vec<&S::Value> = Vec::new();
    for a in args {
        if a.is_comma() {
            groups.push(std::mem::take(&mut current));
        } else {
            current.push(a);
        }
    }
    groups.push(current);

    // An optional direction occupies the first group when it carries no colour.
    let mut angle_deg = 180.0_f32; // CSS default direction is `to bottom`.
    let mut first_stop = 0;
    if let Some(first) = groups.first() {
        if let Some(angle) = parse_gradient_direction::<S>(first) {
            angle_deg = angle;
            first_stop = 1;
        }
    }

    let mut colors: Vec<Color> = Vec::new();
    let mut offsets: Vec<Option<f32>> = Vec::new();
    for group in groups.iter().skip(first_stop) {
        // Named colours and `transparent` tokenise as plain identifiers, so `as_color()` misses
        // them - fall back to string parsing, which `#e6e6e6 25%, transparent 25%` relies on.
        let color = group
            .iter()
            .find_map(|v| v.as_color())
            .map(|(r, g, b, a)| Color::from_rgba(r / 255.0, g / 255.0, b / 255.0, a / 255.0))
            .or_else(|| group.iter().find_map(|v| v.as_string()).and_then(Color::try_from_css));
        let Some(color) = color else {
            continue;
        };
        colors.push(color);
        offsets.push(group.iter().find_map(|v| v.as_percentage()).map(|p| p / 100.0));
    }
    let n = colors.len();
    if n < 2 {
        return None;
    }

    // Anchor the endpoints, then linearly interpolate any interior gaps.
    if offsets[0].is_none() {
        offsets[0] = Some(0.0);
    }
    if offsets[n - 1].is_none() {
        offsets[n - 1] = Some(1.0);
    }
    let mut i = 0;
    while i < n {
        if offsets[i].is_some() {
            i += 1;
            continue;
        }
        let start = i - 1; // resolved (endpoints are anchored)
        let mut end = i;
        while end < n && offsets[end].is_none() {
            end += 1;
        }
        let a = offsets[start].unwrap_or(0.0);
        let b = offsets.get(end).and_then(|o| *o).unwrap_or(1.0);
        let steps = (end - start) as f32;
        for (k, slot) in offsets.iter_mut().enumerate().take(end).skip(start + 1) {
            *slot = Some(a + (b - a) * ((k - start) as f32) / steps);
        }
        i = end;
    }

    // Clamp to [0,1] and keep positions non-decreasing (CSS gradient rule).
    let mut running = 0.0_f32;
    let stops = colors
        .into_iter()
        .zip(offsets)
        .map(|(color, off)| {
            let off = off.unwrap_or(0.0).clamp(0.0, 1.0).max(running);
            running = off;
            ColorStop { offset: off, color }
        })
        .collect();

    Some(Gradient::Linear(LinearGradient {
        angle_deg,
        stops,
        tiling: None,
    }))
}

/// Gradient-line angle in CSS degrees, or `None` if the group is a colour stop rather than a
/// direction (so the gradient uses the default `to bottom`).
fn parse_gradient_direction<S: CssSystem>(group: &[&S::Value]) -> Option<f32> {
    // Angle form: `45deg`, `0.25turn`, `1.5rad`, `100grad`.
    if let Some((v, unit)) = group.first().and_then(|first| first.as_unit()) {
        return match unit {
            "deg" => Some(v),
            "grad" => Some(v * 0.9),
            "rad" => Some(v.to_degrees()),
            "turn" => Some(v * 360.0),
            _ => None,
        };
    }
    // Keyword form: `to <side> [<side>]`.
    let words: Vec<String> = group
        .iter()
        .filter_map(|v| v.as_string())
        .map(|s| s.cow_to_lowercase().into_owned())
        .collect();
    if words.first().map(String::as_str) != Some("to") {
        return None;
    }
    let has = |k: &str| words.iter().any(|w| w == k);
    Some(match (has("top"), has("right"), has("bottom"), has("left")) {
        (true, false, false, false) => 0.0,
        (false, true, false, false) => 90.0,
        (false, false, false, true) => 270.0,
        (true, true, false, false) => 45.0,
        (false, true, true, false) => 135.0,
        (false, false, true, true) => 225.0,
        (true, false, false, true) => 315.0,
        // `to bottom` and any unrecognised combination fall back to a downward gradient.
        _ => 180.0,
    })
}

/// All `linear-gradient(...)` layers of a `background-image` property, in source order (the
/// first listed layer paints on top). Non-gradient layers (`url()`, `none`) are skipped.
fn property_gradient_layers<S: CssSystem>(p: &S::Property) -> Vec<LinearGradient> {
    let mut out = Vec::new();
    let mut push_fn = |name: &str, args: &[S::Value]| {
        if name.eq_ignore_ascii_case("linear-gradient") {
            if let Some(Gradient::Linear(g)) = parse_linear_gradient::<S>(args) {
                out.push(g);
            }
        }
    };
    if let Some((name, args)) = p.as_function() {
        push_fn(name, args);
        return out;
    }
    if let Some(list) = p.as_list() {
        for v in list {
            if let Some((name, args)) = v.as_function() {
                push_fn(name, args);
            }
        }
    }
    out
}

/// One resolved token from a `background-size`/`-position`/`-repeat` value.
enum BgTok {
    /// A `<length>` in px (bare `0` included).
    Len(f32),
    /// A `<percentage>` (0..100). The value is retained for future box-relative resolution;
    /// today a percentage size/position falls back to "fill box" / zero offset.
    #[allow(dead_code)]
    Pct(f32),
    /// A keyword (`cover`, `center`, `no-repeat`, …), lowercased.
    Kw(String),
}

fn value_bg_tok<S: CssSystem>(v: &S::Value) -> Option<BgTok> {
    if let Some((val, unit)) = v.as_unit() {
        if unit.eq_ignore_ascii_case("px") {
            return Some(BgTok::Len(val));
        }
    }
    if let Some(pct) = v.as_percentage() {
        return Some(BgTok::Pct(pct));
    }
    if let Some(n) = v.as_number() {
        if n == 0.0 {
            return Some(BgTok::Len(0.0)); // bare `0`
        }
    }
    v.as_string()
        .map(|s| BgTok::Kw(s.cow_to_ascii_lowercase().into_owned()))
}

fn prop_bg_tok<S: CssSystem>(p: &S::Property) -> Option<BgTok> {
    if let Some((val, unit)) = p.as_unit() {
        if unit.eq_ignore_ascii_case("px") {
            return Some(BgTok::Len(val));
        }
    }
    if let Some(pct) = p.as_percentage() {
        return Some(BgTok::Pct(pct));
    }
    if let Some(n) = p.as_number() {
        if n == 0.0 {
            return Some(BgTok::Len(0.0));
        }
    }
    p.as_string()
        .map(|s| BgTok::Kw(s.cow_to_ascii_lowercase().into_owned()))
}

/// Split a `background-*` longhand into comma-separated groups (one per `<bg-layer>`).
/// A scalar property (e.g. `background-repeat: repeat`) is a single group.
fn bg_token_groups<S: CssSystem>(p: &S::Property) -> Vec<Vec<BgTok>> {
    if let Some(list) = p.as_list() {
        let mut groups: Vec<Vec<BgTok>> = vec![Vec::new()];
        for v in list {
            if v.is_comma() {
                groups.push(Vec::new());
            } else if let Some(t) = value_bg_tok::<S>(v) {
                // `groups` is seeded with one Vec and only grows, so `last_mut` is always Some;
                // handle it without `expect` (which the workspace lints deny).
                if let Some(last) = groups.last_mut() {
                    last.push(t);
                }
            }
        }
        return groups;
    }
    match prop_bg_tok::<S>(p) {
        Some(t) => vec![vec![t]],
        None => Vec::new(),
    }
}

/// `background-size` group → tile size in px, or `None` for `auto`/`cover`/`contain`/`%`
/// (which mean "fill the box", i.e. no tiling).
fn resolve_bg_size(group: &[BgTok]) -> Option<(f32, f32)> {
    let mut dims = Vec::new();
    for t in group {
        match t {
            BgTok::Len(v) => dims.push(*v),
            // Percentage- and keyword-sized backgrounds need the box size to resolve; treat
            // them as "fill the box" for now (no tiling).
            BgTok::Pct(_) | BgTok::Kw(_) => return None,
        }
    }
    match dims.as_slice() {
        [w] => Some((*w, *w)),
        [w, h, ..] => Some((*w, *h)),
        _ => None,
    }
}

/// `background-position` group → (x, y) px phase offset. Percentages and edge keywords need the
/// box size, so they resolve to 0 for now; px offsets are exact.
fn resolve_bg_position(group: &[BgTok]) -> (f32, f32) {
    let lens: Vec<f32> = group
        .iter()
        .filter_map(|t| match t {
            BgTok::Len(v) => Some(*v),
            _ => None,
        })
        .collect();
    match lens.as_slice() {
        [x] => (*x, 0.0),
        [x, y, ..] => (*x, *y),
        _ => (0.0, 0.0),
    }
}

/// `background-repeat` group → (repeat_x, repeat_y). Defaults to repeating both axes.
fn resolve_bg_repeat(group: &[BgTok]) -> (bool, bool) {
    let kws: Vec<&str> = group
        .iter()
        .filter_map(|t| match t {
            BgTok::Kw(k) => Some(k.as_str()),
            _ => None,
        })
        .collect();
    let axis = |k: &str| k != "no-repeat"; // repeat / space / round all tile
    match kws.as_slice() {
        [] => (true, true),
        ["repeat-x"] => (true, false),
        ["repeat-y"] => (false, true),
        [a] => (axis(a), axis(a)),
        [a, b, ..] => (axis(a), axis(b)),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PipelineNodeKind {
    Text,
    Comment,
    Element,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BgSize {
    /// `auto` / absent - the image's intrinsic size.
    Auto,
    /// Explicit lengths in px.
    Length(f32, f32),
    /// `cover` - scale (preserving aspect) so the image fully covers the box, cropping overflow.
    Cover,
    /// `contain` - scale (preserving aspect) so the image fits inside the box, letterboxing.
    Contain,
}

/// Resolved `background-repeat`/`-size`/`-position` for an element's first background layer. Read
/// from the `background` shorthand as well as the longhands, since pages write `background: url(x)
/// repeat`. `cover`/`contain` need the box size, so final tile geometry is computed at paint time.
#[derive(Debug, Clone, Copy)]
pub struct BgImageLayout {
    /// Whether the tile repeats on the x / y axis (`background-repeat`; default repeat both).
    pub repeat: (bool, bool),
    /// Tile origin offset from the box origin, in px (`background-position`, length form).
    pub position: (f32, f32),
    /// Per-axis `center` keyword (`background-position: center`) - resolved against the box at paint.
    pub center: (bool, bool),
    /// Resolved `background-size`.
    pub size: BgSize,
}

impl Default for BgImageLayout {
    fn default() -> Self {
        BgImageLayout {
            repeat: (true, true),
            position: (0.0, 0.0),
            center: (false, false),
            size: BgSize::Auto,
        }
    }
}

// ── PipelineDocument trait ────────────────────────────────────────────────────

pub trait PipelineDocument: Send + Sync {
    fn root(&self) -> Option<NodeId>;
    fn children(&self, id: NodeId) -> Vec<NodeId>;
    fn node_kind(&self, id: NodeId) -> PipelineNodeKind;
    fn tag_name(&self, id: NodeId) -> Option<String>;
    fn is_display_none(&self, id: NodeId) -> bool;
    fn parent(&self, id: NodeId) -> Option<NodeId>;
    fn html_node_id(&self) -> Option<NodeId>;
    fn body_node_id(&self) -> Option<NodeId>;
    fn base_url(&self) -> String;
    fn inner_html(&self, id: NodeId) -> String;
    fn get_node_by_id(&self, _id: NodeId) -> Option<Node> {
        None
    }

    /// Returns the own (explicitly-set) value for `prop` on node `id`, without recursing.
    fn get_own_style(&self, id: NodeId, prop: &StyleProperty) -> Option<Value>;

    /// `background-image` gradient layers in source order (first listed paints on top), each
    /// carrying its resolved tiling (`None` tiling = fill the box). Empty for solid/image
    /// backgrounds.
    fn background_layers(&self, _id: NodeId) -> Vec<Gradient> {
        Vec::new()
    }

    /// Tiling for a raster/SVG `background-image`, read from both the `background` shorthand and
    /// the longhands. Defaults to "repeat both axes, intrinsic size, no offset".
    fn background_image_layout(&self, _id: NodeId) -> BgImageLayout {
        BgImageLayout::default()
    }

    /// Forces the next `get_own_style` to re-evaluate CSS selectors (including `:hover`) from
    /// scratch. No-op for backends that do not cache styles.
    fn clear_style_cache(&self) {}

    /// Cheaper than `clear_style_cache` for hover repaints where only a few elements changed.
    fn invalidate_style_for_nodes(&self, _ids: &[NodeId]) {}

    /// Returns the computed value for `prop` on node `id`:
    ///  1. own value if set,
    ///  2. parent's computed value if the property is inherited,
    ///  3. the CSS-spec initial value otherwise.
    fn get_style(&self, id: NodeId, prop: &StyleProperty) -> Value {
        // A border whose style is none/hidden computes to zero width regardless of the declared or
        // initial width. Enforced here so layout and paint can't disagree about the box.
        if let Some(style_prop) = border_width_peer_style(prop) {
            if let Value::BorderStyle(s) = self.get_style(id, &style_prop) {
                if matches!(s, BorderStyle::None | BorderStyle::Hidden) {
                    return Value::Unit(0.0, Unit::Px);
                }
            }
        }

        let raw = if let Some(v) = self.get_own_style(id, prop) {
            v
        } else {
            // border-*-color's initial value is `currentColor`, not black: an undeclared
            // border color renders in the element's computed `color`
            // (`td { border: solid; color: blue }` draws blue borders).
            if matches!(
                prop,
                StyleProperty::BorderTopColor
                    | StyleProperty::BorderRightColor
                    | StyleProperty::BorderBottomColor
                    | StyleProperty::BorderLeftColor
            ) {
                return self.get_style(id, &StyleProperty::Color);
            }
            // Monospace default-size quirk (Chrome/Firefox both do this): the default
            // font-size is 13px instead of 16px for elements whose font-family is the bare
            // generic `monospace`. Browsers keep the `medium` keyword identity through
            // inheritance and re-evaluate it per family; we approximate by applying the
            // quirk when no ancestor declares a font-size at all.
            if matches!(prop, StyleProperty::FontSize) {
                let family_is_monospace = match self.get_style(id, &StyleProperty::FontFamily) {
                    Value::Keyword(fam) => lookup(fam)
                        .split(',')
                        .next()
                        .is_some_and(|f| f.trim().eq_ignore_ascii_case("monospace")),
                    _ => false,
                };
                if family_is_monospace {
                    let mut cur = self.parent(id);
                    let mut declared = false;
                    while let Some(p) = cur {
                        if self.get_own_style(p, prop).is_some() {
                            declared = true;
                            break;
                        }
                        cur = self.parent(p);
                    }
                    if !declared {
                        return Value::Unit(13.0, Unit::Px);
                    }
                }
            }
            let meta = prop.meta();
            if meta.inherited {
                if let Some(parent) = self.parent(id) {
                    return self.get_style(parent, prop);
                }
            }
            meta.initial_value()
        };

        // Resolve font-relative units (em/rem) to px. `rem` is always relative to the root
        // element's font-size (16px default). `em` is relative to the *parent's* computed
        // font-size for `font-size` itself, and to the element's *own* computed font-size
        // for every other property (e.g. `max-width: 17ch` lands here as `em`).
        match &raw {
            Value::Unit(v, Unit::Rem) => Value::Unit(v * 16.0, Unit::Px),
            Value::Unit(v, Unit::Em) => {
                let basis = if matches!(prop, StyleProperty::FontSize) {
                    match self.parent(id) {
                        Some(parent) => self.font_size_px(parent),
                        None => 16.0,
                    }
                } else {
                    self.font_size_px(id)
                };
                Value::Unit(v * basis, Unit::Px)
            }
            _ => raw,
        }
    }

    /// The computed `font-size` of `id` in px, or 16px if unresolvable. Resolving
    /// `font-size` only ever recurses to the *parent* (never to `id` itself), so this is
    /// safe to call while resolving font-relative units on other properties of `id`.
    fn font_size_px(&self, id: NodeId) -> f32 {
        match self.get_style(id, &StyleProperty::FontSize) {
            Value::Unit(px, Unit::Px) => px,
            _ => 16.0,
        }
    }

    fn get_style_f32(&self, id: NodeId, prop: &StyleProperty) -> f32 {
        match self.get_style(id, prop) {
            Value::Unit(v, _) => v,
            Value::Number(v) => v,
            _ => 0.0,
        }
    }
}

// ── Pseudo-element (::before / ::after) synthetic nodes ───────────────────────
//
// Generated content has no DOM node, but the pipeline is keyed by `NodeId` - so mint synthetic
// ids the adapter resolves on the fly, letting the rest of the pipeline treat them as normal nodes.
//
// Encoding: top bit flags a synthetic id, next two bits are the role, the rest hold the owner
// element id. Real DOM ids are small, so the high bits are free.
const PSEUDO_FLAG: u64 = 1 << 62;
const ROLE_BEFORE_ELEM: u64 = 0; // the ::before pseudo-element box
const ROLE_AFTER_ELEM: u64 = 1; // the ::after pseudo-element box
const ROLE_BEFORE_TEXT: u64 = 2; // generated text child of ::before
const ROLE_AFTER_TEXT: u64 = 3; // generated text child of ::after

const fn is_pseudo_id(id_val: u64) -> bool {
    id_val & PSEUDO_FLAG != 0
}

fn encode_pseudo(owner: NodeId, role: u64) -> NodeId {
    NodeId::from(PSEUDO_FLAG | (u64::from(owner) << 2) | role)
}

fn decode_pseudo(id: NodeId) -> (NodeId, u64) {
    let v = u64::from(id) & !PSEUDO_FLAG;
    (NodeId::from(v >> 2), v & 0b11)
}

const fn role_is_after(role: u64) -> bool {
    matches!(role, ROLE_AFTER_ELEM | ROLE_AFTER_TEXT)
}

// ── Anonymous table boxes (CSS 2.1 §17.2.1, "generate missing parents") ───────
//
// A run of consecutive table-internal children (display: table-cell / table-row /
// row groups / ...) whose parent provides no table context must be wrapped in an
// anonymous table box. The wrapper is minted like a pseudo-element id: bit 61 flags
// the id and the payload is the run's FIRST member. Downstream (render tree, taffy,
// lattice, painter) then sees a regular `display: table` element; lattice's own
// fixup generates the missing rows/row-groups inside it.
const ANON_TABLE_FLAG: u64 = 1 << 61;
/// Anonymous table-ROW wrapper around a run of children that are not proper table/row-group
/// children. Needed not just for CSS structure: the taffy FIRST pass approximates a row as a
/// flex row, so without the wrapper bare cells stack vertically and a fit-content ancestor
/// (e.g. an abs-positioned overlay div) collapses to one cell's width.
const ANON_ROW_FLAG: u64 = 1 << 60;
/// Anonymous table-CELL wrapper around a run of non-cell children inside a row.
const ANON_CELL_FLAG: u64 = 1 << 59;

const ANON_FLAGS: u64 = ANON_TABLE_FLAG | ANON_ROW_FLAG | ANON_CELL_FLAG;

const fn is_anon_table_id(id_val: u64) -> bool {
    id_val & ANON_FLAGS == ANON_TABLE_FLAG && id_val & PSEUDO_FLAG == 0
}

const fn is_anon_row_id(id_val: u64) -> bool {
    id_val & ANON_FLAGS == ANON_ROW_FLAG && id_val & PSEUDO_FLAG == 0
}

const fn is_anon_cell_id(id_val: u64) -> bool {
    id_val & ANON_FLAGS == ANON_CELL_FLAG && id_val & PSEUDO_FLAG == 0
}

/// Any flavour of synthetic anonymous table box.
const fn is_anon_box_id(id_val: u64) -> bool {
    is_anon_table_id(id_val) || is_anon_row_id(id_val) || is_anon_cell_id(id_val)
}

/// The display a synthetic anonymous box carries.
fn anon_box_display(id_val: u64) -> Option<Display> {
    if is_anon_table_id(id_val) {
        Some(Display::Table)
    } else if is_anon_row_id(id_val) {
        Some(Display::TableRow)
    } else if is_anon_cell_id(id_val) {
        Some(Display::TableCell)
    } else {
        None
    }
}

fn encode_anon_table(first_member: NodeId) -> NodeId {
    NodeId::from(ANON_TABLE_FLAG | u64::from(first_member))
}

fn encode_anon_row(first_member: NodeId) -> NodeId {
    NodeId::from(ANON_ROW_FLAG | u64::from(first_member))
}

fn encode_anon_cell(first_member: NodeId) -> NodeId {
    NodeId::from(ANON_CELL_FLAG | u64::from(first_member))
}

fn decode_anon_box(id: NodeId) -> NodeId {
    NodeId::from(u64::from(id) & !ANON_FLAGS)
}

/// Is `child` a proper child of a table box (CSS 2.1 §17.2)? Everything else inside a table
/// gets wrapped in an anonymous row.
fn proper_table_child(d: Option<&Display>) -> bool {
    matches!(
        d,
        Some(
            Display::TableRow
                | Display::TableRowGroup
                | Display::TableHeaderGroup
                | Display::TableFooterGroup
                | Display::TableCaption
                | Display::TableColumn
                | Display::TableColumnGroup
        )
    )
}

/// Does a child with display `child` require a table ancestor that a parent with
/// display `parent` does not provide?
fn needs_table_parent(child: &Display, parent: Option<&Display>) -> bool {
    use Display::*;
    match child {
        TableCell => !matches!(
            parent,
            Some(Table | TableRow | TableRowGroup | TableHeaderGroup | TableFooterGroup)
        ),
        TableRow => !matches!(parent, Some(Table | TableRowGroup | TableHeaderGroup | TableFooterGroup)),
        TableRowGroup | TableHeaderGroup | TableFooterGroup | TableCaption | TableColumnGroup => {
            !matches!(parent, Some(Table))
        }
        TableColumn => !matches!(parent, Some(Table | TableColumnGroup)),
        _ => false,
    }
}

const fn role_is_text(role: u64) -> bool {
    matches!(role, ROLE_BEFORE_TEXT | ROLE_AFTER_TEXT)
}

/// A materialized pseudo-element: its computed style map plus the generated text (if the
/// resolved `content` produced any). `text == None` means an empty box (e.g. `content: ""`).
struct PseudoBox<P> {
    styles: Arc<P>,
    text: Option<String>,
}

fn unquote(s: &str) -> String {
    let b = s.as_bytes();
    if b.len() >= 2 && ((b[0] == b'"' && b[b.len() - 1] == b'"') || (b[0] == b'\'' && b[b.len() - 1] == b'\'')) {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

/// `None` only for `none`/`normal`, which suppress the box entirely.
fn content_token_to_string(s: &str) -> Option<String> {
    match s {
        "none" | "normal" => None,
        // We have no quote-pair stack, so use the typographic defaults.
        "open-quote" => Some("\u{201C}".to_string()),
        "close-quote" => Some("\u{201D}".to_string()),
        "no-open-quote" | "no-close-quote" => Some(String::new()),
        _ => Some(unquote(s)),
    }
}

/// Counter state (counter-reset/-increment scoping) is not tracked yet, so counters resolve to
/// empty text - generated boxes still appear, just without the number.
fn resolve_content_function<S: CssSystem>(name: &str, _args: &[S::Value]) -> String {
    if matches!(name, "counter" | "counters") {
        log::debug!("content: {name}() is not yet supported; rendering empty");
    }
    String::new()
}

fn content_value_to_string<S: CssSystem>(v: &S::Value) -> Option<String> {
    if let Some(s) = v.as_string() {
        return content_token_to_string(s);
    }
    if let Some((name, args)) = v.as_function() {
        return Some(resolve_content_function::<S>(name, args));
    }
    if let Some(list) = v.as_list() {
        let mut out = String::new();
        for item in list {
            if let Some(part) = content_value_to_string::<S>(item) {
                out.push_str(&part);
            }
        }
        return Some(out);
    }
    None
}

/// `None` => generate no box (`content: none | normal`); `Some("")` => an empty box.
fn resolve_content<S: CssSystem>(p: &S::Property) -> Option<String> {
    // A single string/keyword token.
    if let Some(s) = p.as_string() {
        return content_token_to_string(s);
    }
    // A list of tokens (strings, attr()/var() already resolved upstream, counters, quotes).
    if let Some(list) = p.as_list() {
        let mut out = String::new();
        for v in list {
            if let Some(part) = content_value_to_string::<S>(v) {
                out.push_str(&part);
            }
        }
        return Some(out);
    }
    // A bare function value.
    if let Some((name, args)) = p.as_function() {
        return Some(resolve_content_function::<S>(name, args));
    }
    None
}

// ── GosubDocumentAdapter ──────────────────────────────────────────────────────

/// Adapts any `gosub_interface::document::Document<C>` into a `PipelineDocument`.
pub struct GosubDocumentAdapter<C>
where
    C: HasDocument,
    <C::CssSystem as CssSystem>::PropertyMap: Send + Sync,
{
    pub doc: Arc<C::Document>,
    /// Per-node computed-style cache (from CSS selector matching). Populated lazily.
    style_cache: Mutex<HashMap<NodeId, Arc<<C::CssSystem as CssSystem>::PropertyMap>>>,
    /// Per-node inline-style cache (from the `style` attribute, highest specificity).
    inline_style_cache: Mutex<HashMap<NodeId, NodeStyle>>,
    /// Materialized `::before` / `::after` pseudo-boxes, keyed by `(owner, is_after)`.
    /// `None` means "no generated box". Populated lazily.
    #[allow(clippy::type_complexity)]
    pseudo_cache: Mutex<HashMap<(NodeId, bool), Option<Arc<PseudoBox<<C::CssSystem as CssSystem>::PropertyMap>>>>>,
}

impl<C> GosubDocumentAdapter<C>
where
    C: HasDocument + Send + Sync + 'static,
    C::Document: Send + Sync,
    <C::CssSystem as CssSystem>::PropertyMap: Send + Sync,
{
    pub fn new(doc: Arc<C::Document>) -> Self {
        Self {
            doc,
            style_cache: Mutex::new(HashMap::new()),
            inline_style_cache: Mutex::new(HashMap::new()),
            pseudo_cache: Mutex::new(HashMap::new()),
        }
    }

    /// `None` if no rule generates one. Computed and cached on first access.
    fn pseudo_box(
        &self,
        owner: NodeId,
        is_after: bool,
    ) -> Option<Arc<PseudoBox<<C::CssSystem as CssSystem>::PropertyMap>>> {
        if let Some(cached) = self.pseudo_cache.lock().get(&(owner, is_after)) {
            return cached.clone();
        }

        let result = self.compute_pseudo_box(owner, is_after);
        self.pseudo_cache.lock().insert((owner, is_after), result.clone());
        result
    }

    fn compute_pseudo_box(
        &self,
        owner: NodeId,
        is_after: bool,
    ) -> Option<Arc<PseudoBox<<C::CssSystem as CssSystem>::PropertyMap>>> {
        // Pseudo-elements only hang off real elements.
        if self.doc.node_type(owner) != GosubNodeType::ElementNode {
            return None;
        }
        let name = if is_after { "after" } else { "before" };
        let sheets = self.doc.stylesheets();
        let mut prop_map = C::CssSystem::pseudo_properties_from_node::<C>(&*self.doc, owner, sheets, name)?;
        for (_, prop) in prop_map.iter_mut() {
            prop.compute_value();
        }

        // Resolve `content` into generated text. `none`/`normal` means no box at all.
        let content_prop = <_ as CssPropertyMap<C::CssSystem>>::get(&prop_map, "content")?;
        let text = resolve_content::<C::CssSystem>(content_prop)?;

        // `content: ""` (and any all-empty result) generates a box but no text child.
        let text = if text.is_empty() { None } else { Some(text) };

        Some(Arc::new(PseudoBox {
            styles: Arc::new(prop_map),
            text,
        }))
    }

    fn cached_styles(&self, id: NodeId) -> Arc<<C::CssSystem as CssSystem>::PropertyMap> {
        {
            if let Some(arc) = self.style_cache.lock().get(&id) {
                return arc.clone();
            }
        }
        let (prop_map, inline_ns) = self.compute_styles(id);
        let arc = Arc::new(prop_map);
        self.style_cache.lock().insert(id, arc.clone());
        self.inline_style_cache.lock().insert(id, inline_ns);
        arc
    }

    fn compute_styles(&self, id: NodeId) -> (<C::CssSystem as CssSystem>::PropertyMap, NodeStyle) {
        // CSS selectors cannot target text nodes - only elements.
        if self.doc.node_type(id) == GosubNodeType::TextNode {
            return (Default::default(), NodeStyle::new());
        }
        let sheets = self.doc.stylesheets();
        let mut prop_map = C::CssSystem::properties_from_node::<C>(&*self.doc, id, sheets).unwrap_or_default();
        for (_, prop) in prop_map.iter_mut() {
            prop.compute_value();
        }

        // Inline `style` attribute has highest specificity - store separately.
        let inline_ns = if let Some(attrs) = self.doc.attributes(id) {
            if let Some(style_attr) = attrs.get("style") {
                crate::common::document::inline_style::parse_inline_style_attr(style_attr)
            } else {
                NodeStyle::new()
            }
        } else {
            NodeStyle::new()
        };

        (prop_map, inline_ns)
    }

    /// Own style for a pseudo-element id, read from its generated style map.
    fn pseudo_own_style(&self, id: NodeId, prop: &StyleProperty) -> Option<Value> {
        let (owner, role) = decode_pseudo(id);
        // Generated text nodes carry no own style; inheritance flows from the pseudo-element.
        if role_is_text(role) {
            return None;
        }
        let pb = self.pseudo_box(owner, role_is_after(role))?;
        self.style_from_map(id, prop, pb.styles.as_ref())
    }

    // ── Anonymous table synthesis ─────────────────────────────────────────────

    /// The node's computed `display`, if the cascade assigned one.
    fn display_of(&self, id: NodeId) -> Option<Display> {
        match self.get_own_style(id, &StyleProperty::Display) {
            Some(Value::Display(d)) => Some(d),
            _ => None,
        }
    }

    /// Children skipped silently when collecting anonymous runs: whitespace-only text,
    /// comments/doctypes, and `display: none` children (none of them generate a box).
    fn run_skippable(&self, id: NodeId) -> bool {
        let raw = u64::from(id);
        if is_pseudo_id(raw) || is_anon_box_id(raw) {
            return false;
        }
        match self.doc.node_type(id) {
            GosubNodeType::TextNode => {
                if self.doc.text_value(id).is_some_and(|t| !t.trim().is_empty()) {
                    return false;
                }
                // Whitespace-only text: skippable only when collapsing would remove it.
                // Under `white-space: pre`/`pre-wrap` the spaces are content and generate
                // anonymous boxes (CSS 2.1 §17.2.1 considers only whitespace "that would be
                // collapsed"). Resolved over the RAW DOM parent chain: `get_style` routes
                // through `parent()`, whose synthetic-wrapper resolution calls back into
                // `run_skippable` - a cycle.
                let mut cur = self.doc.parent(id);
                while let Some(p) = cur {
                    if let Some(Value::Keyword(k)) = self.get_own_style(p, &StyleProperty::WhiteSpace) {
                        return !matches!(lookup(k).as_str(), "pre" | "pre-wrap");
                    }
                    cur = self.doc.parent(p);
                }
                true
            }
            GosubNodeType::CommentNode | GosubNodeType::DocTypeNode => true,
            _ => matches!(self.display_of(id), Some(Display::None)),
        }
    }

    /// Is `id` an improper child of a row container with display `parent_display`
    /// (a table or row group), i.e. must it be wrapped in an anonymous row?
    fn needy_for_row(&self, id: NodeId, parent_display: &Display) -> bool {
        if self.run_skippable(id) {
            return false;
        }
        let d = self.display_of(id);
        match parent_display {
            Display::Table => !proper_table_child(d.as_ref()),
            // Row groups: only rows are proper.
            _ => !matches!(d, Some(Display::TableRow)),
        }
    }

    /// Is `id` an improper (non-cell) child of a row, i.e. must it be wrapped in an
    /// anonymous cell?
    fn needy_for_cell(&self, id: NodeId) -> bool {
        !self.run_skippable(id) && !matches!(self.display_of(id), Some(Display::TableCell))
    }


    /// Generic run-collapser: replace each run of consecutive `needy` children with one
    /// synthetic id (`encode` of the first member). Skippable children (whitespace text,
    /// comments, display:none) BETWEEN run members are absorbed into the run and dropped.
    fn collapse_runs(&self, kids: Vec<NodeId>, needy: impl Fn(NodeId) -> bool, encode: fn(NodeId) -> NodeId) -> Vec<NodeId> {
        if !kids.iter().any(|&k| needy(k)) {
            return kids;
        }
        let mut out = Vec::with_capacity(kids.len());
        let mut i = 0;
        while i < kids.len() {
            if !needy(kids[i]) {
                out.push(kids[i]);
                i += 1;
                continue;
            }
            out.push(encode(kids[i]));
            i += 1;
            loop {
                let mut j = i;
                while j < kids.len() && self.run_skippable(kids[j]) {
                    j += 1;
                }
                if j < kids.len() && needy(kids[j]) {
                    i = j + 1;
                } else {
                    break;
                }
            }
        }
        out
    }

    /// The real members of a synthetic run: the needy siblings starting at `first`
    /// (interior skippable children are dropped).
    fn run_members(&self, first: NodeId, needy: impl Fn(NodeId) -> bool) -> Vec<NodeId> {
        let mut members = vec![first];
        let Some(parent) = self.doc.parent(first) else {
            return members;
        };
        let kids = self.doc.children(parent);
        let Some(pos) = kids.iter().position(|&k| k == first) else {
            return members;
        };
        let mut i = pos + 1;
        loop {
            let mut j = i;
            while j < kids.len() && self.run_skippable(kids[j]) {
                j += 1;
            }
            if j < kids.len() && needy(kids[j]) {
                members.push(kids[j]);
                i = j + 1;
            } else {
                break;
            }
        }
        members
    }

    /// First member of the run (per `needy`) among `parent`'s children that contains
    /// `member`, mirroring `collapse_runs`' grouping.
    fn run_start_containing(&self, parent: NodeId, member: NodeId, needy: impl Fn(NodeId) -> bool) -> Option<NodeId> {
        let kids = self.doc.children(parent);
        let mut i = 0;
        while i < kids.len() {
            if !needy(kids[i]) {
                i += 1;
                continue;
            }
            let start = kids[i];
            let mut hit = kids[i] == member;
            i += 1;
            loop {
                let mut j = i;
                while j < kids.len() && self.run_skippable(kids[j]) {
                    j += 1;
                }
                if j < kids.len() && needy(kids[j]) {
                    hit |= kids[j] == member;
                    i = j + 1;
                } else {
                    break;
                }
            }
            if hit {
                return Some(start);
            }
        }
        None
    }

    /// Collapse each run of table-internal children lacking a table parent into one
    /// anonymous-table id (CSS 2.1 §17.2.1 "generate missing parents").
    fn wrap_anon_table_runs(&self, parent_display: Option<&Display>, kids: Vec<NodeId>) -> Vec<NodeId> {
        // A parent that itself provides table context never wraps a table: the anonymous
        // row/cell wrappers below own the interior of a table.
        if matches!(
            parent_display,
            Some(
                Display::Table
                    | Display::TableRow
                    | Display::TableRowGroup
                    | Display::TableHeaderGroup
                    | Display::TableFooterGroup
                    | Display::TableColumnGroup
            )
        ) {
            return kids;
        }
        self.collapse_runs(
            kids,
            |id| self.display_of(id).is_some_and(|d| needs_table_parent(&d, parent_display)),
            encode_anon_table,
        )
    }

    /// Collapse each run of improper children of a table / row group into one
    /// anonymous-row id.
    fn wrap_anon_row_runs(&self, parent_display: Option<&Display>, kids: Vec<NodeId>) -> Vec<NodeId> {
        let Some(pd) = parent_display else { return kids };
        if !matches!(
            pd,
            Display::Table | Display::TableRowGroup | Display::TableHeaderGroup | Display::TableFooterGroup
        ) {
            return kids;
        }
        self.collapse_runs(kids, |id| self.needy_for_row(id, pd), encode_anon_row)
    }

    /// Collapse each run of non-cell children of a (real or anonymous) row into one
    /// anonymous-cell id.
    fn wrap_anon_cell_runs(&self, parent_display: Option<&Display>, kids: Vec<NodeId>) -> Vec<NodeId> {
        if !matches!(parent_display, Some(Display::TableRow)) {
            return kids;
        }
        self.collapse_runs(kids, |id| self.needy_for_cell(id), encode_anon_cell)
    }

    /// Start of the maximal sub-run of consecutive `pred` members containing `id`.
    fn sub_run_start(&self, members: &[NodeId], id: NodeId, pred: impl Fn(NodeId) -> bool) -> NodeId {
        let Some(mut i) = members.iter().position(|&m| m == id) else {
            return id;
        };
        while i > 0 && pred(members[i - 1]) {
            i -= 1;
        }
        members[i]
    }

    /// The synthetic wrapper `children()` places `id` under, if any. Run members' parent
    /// chain must route through the anonymous boxes, or sibling walks (whitespace
    /// collapsing, vertical-align resolution) diverge from the tree children() produces.
    fn synthetic_parent_of(&self, id: NodeId) -> Option<NodeId> {
        let raw = u64::from(id);
        if is_pseudo_id(raw) || is_anon_box_id(raw) {
            return None;
        }
        let parent = self.doc.parent(id)?;
        let parent_display = self.display_of(parent);
        let d = self.display_of(id);

        // Inside a real row: non-cell children live in an anonymous cell.
        if matches!(parent_display, Some(Display::TableRow)) {
            if matches!(d, Some(Display::TableCell)) || self.run_skippable(id) {
                return None;
            }
            let start = self.run_start_containing(parent, id, |c| self.needy_for_cell(c))?;
            return Some(encode_anon_cell(start));
        }

        // Inside a real table / row group: improper children live in an anonymous row,
        // and non-cells among them one level deeper in an anonymous cell.
        if matches!(
            parent_display,
            Some(Display::Table | Display::TableRowGroup | Display::TableHeaderGroup | Display::TableFooterGroup)
        ) {
            let pd = parent_display.clone().unwrap_or(Display::Table);
            if !self.needy_for_row(id, &pd) {
                return None;
            }
            let rstart = self.run_start_containing(parent, id, |c| self.needy_for_row(c, &pd))?;
            if matches!(d, Some(Display::TableCell)) {
                return Some(encode_anon_row(rstart));
            }
            let rmembers = self.anon_box_members(encode_anon_row(rstart));
            let cstart = self.sub_run_start(&rmembers, id, |c| self.needy_for_cell(c));
            return Some(encode_anon_cell(cstart));
        }

        // No table context at all: table-internal children live inside an anonymous table.
        let table_needy =
            |c: NodeId| self.display_of(c).is_some_and(|dd| needs_table_parent(&dd, parent_display.as_ref()));
        if !table_needy(id) {
            return None;
        }
        let tstart = self.run_start_containing(parent, id, table_needy)?;
        if proper_table_child(d.as_ref()) {
            return Some(encode_anon_table(tstart));
        }
        let tmembers = self.anon_box_members(encode_anon_table(tstart));
        let rstart = self.sub_run_start(&tmembers, id, |c| self.needy_for_row(c, &Display::Table));
        if matches!(d, Some(Display::TableCell)) {
            return Some(encode_anon_row(rstart));
        }
        let rmembers = self.anon_box_members(encode_anon_row(rstart));
        let cstart = self.sub_run_start(&rmembers, id, |c| self.needy_for_cell(c));
        Some(encode_anon_cell(cstart))
    }

    /// The prefix of `members` starting at `first` for which `pred` holds contiguously.
    fn members_sub_run(&self, members: &[NodeId], first: NodeId, pred: impl Fn(NodeId) -> bool) -> Vec<NodeId> {
        let Some(i) = members.iter().position(|&m| m == first) else {
            return vec![first];
        };
        let mut out = vec![first];
        for &m in &members[i + 1..] {
            if pred(m) {
                out.push(m);
            } else {
                break;
            }
        }
        out
    }

    /// Members of the row-level run containing `id`. In a real table/row-group the run is
    /// collected over the raw siblings; inside an anonymous table it is BOUNDED by the
    /// table's own member run - the broad "improper child" predicate must never leak past
    /// the anonymous table and absorb ordinary siblings (`a <cell/><cell/> d`).
    fn row_run_members_for(&self, id: NodeId) -> Vec<NodeId> {
        let Some(parent) = self.doc.parent(id) else {
            return vec![id];
        };
        let parent_display = self.display_of(parent);
        if matches!(
            parent_display,
            Some(Display::Table | Display::TableRowGroup | Display::TableHeaderGroup | Display::TableFooterGroup)
        ) {
            let pd = parent_display.clone().unwrap_or(Display::Table);
            let rstart = self
                .run_start_containing(parent, id, |c| self.needy_for_row(c, &pd))
                .unwrap_or(id);
            return self.run_members(rstart, |c| self.needy_for_row(c, &pd));
        }
        let table_needy =
            |c: NodeId| self.display_of(c).is_some_and(|d| needs_table_parent(&d, parent_display.as_ref()));
        let tstart = self.run_start_containing(parent, id, table_needy).unwrap_or(id);
        let tmembers = self.run_members(tstart, table_needy);
        let rstart = self.sub_run_start(&tmembers, id, |c| self.needy_for_row(c, &Display::Table));
        self.members_sub_run(&tmembers, rstart, |c| self.needy_for_row(c, &Display::Table))
    }

    /// The real members of a synthetic anonymous box's run (the wrapper's flavour decides
    /// the run predicate).
    fn anon_box_members(&self, anon: NodeId) -> Vec<NodeId> {
        let raw = u64::from(anon);
        let first = decode_anon_box(anon);
        let Some(parent) = self.doc.parent(first) else {
            return vec![first];
        };
        let parent_display = self.display_of(parent);

        if is_anon_table_id(raw) {
            return self.run_members(first, |id| {
                self.display_of(id).is_some_and(|d| needs_table_parent(&d, parent_display.as_ref()))
            });
        }

        if is_anon_row_id(raw) {
            return self.row_run_members_for(first);
        }

        // Anonymous cell: bounded by the enclosing row's members unless the parent is a
        // real row (then the raw sibling run is the row's interior).
        if matches!(parent_display, Some(Display::TableRow)) {
            return self.run_members(first, |id| self.needy_for_cell(id));
        }
        let rmembers = self.row_run_members_for(first);
        self.members_sub_run(&rmembers, first, |c| self.needy_for_cell(c))
    }

    /// HTML `cellspacing` (on the table -> border-spacing) and `cellpadding` (on the table ->
    /// in-table td/th padding, defaulting to 1px like WebKit's
    /// `HTMLTableCellElement::additionalPresentationAttributeStyle` - see the UA sheet comment
    /// at `td:not(table td)`). Returns `None` whenever an author declaration exists for the
    /// property: author styles always beat presentational markup, UA rules never do.
    fn table_attr_hint(
        &self,
        id: NodeId,
        prop: &StyleProperty,
        map: &<C::CssSystem as CssSystem>::PropertyMap,
    ) -> Option<Value> {
        let author_declared = |name: &str| {
            <_ as CssPropertyMap<C::CssSystem>>::get(map, name)
                .and_then(|p| p.winning_origin())
                .is_some_and(|o| matches!(o, CssOrigin::Author))
        };
        let attr_px = |node: NodeId, attr: &str| -> Option<f32> {
            self.doc
                .attributes(node)?
                .get(attr)?
                .trim()
                .parse::<f32>()
                .ok()
                .map(|v| v.max(0.0))
        };

        match prop {
            StyleProperty::BorderSpacingX | StyleProperty::BorderSpacingY => {
                if !self.doc.tag_name(id).is_some_and(|t| t.eq_ignore_ascii_case("table")) {
                    return None;
                }
                if author_declared("border-spacing") {
                    return None;
                }
                attr_px(id, "cellspacing").map(|v| Value::Unit(v, Unit::Px))
            }
            StyleProperty::PaddingTop
            | StyleProperty::PaddingRight
            | StyleProperty::PaddingBottom
            | StyleProperty::PaddingLeft => {
                if !self
                    .doc
                    .tag_name(id)
                    .is_some_and(|t| t.eq_ignore_ascii_case("td") || t.eq_ignore_ascii_case("th"))
                {
                    return None;
                }
                // The hint applies to in-table cells only; a parentless td keeps the UA default.
                let mut table = None;
                let mut cur = self.doc.parent(id);
                while let Some(p) = cur {
                    if self.doc.tag_name(p).is_some_and(|t| t.eq_ignore_ascii_case("table")) {
                        table = Some(p);
                        break;
                    }
                    cur = self.doc.parent(p);
                }
                let table = table?;
                if author_declared(prop.css_name()) || author_declared("padding") {
                    return None;
                }
                Some(Value::Unit(attr_px(table, "cellpadding").unwrap_or(1.0), Unit::Px))
            }
            _ => None,
        }
    }

    /// Bridges a computed `PropertyMap` to a single `Value`, shared by real elements and
    /// pseudo-elements. Handles the `text-decoration` / `background[-image]` shorthands and
    /// `currentColor`. `id` is only used to resolve `currentColor` against the node's `color`.
    fn style_from_map(
        &self,
        id: NodeId,
        prop: &StyleProperty,
        map: &<C::CssSystem as CssSystem>::PropertyMap,
    ) -> Option<Value> {
        let css_name = prop.css_name();

        // For `text-decoration-line`, check the `text-decoration` shorthand FIRST when it
        // is `none` (the shorthand is stored under its own key, not expanded to longhands).
        if matches!(prop, StyleProperty::TextDecorationLine) {
            if let Some(p) = <_ as CssPropertyMap<C::CssSystem>>::get(map, "text-decoration") {
                if p.is_none() {
                    return Some(Value::Keyword(intern("none")));
                }
                if let Some(s) = p.as_string() {
                    if s == "none" || s == "initial" || s == "unset" {
                        return Some(Value::Keyword(intern("none")));
                    }
                    if s.contains("underline") {
                        return Some(Value::Keyword(intern("underline")));
                    }
                    if s.contains("line-through") {
                        return Some(Value::Keyword(intern("line-through")));
                    }
                }
            }
        }

        // background-image: accept the `background-image` longhand or a `url(...)` inside the
        // `background` shorthand. The returned keyword is the unresolved URL.
        if matches!(prop, StyleProperty::BackgroundImage) {
            for key in ["background-image", "background"] {
                if let Some(p) = <_ as CssPropertyMap<C::CssSystem>>::get(map, key) {
                    if let Some(url) = css_property_url::<C::CssSystem>(p) {
                        return Some(Value::Keyword(intern(&url)));
                    }
                }
            }
            return None;
        }

        // `currentColor` on any color property except `color` itself resolves to the node's
        // computed `color`. (`color: currentColor` would be self-referential, so it is left to
        // resolve via the normal cascade.)
        if matches!(
            prop,
            StyleProperty::BackgroundColor
                | StyleProperty::BorderTopColor
                | StyleProperty::BorderRightColor
                | StyleProperty::BorderBottomColor
                | StyleProperty::BorderLeftColor
        ) {
            if let Some(p) = <_ as CssPropertyMap<C::CssSystem>>::get(map, css_name) {
                if p.as_string().is_some_and(|s| s.eq_ignore_ascii_case("currentcolor")) {
                    return Some(self.get_style(id, &StyleProperty::Color));
                }
            }
        }

        // Inset properties are modelled with logical variants, but pages usually write the
        // physical `top`/`right`/`bottom`/`left`. Accept either key (the physical aliasing is valid
        // for the default horizontal-tb, ltr writing mode this engine assumes).
        let inset_physical = match prop {
            StyleProperty::InsetBlockStart => Some("top"),
            StyleProperty::InsetBlockEnd => Some("bottom"),
            StyleProperty::InsetInlineStart => Some("left"),
            StyleProperty::InsetInlineEnd => Some("right"),
            _ => None,
        };
        if let Some(physical) = inset_physical {
            for key in [css_name, physical] {
                if let Some(p) = <_ as CssPropertyMap<C::CssSystem>>::get(map, key) {
                    if let Some(v) = css_property_to_value::<C::CssSystem>(p, prop) {
                        return Some(v);
                    }
                }
            }
            return None;
        }

        if let Some(p) = <_ as CssPropertyMap<C::CssSystem>>::get(map, css_name) {
            if let Some(v) = css_property_to_value::<C::CssSystem>(p, prop) {
                return Some(v);
            }
        }

        // The `background` shorthand is stored under its own key and never expanded to longhands,
        // so extract the colour token from it when the longhand is absent.
        if matches!(prop, StyleProperty::BackgroundColor) {
            if let Some(p) = <_ as CssPropertyMap<C::CssSystem>>::get(map, "background") {
                if let Some((r, g, b, a)) = css_property_bg_color::<C::CssSystem>(p) {
                    return Some(Value::Color(r, g, b, a));
                }
            }
        }
        None
    }

    fn find_child_by_tag(&self, parent: NodeId, tag: &str) -> Option<NodeId> {
        self.doc
            .children(parent)
            .iter()
            .find(|&&child| self.doc.tag_name(child).is_some_and(|t| t.eq_ignore_ascii_case(tag)))
            .copied()
    }
}

impl<C> PipelineDocument for GosubDocumentAdapter<C>
where
    C: HasDocument + Send + Sync + 'static,
    C::Document: Send + Sync,
    <C::CssSystem as CssSystem>::PropertyMap: Send + Sync,
{
    fn root(&self) -> Option<NodeId> {
        self.html_node_id().or_else(|| Some(self.doc.root()))
    }

    fn children(&self, id: NodeId) -> Vec<NodeId> {
        if is_anon_table_id(u64::from(id)) {
            // The anonymous table's members may need row wrappers of their own.
            let members = self.anon_box_members(id);
            return self.wrap_anon_row_runs(Some(&Display::Table), members);
        }
        if is_anon_row_id(u64::from(id)) {
            // ...and an anonymous row's members may need cell wrappers.
            let members = self.anon_box_members(id);
            return self.wrap_anon_cell_runs(Some(&Display::TableRow), members);
        }
        if is_anon_cell_id(u64::from(id)) {
            return self.anon_box_members(id);
        }
        if is_pseudo_id(u64::from(id)) {
            let (owner, role) = decode_pseudo(id);
            // A pseudo-element's only child is its generated text (if any); text nodes are leaves.
            if role_is_text(role) {
                return Vec::new();
            }
            return match self.pseudo_box(owner, role_is_after(role)) {
                Some(pb) if pb.text.is_some() => {
                    let text_role = if role_is_after(role) {
                        ROLE_AFTER_TEXT
                    } else {
                        ROLE_BEFORE_TEXT
                    };
                    vec![encode_pseudo(owner, text_role)]
                }
                _ => Vec::new(),
            };
        }

        let mut out = Vec::new();
        // `::before` is inserted as the first child, `::after` as the last.
        if self.pseudo_box(id, false).is_some() {
            out.push(encode_pseudo(id, ROLE_BEFORE_ELEM));
        }
        out.extend(self.doc.children(id).iter().copied());
        if self.pseudo_box(id, true).is_some() {
            out.push(encode_pseudo(id, ROLE_AFTER_ELEM));
        }
        let display = self.display_of(id);
        let out = self.wrap_anon_table_runs(display.as_ref(), out);
        let out = self.wrap_anon_row_runs(display.as_ref(), out);
        self.wrap_anon_cell_runs(display.as_ref(), out)
    }

    fn node_kind(&self, id: NodeId) -> PipelineNodeKind {
        if is_anon_box_id(u64::from(id)) {
            return PipelineNodeKind::Element;
        }
        if is_pseudo_id(u64::from(id)) {
            let (_, role) = decode_pseudo(id);
            return if role_is_text(role) {
                PipelineNodeKind::Text
            } else {
                PipelineNodeKind::Element
            };
        }
        match self.doc.node_type(id) {
            GosubNodeType::TextNode => PipelineNodeKind::Text,
            GosubNodeType::CommentNode | GosubNodeType::DocTypeNode => PipelineNodeKind::Comment,
            GosubNodeType::ElementNode => PipelineNodeKind::Element,
            GosubNodeType::DocumentNode => PipelineNodeKind::Element,
        }
    }

    fn tag_name(&self, id: NodeId) -> Option<String> {
        // Pseudo-elements and anonymous table boxes have no tag name.
        if is_pseudo_id(u64::from(id)) || is_anon_box_id(u64::from(id)) {
            return None;
        }
        self.doc.tag_name(id).map(|s| s.to_string())
    }

    fn is_display_none(&self, id: NodeId) -> bool {
        matches!(
            self.get_own_style(id, &StyleProperty::Display),
            Some(Value::Display(Display::None))
        )
    }

    fn parent(&self, id: NodeId) -> Option<NodeId> {
        // Synthetic anonymous boxes: parent is the members' real parent when it provides the
        // right context, else the next synthetic wrapper up - located by finding the enclosing
        // run's start among the real parent's children.
        if is_anon_cell_id(u64::from(id)) {
            let first = decode_anon_box(id);
            let real_parent = self.doc.parent(first)?;
            if matches!(self.display_of(real_parent), Some(Display::TableRow)) {
                return Some(real_parent);
            }
            // The enclosing anonymous row wraps the row-level run containing this cell run.
            let rmembers = self.row_run_members_for(first);
            return Some(encode_anon_row(rmembers[0]));
        }
        if is_anon_row_id(u64::from(id)) {
            let first = decode_anon_box(id);
            let real_parent = self.doc.parent(first)?;
            let parent_display = self.display_of(real_parent);
            if matches!(
                parent_display,
                Some(Display::Table | Display::TableRowGroup | Display::TableHeaderGroup | Display::TableFooterGroup)
            ) {
                return Some(real_parent);
            }
            // The enclosing anonymous table wraps the table-level run containing this row run.
            let start = self
                .run_start_containing(real_parent, first, |c| {
                    self.display_of(c)
                        .is_some_and(|d| needs_table_parent(&d, parent_display.as_ref()))
                })
                .unwrap_or(first);
            return Some(encode_anon_table(start));
        }
        if is_anon_table_id(u64::from(id)) {
            return self.doc.parent(decode_anon_box(id));
        }
        // Members of a synthesized run report the wrapper as their parent, so the parent
        // chain matches the child lists children() hands out.
        if let Some(wrapper) = self.synthetic_parent_of(id) {
            return Some(wrapper);
        }
        if is_pseudo_id(u64::from(id)) {
            let (owner, role) = decode_pseudo(id);
            // Text child's parent is its pseudo-element; the pseudo-element's parent is the owner.
            return Some(if role_is_text(role) {
                encode_pseudo(
                    owner,
                    if role_is_after(role) {
                        ROLE_AFTER_ELEM
                    } else {
                        ROLE_BEFORE_ELEM
                    },
                )
            } else {
                owner
            });
        }
        self.doc.parent(id)
    }

    fn get_own_style(&self, id: NodeId, prop: &StyleProperty) -> Option<Value> {
        // An anonymous table box IS its display (table / table-row) and has no other own
        // styles; inherited properties resolve through its (real) parent.
        if let Some(d) = anon_box_display(u64::from(id)) {
            return matches!(prop, StyleProperty::Display).then(|| Value::Display(d));
        }
        // Generated content (::before / ::after) draws its styles from a separate map.
        if is_pseudo_id(u64::from(id)) {
            return self.pseudo_own_style(id, prop);
        }

        let arc = self.cached_styles(id);

        // Inline styles (from `style` attribute) have highest specificity.
        if let Some(inline) = self.inline_style_cache.lock().get(&id) {
            if let Some(v) = inline.get_own(prop) {
                return Some(v.clone());
            }
        }

        // `cellspacing`/`cellpadding` are presentational hints that sit BETWEEN origins: they
        // beat user-agent rules (notably the UA `table { border-spacing: 2px }`) but lose to
        // any author declaration, so they must be consulted before the cascaded map.
        if let Some(v) = self.table_attr_hint(id, prop, arc.as_ref()) {
            return Some(v);
        }

        if let Some(v) = self.style_from_map(id, prop, arc.as_ref()) {
            return Some(v);
        }

        // HTML presentation attributes (bgcolor, width, …) as lowest-specificity fallback.
        if let Some(attrs) = self.doc.attributes(id) {
            return crate::common::document::inline_style::html_presentation_attr(attrs, prop);
        }

        None
    }

    fn background_layers(&self, id: NodeId) -> Vec<Gradient> {
        if is_anon_box_id(u64::from(id)) {
            return Vec::new();
        }
        // Read the layers from the pseudo-element's own map, never the owner's.
        let arc = if is_pseudo_id(u64::from(id)) {
            let (owner, role) = decode_pseudo(id);
            if role_is_text(role) {
                return Vec::new();
            }
            match self.pseudo_box(owner, role_is_after(role)) {
                Some(pb) => pb.styles.clone(),
                None => return Vec::new(),
            }
        } else {
            self.cached_styles(id)
        };
        let map = arc.as_ref();

        let mut layers = Vec::new();
        for key in ["background-image", "background"] {
            if let Some(p) = <_ as CssPropertyMap<C::CssSystem>>::get(map, key) {
                layers = property_gradient_layers::<C::CssSystem>(p);
                if !layers.is_empty() {
                    break;
                }
            }
        }
        if layers.is_empty() {
            return Vec::new();
        }

        // `background-size/-position/-repeat` are per-layer lists cycled to the layer count.
        let read_groups = |key: &str| {
            <_ as CssPropertyMap<C::CssSystem>>::get(map, key)
                .map(bg_token_groups::<C::CssSystem>)
                .unwrap_or_default()
        };
        let size_groups = read_groups("background-size");
        let pos_groups = read_groups("background-position");
        let rep_groups = read_groups("background-repeat");
        let pick =
            |groups: &[Vec<BgTok>], i: usize| -> Option<usize> { (!groups.is_empty()).then(|| i % groups.len()) };

        for (i, g) in layers.iter_mut().enumerate() {
            let Some((tw, th)) = pick(&size_groups, i).and_then(|j| resolve_bg_size(&size_groups[j])) else {
                continue; // no explicit size → fill the box (no tiling)
            };
            if tw <= 0.0 || th <= 0.0 {
                continue;
            }
            let position = pick(&pos_groups, i)
                .map(|j| resolve_bg_position(&pos_groups[j]))
                .unwrap_or((0.0, 0.0));
            let repeat = pick(&rep_groups, i)
                .map(|j| resolve_bg_repeat(&rep_groups[j]))
                .unwrap_or((true, true));
            g.tiling = Some(Tiling {
                tile_size: (tw, th),
                position,
                repeat,
            });
        }

        layers.into_iter().map(Gradient::Linear).collect()
    }

    fn background_image_layout(&self, id: NodeId) -> BgImageLayout {
        if is_anon_box_id(u64::from(id)) {
            return BgImageLayout::default();
        }
        let arc = self.cached_styles(id);
        let map = arc.as_ref();

        // The shorthand carries repeat/size keywords inline (`background: url(x) no-repeat center
        // / contain`) and then the longhands are usually empty, so scan both.
        let mut keywords: Vec<String> = Vec::new();
        let mut explicit_size: Option<(f32, f32)> = None;
        let mut position: Option<(f32, f32)> = None;

        let mut scan = |key: &str, read_size: bool, read_pos: bool| {
            let Some(p) = <_ as CssPropertyMap<C::CssSystem>>::get(map, key) else {
                return;
            };
            let groups = bg_token_groups::<C::CssSystem>(p);
            let Some(group) = groups.first() else {
                return;
            };
            if read_size && explicit_size.is_none() {
                explicit_size = resolve_bg_size(group);
            }
            if read_pos && position.is_none() {
                let pos = resolve_bg_position(group);
                if pos != (0.0, 0.0) {
                    position = Some(pos);
                }
            }
            for t in group {
                if let BgTok::Kw(k) = t {
                    keywords.push(k.clone());
                }
            }
        };
        // The shorthand mixes position and size (split by `/`); reading its bare lengths as a
        // position is unreliable, so only take position/size from the dedicated longhands.
        scan("background", false, false);
        scan("background-repeat", false, false);
        scan("background-size", true, false);
        scan("background-position", false, true);

        let has = |k: &str| keywords.iter().any(|s| s == k);
        let repeat = if has("no-repeat") {
            (false, false)
        } else if has("repeat-x") {
            (true, false)
        } else if has("repeat-y") {
            (false, true)
        } else {
            (true, true)
        };
        let size = match explicit_size {
            Some((w, h)) => BgSize::Length(w, h),
            None if has("cover") => BgSize::Cover,
            None if has("contain") => BgSize::Contain,
            None => BgSize::Auto,
        };
        // A length `background-position` wins; otherwise a bare `center` centers both axes.
        let (position, center) = match position {
            Some(pos) => (pos, (false, false)),
            None if has("center") => ((0.0, 0.0), (true, true)),
            None => ((0.0, 0.0), (false, false)),
        };

        BgImageLayout {
            repeat,
            position,
            center,
            size,
        }
    }

    fn clear_style_cache(&self) {
        self.style_cache.lock().clear();
        self.inline_style_cache.lock().clear();
        self.pseudo_cache.lock().clear();
    }

    fn invalidate_style_for_nodes(&self, ids: &[NodeId]) {
        let mut cache = self.style_cache.lock();
        let mut inline_cache = self.inline_style_cache.lock();
        let mut pseudo_cache = self.pseudo_cache.lock();
        for id in ids {
            cache.remove(id);
            inline_cache.remove(id);
            // Drop both pseudo-boxes belonging to this owner.
            pseudo_cache.remove(&(*id, false));
            pseudo_cache.remove(&(*id, true));
        }
    }

    fn html_node_id(&self) -> Option<NodeId> {
        let root = self.doc.root();
        self.find_child_by_tag(root, "html")
    }

    fn body_node_id(&self) -> Option<NodeId> {
        let html = self.html_node_id().or_else(|| Some(self.doc.root()))?;
        self.find_child_by_tag(html, "body")
    }

    fn base_url(&self) -> String {
        self.doc.url().map(|u| u.to_string()).unwrap_or_default()
    }

    fn inner_html(&self, id: NodeId) -> String {
        if is_pseudo_id(u64::from(id)) || is_anon_box_id(u64::from(id)) {
            return String::new();
        }
        self.doc.write_from_node(id)
    }

    fn get_node_by_id(&self, id: NodeId) -> Option<Node> {
        // Synthetic anonymous-table wrapper: a tagless `display: table` / `table-row` element.
        if let Some(d) = anon_box_display(u64::from(id)) {
            let mut style = NodeStyle::new();
            // An anonymous table generated in INLINE context is an inline-table (CSS 2.1
            // §17.2.1). We have no inline-table display; marking the synthetic NODE
            // inline-block makes the layouter's line grouping keep it (and the whitespace
            // around it) in the line box, while the cascade via get_own_style still reports
            // `table` to the converter, lattice, and painter.
            let node_display = if matches!(d, Display::Table) {
                let parent_inline = self.parent(id).is_some_and(|p| {
                    matches!(
                        self.display_of(p),
                        None | Some(
                            Display::Inline | Display::InlineBlock | Display::InlineFlex | Display::InlineGrid
                        )
                    ) && !matches!(self.doc.node_type(p), GosubNodeType::DocumentNode)
                });
                if parent_inline {
                    Display::InlineBlock
                } else {
                    d
                }
            } else {
                d
            };
            style.set(StyleProperty::Display, Value::Display(node_display));
            return Some(Node {
                node_id: id,
                parent_id: self.parent(id),
                children: self.children(id),
                node_type: NodeType::Element(ElementData::new(String::new(), Some(AttrMap::new()), Some(style))),
            });
        }
        // Synthetic pseudo nodes: build a transient Element (the box) or Text (its content).
        if is_pseudo_id(u64::from(id)) {
            let (owner, role) = decode_pseudo(id);
            let node_type = if role_is_text(role) {
                let text = self
                    .pseudo_box(owner, role_is_after(role))
                    .and_then(|pb| pb.text.clone());
                NodeType::Text(text.unwrap_or_default())
            } else {
                // Carry the computed `display` on the synthetic element so the layouter's
                // inline-vs-block grouping (which is tag-name based and would see an empty tag)
                // treats the pseudo-element correctly. ::before/::after default to inline.
                let mut style = NodeStyle::new();
                style.set(StyleProperty::Display, self.get_style(id, &StyleProperty::Display));
                NodeType::Element(ElementData::new(String::new(), Some(AttrMap::new()), Some(style)))
            };
            return Some(Node {
                node_id: id,
                parent_id: self.parent(id),
                children: self.children(id),
                node_type,
            });
        }

        let parent_id = self.doc.parent(id);
        let children = self.doc.children(id).to_vec();

        let node_type = match self.doc.node_type(id) {
            GosubNodeType::TextNode => {
                let text = self.doc.text_value(id).unwrap_or("").to_string();
                // Text nodes carry no own style; inheritance handled by get_style() chain.
                NodeType::Text(text)
            }
            GosubNodeType::CommentNode => {
                let comment = self.doc.comment_value(id).unwrap_or("").to_string();
                NodeType::Comment(comment)
            }
            GosubNodeType::ElementNode => {
                let tag_name = self.doc.tag_name(id).unwrap_or("").to_string();
                let mut attr_map = AttrMap::new();
                if let Some(attrs) = self.doc.attributes(id) {
                    for (k, v) in attrs {
                        attr_map.set(k, v);
                    }
                }
                // Styles are normally read via `doc.get_own_style()`, but the layouter's
                // inline-vs-block grouping reads the local NodeStyle only - so carry the cascaded
                // `display` onto the Node for rules like `figcaption b { display: block }`.
                // Only when the cascade assigned one: `None` preserves the intrinsic tag-name
                // fallback, since the incomplete UA stylesheet makes get_style()'s `inline`
                // initial value the wrong answer here.
                let styles = self.get_own_style(id, &StyleProperty::Display).map(|display| {
                    let mut style = NodeStyle::new();
                    style.set(StyleProperty::Display, display);
                    style
                });
                let element_data = ElementData::new(tag_name, Some(attr_map), styles);
                NodeType::Element(element_data)
            }
            _ => return None,
        };

        Some(Node {
            node_id: id,
            parent_id,
            children,
            node_type,
        })
    }
}

// ── Helpers used by the bridge ────────────────────────────────────────────────

/// The `border-*-style` governing `prop`, or None if `prop` isn't a border width.
fn border_width_peer_style(prop: &StyleProperty) -> Option<StyleProperty> {
    Some(match prop {
        StyleProperty::BorderTopWidth => StyleProperty::BorderTopStyle,
        StyleProperty::BorderRightWidth => StyleProperty::BorderRightStyle,
        StyleProperty::BorderBottomWidth => StyleProperty::BorderBottomStyle,
        StyleProperty::BorderLeftWidth => StyleProperty::BorderLeftStyle,
        _ => return None,
    })
}

fn str_to_border_style(s: &str) -> BorderStyle {
    match s {
        "hidden" => BorderStyle::Hidden,
        "solid" => BorderStyle::Solid,
        "dashed" => BorderStyle::Dashed,
        "dotted" => BorderStyle::Dotted,
        "double" => BorderStyle::Double,
        "groove" => BorderStyle::Groove,
        "ridge" => BorderStyle::Ridge,
        "inset" => BorderStyle::Inset,
        "outset" => BorderStyle::Outset,
        _ => BorderStyle::None,
    }
}

/// Intercepts system color keywords before the normal parse path, since `RgbColor::from` returns
/// black for any string it doesn't recognise.
fn css_system_color(name: &str) -> Option<(u8, u8, u8, u8)> {
    match name.cow_to_ascii_lowercase().as_ref() {
        // Highlight / mark
        "mark" => Some((255, 255, 0, 255)),
        "marktext" => Some((0, 0, 0, 255)),
        // Form fields
        "field" | "canvas" => Some((255, 255, 255, 255)),
        "fieldtext" | "canvastext" | "buttontext" | "graytext" => Some((0, 0, 0, 255)),
        "buttonface" | "threedface" => Some((240, 240, 240, 255)),
        "buttonborder" | "threedlightshadow" | "threedhighlight" => Some((160, 160, 160, 255)),
        // Selection / highlights
        "highlight" | "selecteditem" | "activecaption" => Some((0, 120, 215, 255)),
        "highlighttext" | "selecteditemtext" | "captiontext" => Some((255, 255, 255, 255)),
        // Links
        "linktext" | "activetext" => Some((0, 0, 238, 255)),
        "visitedtext" => Some((85, 26, 139, 255)),
        // Misc
        "accentcolor" => Some((0, 120, 215, 255)),
        "accentcolortext" => Some((255, 255, 255, 255)),
        "window" | "appworkspace" | "scrollbar" | "background" | "menu" => Some((240, 240, 240, 255)),
        "windowtext" | "menutext" | "infotext" | "inactivecaptiontext" => Some((0, 0, 0, 255)),
        _ => None,
    }
}
