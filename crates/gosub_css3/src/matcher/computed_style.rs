//! Turn a cascaded property map into the typed [`ComputedStyle`] the render pipeline reads.
//!
//! This is the one place a CSS value becomes a typed field. Everything a consumer used to do
//! for itself - reading a colour keyword, deciding what `currentColor` means, working out what
//! a `font-size` keyword is worth in pixels, zeroing a border whose style is `none` - happens
//! here, once per element, against the parent's already-computed struct.
//!
//! What is deliberately *not* here: percentages. A percentage needs a containing block, so it
//! travels on in [`LengthPercentage`] and layout settles it.
//!
//! Some of what this does belongs further up, in the crate's own computed stage, and will move
//! there: the system colours, the `font-size` keyword scale, and the `ch`/`ex`/`lh`/`ic`
//! approximations are all computed-value questions the cascade could answer for every consumer
//! rather than for this one. They are here for now because moving them would change the value
//! the map reports, and this step changes nothing a map says.

use cow_utils::CowUtils as _;
use std::sync::Arc;

use gosub_interface::style::{
    AlignValue, BorderCollapse, BorderStyle, BoxSizing, CaptionSide, Clear, Color, ComputedStyle, Display,
    FlexDirection, FlexWrap, Float, FontStyle, FontWeight, GridAutoFlow, LengthPercentage, LengthPercentageAuto,
    LetterSpacing, LineHeight, Overflow, Position, Prop, TableLayout, TextAlign, TextDecorationLine, TextTransform,
    TextWrap, VerticalAlign, WhiteSpace, ZIndex,
};

use crate::matcher::property_ids::{LonghandId, PropertyId, ShorthandId};
use crate::matcher::styling::CssProperties;
use crate::stylesheet::CssValue;

const fn longhand(id: LonghandId) -> PropertyId {
    PropertyId::Longhand(id)
}

const fn shorthand(id: ShorthandId) -> PropertyId {
    PropertyId::Shorthand(id)
}

/// The value of one property on this element, or `None` when the map has no entry for it or the
/// entry never resolved to anything.
fn value(map: &CssProperties, id: PropertyId) -> Option<&CssValue> {
    match map.get_id(id) {
        Some(property) if !matches!(property.actual, CssValue::None) => Some(&property.actual),
        _ => None,
    }
}

// ── Value readers, one per shape the cascade produces ────────────────────────

fn as_string(value: &CssValue) -> Option<&str> {
    match value {
        CssValue::String(string) => Some(string),
        _ => None,
    }
}

#[expect(clippy::cast_possible_truncation, reason = "style values are carried at f32")]
fn as_number(value: &CssValue) -> Option<f32> {
    match value {
        CssValue::Number(number, _) => Some(*number as f32),
        // A bare `0` parses to its own variant; it is still the number zero.
        CssValue::Zero => Some(0.0),
        _ => None,
    }
}

#[expect(clippy::cast_possible_truncation, reason = "style values are carried at f32")]
fn as_percentage(value: &CssValue) -> Option<f32> {
    match value {
        CssValue::Percentage(pct) => Some(*pct as f32),
        _ => None,
    }
}

#[expect(clippy::cast_possible_truncation, reason = "style values are carried at f32")]
fn as_unit(value: &CssValue) -> Option<(f32, &str)> {
    match value {
        CssValue::Unit(number, unit) => Some((*number as f32, unit)),
        _ => None,
    }
}

fn as_list(value: &CssValue) -> Option<&[CssValue]> {
    match value {
        CssValue::List(list) => Some(list),
        _ => None,
    }
}

fn as_function(value: &CssValue) -> Option<(&str, &[CssValue])> {
    match value {
        CssValue::Function(name, args) => Some((name, args)),
        _ => None,
    }
}

/// How much of the element's own font-size one unit of `unit` is worth, for the font-relative
/// units the computed stage leaves unresolved.
///
/// Real font metrics would answer these; these factors are the stand-ins that have always been
/// used. `ch` is the advance of "0", `ex` the x-height, `lh` a line box. 0.55 rather than the
/// spec's 0.5 fallback for `ch`: proportional faces sit nearer 0.52-0.6em, and 0.5 makes a
/// `max-width: 17ch` wrap a line early.
fn font_relative_factor(unit: &str) -> Option<f32> {
    match unit {
        "em" => Some(1.0),
        "ch" => Some(0.55),
        "ex" => Some(0.5),
        "ic" => Some(1.0),
        "lh" => Some(1.4),
        _ => None,
    }
}

/// A `<length-percentage>` in the element's own font-size context.
///
/// `em` and `rem` are already pixels by the time a value gets here - the computed stage does
/// them, which is where css-values says they belong. What arrives unresolved is the handful of
/// units nothing can give a value to without font metrics.
fn length_percentage(value: &CssValue, font_size: f32) -> Option<LengthPercentage> {
    if let Some((number, unit)) = as_unit(value) {
        if let Some(factor) = font_relative_factor(unit) {
            return Some(LengthPercentage::Px(number * factor * font_size));
        }
        return Some(LengthPercentage::Px(value.unit_to_px()));
    }
    if let Some(pct) = as_percentage(value) {
        return Some(LengthPercentage::Percent(pct));
    }
    // A bare number in a length slot is read as pixels, which is what `top: 0` and `margin: 0`
    // rely on.
    as_number(value).map(LengthPercentage::Px)
}

/// The same, plus `auto`. Any keyword that is not a length is `auto` here: none of the
/// consumers act on one, and `auto` is what each of them falls back to.
fn length_percentage_auto(value: &CssValue, font_size: f32) -> LengthPercentageAuto {
    match length_percentage(value, font_size) {
        Some(LengthPercentage::Px(px)) => LengthPercentageAuto::Px(px),
        Some(LengthPercentage::Percent(pct)) => LengthPercentageAuto::Percent(pct),
        None => LengthPercentageAuto::Auto,
    }
}

/// A plain px length, for the properties whose value space holds nothing else. A percentage is
/// not one of them, so it leaves the property unset rather than being read as pixels.
fn length_px(value: &CssValue, font_size: f32) -> Option<f32> {
    match length_percentage(value, font_size)? {
        LengthPercentage::Px(px) => Some(px),
        LengthPercentage::Percent(_) => None,
    }
}

// ── Colours ──────────────────────────────────────────────────────────────────

/// The system colour a keyword names.
///
/// `RgbColor::try_from_str` answers the named colours, and these are not among them, so without
/// this a `buttonface` would fall through to opaque black. Belongs in the crate's computed
/// stage, where every other colour keyword resolves; kept here for now so the map keeps saying
/// exactly what it says today.
fn system_color(name: &str) -> Option<Color> {
    let rgba = |r, g, b, a| Some(Color::rgba(r, g, b, a));
    match name.cow_to_ascii_lowercase().as_ref() {
        // Highlight / mark
        "mark" => rgba(255, 255, 0, 255),
        "marktext" => rgba(0, 0, 0, 255),
        // Form fields
        "field" | "canvas" => rgba(255, 255, 255, 255),
        "fieldtext" | "canvastext" | "buttontext" | "graytext" => rgba(0, 0, 0, 255),
        "buttonface" | "threedface" => rgba(240, 240, 240, 255),
        "buttonborder" | "threedlightshadow" | "threedhighlight" => rgba(160, 160, 160, 255),
        // Gosub blue; the cascade strips the vendor prefix before we see it.
        "-webkit-focus-ring-color" | "focus-ring-color" => rgba(0x23, 0x82, 0xeb, 255),
        // Selection / highlights
        "highlight" | "selecteditem" | "activecaption" => rgba(0, 120, 215, 255),
        "highlighttext" | "selecteditemtext" | "captiontext" => rgba(255, 255, 255, 255),
        // Links
        "linktext" | "activetext" => rgba(0, 0, 238, 255),
        "visitedtext" => rgba(85, 26, 139, 255),
        // Misc
        "accentcolor" => rgba(0, 120, 215, 255),
        "accentcolortext" => rgba(255, 255, 255, 255),
        "window" | "appworkspace" | "scrollbar" | "background" | "menu" => rgba(240, 240, 240, 255),
        "windowtext" | "menutext" | "infotext" | "inactivecaptiontext" => rgba(0, 0, 0, 255),
        _ => None,
    }
}

/// Whether a keyword is one of the CSS-wide ones.
///
/// The cascade resolves all five before a value gets here, so this only catches a value built
/// by hand. Reading one as a colour would paint the element black, which is why it is checked.
fn is_css_wide(keyword: &str) -> bool {
    ["initial", "unset", "revert", "revert-layer", "inherit"]
        .iter()
        .any(|name| keyword.eq_ignore_ascii_case(name))
}

/// The colour a value names, or `None` when it names none - in which case the property keeps
/// whatever it would have had.
#[expect(clippy::cast_possible_truncation, reason = "a colour channel is a byte")]
#[expect(clippy::cast_sign_loss, reason = "a colour channel is never negative")]
fn color(value: &CssValue) -> Option<Color> {
    if let Some(keyword) = as_string(value) {
        if is_css_wide(keyword) {
            return None;
        }
        if let Some(system) = system_color(keyword) {
            return Some(system);
        }
    }
    let rgb = value.to_color()?;
    Some(Color::rgba(rgb.r as u8, rgb.g as u8, rgb.b as u8, rgb.a as u8))
}

/// Whether a value is the `currentcolor` keyword, which stands for the element's own `color`.
fn is_current_color(value: &CssValue) -> bool {
    as_string(value).is_some_and(|keyword| keyword.eq_ignore_ascii_case("currentcolor"))
}

/// The first colour token of a `background` shorthand (`#fff url(...) no-repeat`).
fn shorthand_background_color(value: &CssValue) -> Option<Color> {
    if let Some(keyword) = as_string(value) {
        if let Some(system) = system_color(keyword) {
            return Some(system);
        }
    }
    if let Some(direct) = color(value) {
        return Some(direct);
    }
    as_list(value)?.iter().find_map(|item| match item {
        CssValue::Color(_) => color(item),
        CssValue::String(keyword) => system_color(keyword),
        _ => None,
    })
}

// ── `url()` ──────────────────────────────────────────────────────────────────

/// The first `url(...)` target in a value tree, dequoted.
fn first_url(value: &CssValue) -> Option<&str> {
    if let Some((name, args)) = as_function(value) {
        if name.eq_ignore_ascii_case("url") {
            if let Some(target) = args.iter().find_map(as_string) {
                return Some(target.trim_matches(['"', '\'']));
            }
        }
    }
    as_list(value)?.iter().find_map(first_url)
}

// ── Grid text ────────────────────────────────────────────────────────────────

/// One grid track-list value back as CSS text (`1fr`, `minmax(100px, 1fr)`, ...), which is the
/// form the layouter's track parser takes.
fn grid_text(value: &CssValue) -> String {
    if let Some(string) = as_string(value) {
        return string.to_string();
    }
    if let Some((number, unit)) = as_unit(value) {
        return format!("{number}{unit}");
    }
    if let Some(pct) = as_percentage(value) {
        return format!("{pct}%");
    }
    if matches!(value, CssValue::Comma) {
        return ",".to_string();
    }
    if let Some((name, args)) = as_function(value) {
        return format!("{name}({})", grid_args(args));
    }
    if let Some(list) = as_list(value) {
        return list.iter().map(grid_text).collect::<Vec<_>>().join(" ");
    }
    if let Some(number) = as_number(value) {
        return format!("{number}");
    }
    String::new()
}

/// Grid function arguments (`repeat(3, 1fr)`): commas as `, `, everything else space-separated.
fn grid_args(args: &[CssValue]) -> String {
    let mut out = String::new();
    for arg in args {
        if matches!(arg, CssValue::Comma) {
            out.push_str(", ");
        } else {
            if !out.is_empty() && !out.ends_with(' ') {
                out.push(' ');
            }
            out.push_str(&grid_text(arg));
        }
    }
    out.trim().to_string()
}

/// A `grid-template-*` track list as one string, covering every shape it arrives in.
fn grid_track_list(value: &CssValue) -> Option<String> {
    if let Some(string) = as_string(value) {
        return Some(string.to_string());
    }
    if let Some((name, args)) = as_function(value) {
        return Some(format!("{name}({})", grid_args(args)));
    }
    if let Some(list) = as_list(value) {
        return Some(list.iter().map(grid_text).collect::<Vec<_>>().join(" "));
    }
    if let Some((number, unit)) = as_unit(value) {
        return Some(format!("{number}{unit}"));
    }
    as_percentage(value).map(|pct| format!("{pct}%"))
}

/// A grid placement (`content`, `1 / 3`) as one string.
fn grid_placement(value: &CssValue) -> Option<String> {
    if let Some(string) = as_string(value) {
        return Some(string.to_string());
    }
    Some(as_list(value)?.iter().map(grid_text).collect::<Vec<_>>().join(" "))
}

/// `grid-template-areas`: one quoted string per row, joined with a character an area name
/// cannot contain, since the row boundaries carry the meaning.
fn grid_areas(value: &CssValue) -> Option<String> {
    let rows: Vec<&str> = match as_list(value) {
        Some(list) => list.iter().filter_map(as_string).collect(),
        None => vec![as_string(value)?],
    };
    Some(
        rows.iter()
            .map(|row| row.trim_matches(['"', '\'']))
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

// ── Keyword tables ───────────────────────────────────────────────────────────

fn display_of(keyword: &str) -> Display {
    match keyword {
        "block" => Display::Block,
        "inline" => Display::Inline,
        "inline-block" => Display::InlineBlock,
        "none" => Display::None,
        "flex" => Display::Flex,
        "inline-flex" => Display::InlineFlex,
        "grid" => Display::Grid,
        "inline-grid" => Display::InlineGrid,
        "table" => Display::Table,
        "inline-table" => Display::InlineTable,
        "table-caption" => Display::TableCaption,
        "table-cell" => Display::TableCell,
        "table-footer-group" => Display::TableFooterGroup,
        "table-header-group" => Display::TableHeaderGroup,
        "table-row" => Display::TableRow,
        "table-row-group" => Display::TableRowGroup,
        "table-column" => Display::TableColumn,
        "table-column-group" => Display::TableColumnGroup,
        // `contents` and `list-item` among others: no box type of their own here, and a block
        // box is the closest this engine has.
        _ => Display::Block,
    }
}

fn border_style_of(keyword: &str) -> BorderStyle {
    match keyword {
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

fn align_of(keyword: &str) -> AlignValue {
    match keyword {
        "normal" => AlignValue::Normal,
        "auto" => AlignValue::Auto,
        "stretch" => AlignValue::Stretch,
        "center" => AlignValue::Center,
        "start" => AlignValue::Start,
        "end" => AlignValue::End,
        "flex-start" => AlignValue::FlexStart,
        "flex-end" => AlignValue::FlexEnd,
        "baseline" => AlignValue::Baseline,
        "space-between" => AlignValue::SpaceBetween,
        "space-around" => AlignValue::SpaceAround,
        "space-evenly" => AlignValue::SpaceEvenly,
        "legacy" => AlignValue::Legacy,
        _ => AlignValue::Other,
    }
}

/// The absolute `font-size` keywords, in px. The CSS scale with `medium` at 16px.
fn absolute_font_size(keyword: &str) -> Option<f32> {
    match keyword {
        "xx-small" => Some(9.0),
        "x-small" => Some(10.0),
        "small" => Some(13.0),
        "medium" => Some(16.0),
        "large" => Some(18.0),
        "x-large" => Some(24.0),
        "xx-large" => Some(32.0),
        "xxx-large" => Some(48.0),
        _ => None,
    }
}

// ── The conversion ───────────────────────────────────────────────────────────

/// The initial `border-*-width` and `outline-width`: `medium`, which every browser draws as
/// 3px. It only survives where the matching style says a border is drawn at all.
const MEDIUM_BORDER_WIDTH: f32 = 3.0;

/// The `font-size` a bare generic `monospace` family defaults to.
///
/// Chrome and Firefox both do this. They keep the `medium` keyword's identity through
/// inheritance and re-evaluate it per family; this approximates that by applying the quirk only
/// where nothing in the ancestor chain ever said how big the text should be.
const MONOSPACE_DEFAULT_FONT_SIZE: f32 = 13.0;

/// Read one property and, when it says something, write it to a field and record that the
/// element's own cascade had a value for it.
macro_rules! apply {
    ($map:expr, $style:expr, $id:expr, $prop:ident, $($path:ident).+, $read:expr) => {
        if let Some(read_value) = value($map, $id).and_then($read) {
            $style.$($path).+ = read_value;
            $style.declared.set(Prop::$prop);
        }
    };
}

/// This element's [`ComputedStyle`], given its cascaded map and its parent's struct.
///
/// The parent is what every inherited property falls back to and what `currentColor` and the
/// relative `font-size` keywords are measured against, so styles resolve top-down - which is
/// the order the cascade itself runs in.
#[must_use]
pub fn computed_style(map: &CssProperties, parent: Option<&ComputedStyle>) -> ComputedStyle {
    // A text node has no map of its own, and nothing else can reach one: it takes exactly what
    // it inherits. Worth its own answer because half the nodes on a page are text.
    if map.is_empty() {
        return ComputedStyle::inherit_from(parent);
    }

    let mut style = ComputedStyle::inherit_from(parent);

    resolve_font(map, &mut style, parent);
    let font_size = style.inherited.font_size;

    resolve_inherited(map, &mut style, font_size);
    resolve_box(map, &mut style);
    resolve_sizes(map, &mut style, font_size);
    resolve_borders(map, &mut style, font_size);
    resolve_outline(map, &mut style, font_size);
    resolve_background(map, &mut style);
    resolve_insets(map, &mut style, font_size);
    resolve_flex(map, &mut style, font_size);
    resolve_grid(map, &mut style);

    style
}

/// `font-family` and `font-size`, which everything else is measured against.
fn resolve_font(map: &CssProperties, style: &mut ComputedStyle, parent: Option<&ComputedStyle>) {
    apply!(
        map,
        style,
        longhand(LonghandId::FontFamily),
        FontFamily,
        inherited.font_family,
        font_family
    );

    let parent_font_size = parent.map_or(16.0, |parent| parent.inherited.font_size);

    let Some(declared) = value(map, longhand(LonghandId::FontSize)) else {
        // Nothing anywhere up the chain declared a size, so a bare generic `monospace` family
        // gets the smaller default browsers give it.
        if !style.inherited.font_size_declared_in_chain && family_is_monospace(&style.inherited.font_family) {
            style.inherited.font_size = MONOSPACE_DEFAULT_FONT_SIZE;
        }
        return;
    };
    style.inherited.font_size_declared_in_chain = true;
    style.declared.set(Prop::FontSize);

    // An `em` on `font-size` is a multiple of the *parent's* size, which is the basis the
    // cascade already resolved it against. What is left is the keywords, and a percentage on
    // the off chance the computed stage did not settle it.
    style.inherited.font_size = if let Some((number, unit)) = as_unit(declared) {
        match font_relative_factor(unit) {
            Some(factor) => number * factor * parent_font_size,
            None => declared.unit_to_px(),
        }
    } else if let Some(pct) = as_percentage(declared) {
        parent_font_size * pct / 100.0
    } else if let Some(number) = as_number(declared) {
        number
    } else if let Some(keyword) = as_string(declared) {
        match absolute_font_size(keyword) {
            Some(px) => px,
            // The relative keywords step by the spec's suggested 1.2 factor.
            None if keyword.eq_ignore_ascii_case("smaller") => parent_font_size / 1.2,
            None if keyword.eq_ignore_ascii_case("larger") => parent_font_size * 1.2,
            // A keyword nothing here knows: the readers all fall back to the initial size.
            None => 16.0,
        }
    } else {
        16.0
    };
}

/// The family list as written, so the font system can walk it. A list arrives flat - `DejaVu
/// Sans` is two identifier tokens - so adjacent tokens are rejoined with a space and only a
/// comma separates one family from the next.
fn font_family(value: &CssValue) -> Option<Arc<str>> {
    if let Some(single) = as_string(value) {
        return Some(Arc::from(single));
    }
    let list = as_list(value)?;
    let mut names = String::new();
    let mut need_space = false;
    for item in list {
        if matches!(item, CssValue::Comma) {
            names.push_str(", ");
            need_space = false;
            continue;
        }
        let Some(name) = as_string(item) else { continue };
        if need_space {
            names.push(' ');
        }
        names.push_str(name);
        need_space = true;
    }
    (!names.is_empty()).then(|| Arc::from(names.as_str()))
}

/// Whether the first family named is the bare generic `monospace`.
fn family_is_monospace(family: &str) -> bool {
    family
        .split(',')
        .next()
        .is_some_and(|first| first.trim().eq_ignore_ascii_case("monospace"))
}

fn resolve_inherited(map: &CssProperties, style: &mut ComputedStyle, font_size: f32) {
    apply!(map, style, longhand(LonghandId::Color), Color, inherited.color, color);

    apply!(
        map,
        style,
        longhand(LonghandId::FontStyle),
        FontStyle,
        inherited.font_style,
        |value| as_string(value).map(|keyword| match keyword {
            "italic" => FontStyle::Italic,
            "oblique" => FontStyle::Oblique,
            _ => FontStyle::Normal,
        })
    );

    apply!(
        map,
        style,
        longhand(LonghandId::FontWeight),
        FontWeight,
        inherited.font_weight,
        |value| {
            if let Some(number) = as_number(value) {
                return Some(FontWeight::Number(number));
            }
            as_string(value).map(|keyword| match keyword {
                "bold" => FontWeight::Bold,
                "bolder" => FontWeight::Bolder,
                "lighter" => FontWeight::Lighter,
                _ => FontWeight::Normal,
            })
        }
    );

    // A percentage `line-height` is a fraction of the element's own font-size, which is exactly
    // what an `em` means here, so both land as pixels. A unitless number stays a number: it
    // inherits as a multiplier rather than as the length it happens to be worth here.
    apply!(
        map,
        style,
        longhand(LonghandId::LineHeight),
        LineHeight,
        inherited.line_height,
        |value| {
            if let Some((number, unit)) = as_unit(value) {
                return Some(LineHeight::Px(match font_relative_factor(unit) {
                    Some(factor) => number * factor * font_size,
                    None => value.unit_to_px(),
                }));
            }
            if let Some(pct) = as_percentage(value) {
                return Some(LineHeight::Px(font_size * pct / 100.0));
            }
            if let Some(number) = as_number(value) {
                return Some(LineHeight::Number(number));
            }
            // `normal`, and anything else that is not a length: the font metrics decide.
            as_string(value).map(|_| LineHeight::Normal)
        }
    );

    apply!(
        map,
        style,
        longhand(LonghandId::TextAlign),
        TextAlign,
        inherited.text_align,
        |value| as_string(value).map(|keyword| match keyword {
            "right" => TextAlign::Right,
            // `-webkit-center` is what the HTML rendering spec's user-agent sheet gives
            // `<caption>`; it is plain centring.
            "center" | "-webkit-center" => TextAlign::Center,
            "justify" => TextAlign::Justify,
            "start" => TextAlign::Start,
            "end" => TextAlign::End,
            "match-parent" => TextAlign::MatchParent,
            _ => TextAlign::Left,
        })
    );

    apply!(
        map,
        style,
        longhand(LonghandId::TextTransform),
        TextTransform,
        inherited.text_transform,
        |value| as_string(value).map(|keyword| match keyword {
            "uppercase" => TextTransform::Uppercase,
            "lowercase" => TextTransform::Lowercase,
            "capitalize" => TextTransform::Capitalize,
            _ => TextTransform::None,
        })
    );

    resolve_text_decoration(map, style);

    apply!(
        map,
        style,
        longhand(LonghandId::WhiteSpace),
        WhiteSpace,
        inherited.white_space,
        |value| as_string(value).map(|keyword| match keyword {
            "pre" => WhiteSpace::Pre,
            "nowrap" => WhiteSpace::NoWrap,
            "pre-wrap" => WhiteSpace::PreWrap,
            "pre-line" => WhiteSpace::PreLine,
            "break-spaces" => WhiteSpace::BreakSpaces,
            _ => WhiteSpace::Normal,
        })
    );

    apply!(
        map,
        style,
        longhand(LonghandId::LetterSpacing),
        LetterSpacing,
        inherited.letter_spacing,
        |value| Some(match length_percentage(value, font_size) {
            Some(length) => LetterSpacing::Length(length),
            None => LetterSpacing::Normal,
        })
    );

    apply!(
        map,
        style,
        longhand(LonghandId::CaptionSide),
        CaptionSide,
        inherited.caption_side,
        |value| as_string(value).map(|keyword| match keyword {
            "bottom" | "block-end" => CaptionSide::Bottom,
            _ => CaptionSide::Top,
        })
    );

    apply!(
        map,
        style,
        longhand(LonghandId::BorderCollapse),
        BorderCollapse,
        inherited.border_collapse,
        |value| as_string(value).map(|keyword| match keyword {
            "collapse" => BorderCollapse::Collapse,
            _ => BorderCollapse::Separate,
        })
    );

    resolve_border_spacing(map, style, font_size);
}

/// `text-decoration-line`, read from the shorthand as well: the shorthand keeps its own entry
/// and is never expanded into the longhand, so a page that writes `text-decoration: none` is
/// only visible there.
fn resolve_text_decoration(map: &CssProperties, style: &mut ComputedStyle) {
    if let Some(property) = map.get_id(shorthand(ShorthandId::TextDecoration)) {
        if matches!(property.actual, CssValue::None) {
            style.inherited.text_decoration_line = TextDecorationLine::NONE;
            style.declared.set(Prop::TextDecorationLine);
            return;
        }
        if let Some(keyword) = as_string(&property.actual) {
            let line = if keyword == "none" || keyword == "initial" || keyword == "unset" {
                Some(TextDecorationLine::NONE)
            } else if keyword.contains("underline") {
                Some(TextDecorationLine {
                    underline: true,
                    line_through: false,
                })
            } else if keyword.contains("line-through") {
                Some(TextDecorationLine {
                    underline: false,
                    line_through: true,
                })
            } else {
                None
            };
            if let Some(line) = line {
                style.inherited.text_decoration_line = line;
                style.declared.set(Prop::TextDecorationLine);
                return;
            }
        }
    }

    apply!(
        map,
        style,
        longhand(LonghandId::TextDecorationLine),
        TextDecorationLine,
        inherited.text_decoration_line,
        |value| as_string(value).map(|keyword| TextDecorationLine {
            underline: keyword.contains("underline"),
            line_through: keyword.contains("line-through"),
        })
    );
}

/// `border-spacing` is one declaration feeding two axes: one length applies to both, two are
/// horizontal then vertical (CSS 2 §17.6.1).
fn resolve_border_spacing(map: &CssProperties, style: &mut ComputedStyle, font_size: f32) {
    let Some(declared) = value(map, longhand(LonghandId::BorderSpacing)) else {
        return;
    };
    let (x, y) = if let Some(list) = as_list(declared) {
        let lengths: Vec<f32> = list.iter().filter_map(|item| length_px(item, font_size)).collect();
        match lengths.as_slice() {
            [x, y, ..] => (Some(*x), Some(*y)),
            [both] => (Some(*both), None),
            [] => (None, None),
        }
    } else {
        (length_px(declared, font_size), None)
    };
    if let Some(x) = x {
        style.inherited.border_spacing_x = x;
        style.declared.set(Prop::BorderSpacingX);
    }
    // A single length applies to both axes; with two, the second is the vertical one.
    if let Some(y) = y.or(x) {
        style.inherited.border_spacing_y = y;
        style.declared.set(Prop::BorderSpacingY);
    }
}

fn resolve_box(map: &CssProperties, style: &mut ComputedStyle) {
    apply!(
        map,
        style,
        longhand(LonghandId::Display),
        Display,
        box_group.display,
        |value| as_string(value).map(display_of)
    );

    apply!(
        map,
        style,
        longhand(LonghandId::Position),
        Position,
        box_group.position,
        |value| as_string(value).map(|keyword| match keyword {
            "relative" => Position::Relative,
            "absolute" => Position::Absolute,
            "fixed" => Position::Fixed,
            "sticky" => Position::Sticky,
            _ => Position::Static,
        })
    );

    apply!(
        map,
        style,
        longhand(LonghandId::Float),
        Float,
        box_group.float,
        |value| {
            as_string(value).map(|keyword| match keyword {
                "left" => Float::Left,
                "right" => Float::Right,
                _ => Float::None,
            })
        }
    );

    apply!(
        map,
        style,
        longhand(LonghandId::Clear),
        Clear,
        box_group.clear,
        |value| {
            as_string(value).map(|keyword| match keyword {
                "left" => Clear::Left,
                "right" => Clear::Right,
                "both" => Clear::Both,
                _ => Clear::None,
            })
        }
    );

    apply!(
        map,
        style,
        longhand(LonghandId::BoxSizing),
        BoxSizing,
        box_group.box_sizing,
        |value| as_string(value).map(|keyword| match keyword {
            "border-box" => BoxSizing::BorderBox,
            _ => BoxSizing::ContentBox,
        })
    );

    apply!(
        map,
        style,
        longhand(LonghandId::OverflowX),
        OverflowX,
        box_group.overflow_x,
        |value| as_string(value).map(overflow_of)
    );
    apply!(
        map,
        style,
        longhand(LonghandId::OverflowY),
        OverflowY,
        box_group.overflow_y,
        |value| as_string(value).map(overflow_of)
    );

    apply!(
        map,
        style,
        longhand(LonghandId::ZIndex),
        ZIndex,
        box_group.z_index,
        |value| {
            if let Some(number) = as_number(value) {
                return Some(ZIndex::Index(number));
            }
            as_string(value).map(|_| ZIndex::Auto)
        }
    );

    apply!(
        map,
        style,
        longhand(LonghandId::Opacity),
        Opacity,
        box_group.opacity,
        as_number
    );

    apply!(
        map,
        style,
        longhand(LonghandId::MixBlendMode),
        MixBlendMode,
        box_group.mix_blend_mode,
        |value| as_string(value).map(Arc::from)
    );

    apply!(
        map,
        style,
        longhand(LonghandId::Resize),
        Resize,
        box_group.resize,
        |value| { as_string(value).map(Arc::from) }
    );

    apply!(
        map,
        style,
        longhand(LonghandId::ScrollbarWidth),
        ScrollbarWidth,
        box_group.scrollbar_width,
        |value| as_number(value).map(Some)
    );

    apply!(
        map,
        style,
        longhand(LonghandId::AspectRatio),
        AspectRatio,
        box_group.aspect_ratio,
        |value| as_number(value).map(Some)
    );

    apply!(
        map,
        style,
        shorthand(ShorthandId::TextWrap),
        TextWrap,
        box_group.text_wrap,
        |value| as_string(value).map(|keyword| match keyword {
            "nowrap" => TextWrap::NoWrap,
            "balance" => TextWrap::Balance,
            "pretty" => TextWrap::Pretty,
            "stable" => TextWrap::Stable,
            _ => TextWrap::Wrap,
        })
    );

    apply!(
        map,
        style,
        longhand(LonghandId::TableLayout),
        TableLayout,
        box_group.table_layout,
        |value| as_string(value).map(|keyword| match keyword {
            "fixed" => TableLayout::Fixed,
            _ => TableLayout::Auto,
        })
    );

    apply!(
        map,
        style,
        longhand(LonghandId::VerticalAlign),
        VerticalAlign,
        box_group.vertical_align,
        |value| as_string(value).map(|keyword| match keyword {
            "baseline" => VerticalAlign::Baseline,
            "sub" => VerticalAlign::Sub,
            "super" => VerticalAlign::Super,
            "text-top" => VerticalAlign::TextTop,
            "text-bottom" => VerticalAlign::TextBottom,
            "middle" => VerticalAlign::Middle,
            "top" => VerticalAlign::Top,
            "bottom" => VerticalAlign::Bottom,
            // A length, and the `inherit` the user-agent sheet puts on cells: nothing this
            // engine acts on, and the cell-alignment walk keeps climbing past it.
            _ => VerticalAlign::Other,
        })
    );
}

fn overflow_of(keyword: &str) -> Overflow {
    match keyword {
        "hidden" => Overflow::Hidden,
        "clip" => Overflow::Clip,
        "scroll" => Overflow::Scroll,
        "auto" => Overflow::Auto,
        _ => Overflow::Visible,
    }
}

fn resolve_sizes(map: &CssProperties, style: &mut ComputedStyle, font_size: f32) {
    let lpa = |value: &CssValue| Some(length_percentage_auto(value, font_size));
    let lp = |value: &CssValue| length_percentage(value, font_size);

    apply!(map, style, longhand(LonghandId::Width), Width, size.width, lpa);
    apply!(map, style, longhand(LonghandId::Height), Height, size.height, lpa);
    apply!(
        map,
        style,
        longhand(LonghandId::MinWidth),
        MinWidth,
        size.min_width,
        lpa
    );
    apply!(
        map,
        style,
        longhand(LonghandId::MinHeight),
        MinHeight,
        size.min_height,
        lpa
    );
    apply!(
        map,
        style,
        longhand(LonghandId::MaxWidth),
        MaxWidth,
        size.max_width,
        lpa
    );
    apply!(
        map,
        style,
        longhand(LonghandId::MaxHeight),
        MaxHeight,
        size.max_height,
        lpa
    );

    apply!(map, style, longhand(LonghandId::MarginTop), MarginTop, margin.top, lpa);
    apply!(
        map,
        style,
        longhand(LonghandId::MarginRight),
        MarginRight,
        margin.right,
        lpa
    );
    apply!(
        map,
        style,
        longhand(LonghandId::MarginBottom),
        MarginBottom,
        margin.bottom,
        lpa
    );
    apply!(
        map,
        style,
        longhand(LonghandId::MarginLeft),
        MarginLeft,
        margin.left,
        lpa
    );

    apply!(
        map,
        style,
        longhand(LonghandId::PaddingTop),
        PaddingTop,
        padding.top,
        lp
    );
    apply!(
        map,
        style,
        longhand(LonghandId::PaddingRight),
        PaddingRight,
        padding.right,
        lp
    );
    apply!(
        map,
        style,
        longhand(LonghandId::PaddingBottom),
        PaddingBottom,
        padding.bottom,
        lp
    );
    apply!(
        map,
        style,
        longhand(LonghandId::PaddingLeft),
        PaddingLeft,
        padding.left,
        lp
    );
}

fn resolve_borders(map: &CssProperties, style: &mut ComputedStyle, font_size: f32) {
    let bstyle = |value: &CssValue| as_string(value).map(border_style_of);
    apply!(
        map,
        style,
        longhand(LonghandId::BorderTopStyle),
        BorderTopStyle,
        border.top_style,
        bstyle
    );
    apply!(
        map,
        style,
        longhand(LonghandId::BorderRightStyle),
        BorderRightStyle,
        border.right_style,
        bstyle
    );
    apply!(
        map,
        style,
        longhand(LonghandId::BorderBottomStyle),
        BorderBottomStyle,
        border.bottom_style,
        bstyle
    );
    apply!(
        map,
        style,
        longhand(LonghandId::BorderLeftStyle),
        BorderLeftStyle,
        border.left_style,
        bstyle
    );

    // The declared width, before the style has its say. `medium` is the initial value, and it
    // is what an element with a style but no width of its own gets.
    let width_of = |id: LonghandId| {
        value(map, longhand(id))
            .and_then(|value| length_px(value, font_size))
            .unwrap_or(MEDIUM_BORDER_WIDTH)
    };
    let sides = [
        (LonghandId::BorderTopWidth, Prop::BorderTopWidth),
        (LonghandId::BorderRightWidth, Prop::BorderRightWidth),
        (LonghandId::BorderBottomWidth, Prop::BorderBottomWidth),
        (LonghandId::BorderLeftWidth, Prop::BorderLeftWidth),
    ];
    for (id, prop) in sides {
        if value(map, longhand(id)).is_some() {
            style.declared.set(prop);
        }
    }
    // A border whose style is `none` or `hidden` is zero wide whatever was declared
    // (css-backgrounds-3 §4.3), so layout and paint cannot disagree about the box.
    let width_or_zero = |visible: bool, id: LonghandId| if visible { width_of(id) } else { 0.0 };
    style.border.top_width = width_or_zero(style.border.top_style.is_visible(), LonghandId::BorderTopWidth);
    style.border.right_width = width_or_zero(style.border.right_style.is_visible(), LonghandId::BorderRightWidth);
    style.border.bottom_width = width_or_zero(style.border.bottom_style.is_visible(), LonghandId::BorderBottomWidth);
    style.border.left_width = width_or_zero(style.border.left_style.is_visible(), LonghandId::BorderLeftWidth);

    // `currentColor` is the initial value of every border colour, so an undeclared one renders
    // in the element's own text colour: `td { border: solid; color: blue }` draws blue borders.
    let current = style.inherited.color;
    style.border.top_color = current;
    style.border.right_color = current;
    style.border.bottom_color = current;
    style.border.left_color = current;
    let border_color = move |value: &CssValue| {
        if is_current_color(value) {
            return Some(current);
        }
        color(value)
    };
    apply!(
        map,
        style,
        longhand(LonghandId::BorderTopColor),
        BorderTopColor,
        border.top_color,
        border_color
    );
    apply!(
        map,
        style,
        longhand(LonghandId::BorderRightColor),
        BorderRightColor,
        border.right_color,
        border_color
    );
    apply!(
        map,
        style,
        longhand(LonghandId::BorderBottomColor),
        BorderBottomColor,
        border.bottom_color,
        border_color
    );
    apply!(
        map,
        style,
        longhand(LonghandId::BorderLeftColor),
        BorderLeftColor,
        border.left_color,
        border_color
    );

    let lp = |value: &CssValue| length_percentage(value, font_size);
    apply!(
        map,
        style,
        longhand(LonghandId::BorderTopLeftRadius),
        BorderTopLeftRadius,
        border.top_left_radius,
        lp
    );
    apply!(
        map,
        style,
        longhand(LonghandId::BorderTopRightRadius),
        BorderTopRightRadius,
        border.top_right_radius,
        lp
    );
    apply!(
        map,
        style,
        longhand(LonghandId::BorderBottomLeftRadius),
        BorderBottomLeftRadius,
        border.bottom_left_radius,
        lp
    );
    apply!(
        map,
        style,
        longhand(LonghandId::BorderBottomRightRadius),
        BorderBottomRightRadius,
        border.bottom_right_radius,
        lp
    );
}

fn resolve_outline(map: &CssProperties, style: &mut ComputedStyle, font_size: f32) {
    apply!(
        map,
        style,
        longhand(LonghandId::OutlineStyle),
        OutlineStyle,
        outline.style,
        |value| as_string(value).map(|keyword| {
            // `auto`, the user-agent focus ring, paints as a solid line.
            if keyword.eq_ignore_ascii_case("auto") {
                BorderStyle::Solid
            } else {
                border_style_of(keyword)
            }
        })
    );

    let declared_width = value(map, longhand(LonghandId::OutlineWidth));
    if declared_width.is_some() {
        style.declared.set(Prop::OutlineWidth);
    }
    let width = declared_width
        .and_then(|value| length_px(value, font_size))
        .unwrap_or(MEDIUM_BORDER_WIDTH);
    style.outline.width = if style.outline.style.is_visible() { width } else { 0.0 };

    // Unlike the border colours, an undeclared `outline-color` has always been plain black
    // here. Only the keywords follow the text colour: `currentColor`, and the `auto` that is
    // the property's real initial value.
    let current = style.inherited.color;
    apply!(
        map,
        style,
        longhand(LonghandId::OutlineColor),
        OutlineColor,
        outline.color,
        move |value: &CssValue| {
            if is_current_color(value) || as_string(value).is_some_and(|k| k.eq_ignore_ascii_case("auto")) {
                return Some(current);
            }
            color(value)
        }
    );

    apply!(
        map,
        style,
        longhand(LonghandId::OutlineOffset),
        OutlineOffset,
        outline.offset,
        |value| length_px(value, font_size)
    );
}

fn resolve_background(map: &CssProperties, style: &mut ComputedStyle) {
    let current = style.inherited.color;
    apply!(
        map,
        style,
        longhand(LonghandId::BackgroundColor),
        BackgroundColor,
        background.color,
        move |value: &CssValue| {
            if is_current_color(value) {
                return Some(current);
            }
            color(value)
        }
    );
    // The `background` shorthand keeps its own entry and is never expanded into the longhands,
    // so a page that writes `background: #fff url(x)` is only visible there.
    if !style.has(Prop::BackgroundColor) {
        apply!(
            map,
            style,
            shorthand(ShorthandId::Background),
            BackgroundColor,
            background.color,
            shorthand_background_color
        );
    }

    for id in [
        longhand(LonghandId::BackgroundImage),
        shorthand(ShorthandId::Background),
    ] {
        if let Some(url) = value(map, id).and_then(first_url) {
            style.background.image = Some(Arc::from(url));
            style.declared.set(Prop::BackgroundImage);
            break;
        }
    }
}

/// The insets, read from the logical name first and then the physical one.
///
/// Pages write `top`/`right`/`bottom`/`left`; the pipeline models the insets logically. The
/// aliasing is exact in the horizontal-tb, ltr writing mode this engine assumes.
fn resolve_insets(map: &CssProperties, style: &mut ComputedStyle, font_size: f32) {
    let sides = [
        (
            LonghandId::InsetBlockStart,
            LonghandId::Top,
            Prop::InsetBlockStart,
            0usize,
        ),
        (LonghandId::InsetBlockEnd, LonghandId::Bottom, Prop::InsetBlockEnd, 1),
        (
            LonghandId::InsetInlineStart,
            LonghandId::Left,
            Prop::InsetInlineStart,
            2,
        ),
        (LonghandId::InsetInlineEnd, LonghandId::Right, Prop::InsetInlineEnd, 3),
    ];
    for (logical, physical, prop, slot) in sides {
        let Some(declared) = value(map, longhand(logical)).or_else(|| value(map, longhand(physical))) else {
            continue;
        };
        let resolved = length_percentage_auto(declared, font_size);
        match slot {
            0 => style.inset.block_start = resolved,
            1 => style.inset.block_end = resolved,
            2 => style.inset.inline_start = resolved,
            _ => style.inset.inline_end = resolved,
        }
        style.declared.set(prop);
    }
}

fn resolve_flex(map: &CssProperties, style: &mut ComputedStyle, font_size: f32) {
    apply!(
        map,
        style,
        longhand(LonghandId::FlexBasis),
        FlexBasis,
        flex.basis,
        |value| Some(length_percentage_auto(value, font_size))
    );

    apply!(
        map,
        style,
        longhand(LonghandId::FlexDirection),
        FlexDirection,
        flex.direction,
        |value| as_string(value).map(|keyword| match keyword {
            "row-reverse" => FlexDirection::RowReverse,
            "column" => FlexDirection::Column,
            "column-reverse" => FlexDirection::ColumnReverse,
            _ => FlexDirection::Row,
        })
    );

    apply!(
        map,
        style,
        longhand(LonghandId::FlexGrow),
        FlexGrow,
        flex.grow,
        as_number
    );
    apply!(
        map,
        style,
        longhand(LonghandId::FlexShrink),
        FlexShrink,
        flex.shrink,
        as_number
    );

    apply!(
        map,
        style,
        longhand(LonghandId::FlexWrap),
        FlexWrap,
        flex.wrap,
        |value| as_string(value).map(|keyword| match keyword {
            "wrap" => FlexWrap::Wrap,
            "wrap-reverse" => FlexWrap::WrapReverse,
            _ => FlexWrap::NoWrap,
        })
    );

    apply!(map, style, shorthand(ShorthandId::Gap), Gap, flex.gap, |value| {
        length_percentage(value, font_size)
    });

    let align = |value: &CssValue| as_string(value).map(align_of);
    apply!(
        map,
        style,
        longhand(LonghandId::AlignItems),
        AlignItems,
        flex.align_items,
        align
    );
    apply!(
        map,
        style,
        longhand(LonghandId::AlignSelf),
        AlignSelf,
        flex.align_self,
        align
    );
    apply!(
        map,
        style,
        longhand(LonghandId::AlignContent),
        AlignContent,
        flex.align_content,
        align
    );
    apply!(
        map,
        style,
        longhand(LonghandId::JustifyItems),
        JustifyItems,
        flex.justify_items,
        align
    );
    apply!(
        map,
        style,
        longhand(LonghandId::JustifySelf),
        JustifySelf,
        flex.justify_self,
        align
    );
    apply!(
        map,
        style,
        longhand(LonghandId::JustifyContent),
        JustifyContent,
        flex.justify_content,
        align
    );
}

fn resolve_grid(map: &CssProperties, style: &mut ComputedStyle) {
    let track_list = |value: &CssValue| grid_track_list(value).map(Arc::from);
    apply!(
        map,
        style,
        longhand(LonghandId::GridTemplateRows),
        GridTemplateRows,
        grid.template_rows,
        track_list
    );
    apply!(
        map,
        style,
        longhand(LonghandId::GridTemplateColumns),
        GridTemplateColumns,
        grid.template_columns,
        track_list
    );
    apply!(
        map,
        style,
        longhand(LonghandId::GridAutoRows),
        GridAutoRows,
        grid.auto_rows,
        track_list
    );
    apply!(
        map,
        style,
        longhand(LonghandId::GridAutoColumns),
        GridAutoColumns,
        grid.auto_columns,
        track_list
    );

    let placement = |value: &CssValue| grid_placement(value).map(Arc::from);
    apply!(
        map,
        style,
        shorthand(ShorthandId::GridRow),
        GridRow,
        grid.row,
        placement
    );
    apply!(
        map,
        style,
        shorthand(ShorthandId::GridColumn),
        GridColumn,
        grid.column,
        placement
    );
    apply!(
        map,
        style,
        shorthand(ShorthandId::GridArea),
        GridArea,
        grid.area,
        placement
    );

    apply!(
        map,
        style,
        longhand(LonghandId::GridTemplateAreas),
        GridTemplateAreas,
        grid.template_areas,
        |value| grid_areas(value).map(Arc::from)
    );

    apply!(
        map,
        style,
        longhand(LonghandId::GridAutoFlow),
        GridAutoFlow,
        grid.auto_flow,
        |value| as_string(value).map(|keyword| match keyword {
            "column" => GridAutoFlow::Column,
            "row dense" => GridAutoFlow::RowDense,
            "column dense" => GridAutoFlow::ColumnDense,
            _ => GridAutoFlow::Row,
        })
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matcher::styling::{no_location, CssProperty, DeclarationProperty};
    use crate::stylesheet::Specificity;
    use gosub_interface::css3::CssOrigin;

    /// A property map holding exactly these declarations, computed the way the cascade computes
    /// one. `font_size_basis` is what an `em` in them resolves against, as the cascade sets it.
    fn map_with(declarations: &[(&str, CssValue)], font_size_basis: f32) -> CssProperties {
        let mut map = CssProperties::new();
        for (name, value) in declarations {
            let id = PropertyId::from_name(name).expect("a property the tests name");
            let mut property = CssProperty::new(id);
            property.declared.push(DeclarationProperty {
                value: value.clone(),
                origin: CssOrigin::Author,
                important: false,
                location: no_location(),
                specificity: Specificity::new(0, 0, 1),
                shadow_depth: 0,
                order: 0,
                layer: None,
                attached: false,
            });
            map.insert_id(id, property);
        }
        map.font_size_px = font_size_basis;
        for (_, property) in map.iter_ids_mut() {
            property.font_size_basis = font_size_basis;
            property.root_font_size_basis = font_size_basis;
            property.mark_dirty();
            property.compute_value();
        }
        map
    }

    fn map(declarations: &[(&str, CssValue)]) -> CssProperties {
        map_with(declarations, 16.0)
    }

    fn px(value: f64) -> CssValue {
        CssValue::Unit(value, "px".to_string())
    }

    fn keyword(value: &str) -> CssValue {
        CssValue::String(value.to_string())
    }

    /// Nothing declared is the parent's inherited group and the initial value of everything
    /// else - and no property reports itself as declared.
    #[test]
    fn an_empty_map_inherits_and_nothing_else() {
        let parent = computed_style(&map(&[("color", keyword("red")), ("width", px(100.0))]), None);
        let child = computed_style(&CssProperties::new(), Some(&parent));

        assert_eq!(child.inherited.color, Color::rgba(255, 0, 0, 255));
        assert_eq!(child.size.width, LengthPercentageAuto::Auto);
        assert!(!child.has(Prop::Color));
        assert!(!child.has(Prop::Width));
    }

    /// `transparent` is a colour with zero alpha, not "no colour". The CSS-triangle idiom
    /// (`border-color: transparent transparent green`) depends on it: an unresolved
    /// `transparent` paints a solid black box.
    #[test]
    fn transparent_is_a_colour() {
        let style = computed_style(&map(&[("background-color", keyword("transparent"))]), None);
        assert_eq!(style.background.color, Color::TRANSPARENT);
        assert!(style.has(Prop::BackgroundColor));
    }

    /// The system colours are not in the named-colour table, so without their own answer they
    /// would fall through to opaque black.
    #[test]
    fn system_colours_resolve() {
        let style = computed_style(&map(&[("background-color", keyword("buttonface"))]), None);
        assert_eq!(style.background.color, Color::rgba(240, 240, 240, 255));

        let style = computed_style(&map(&[("color", keyword("-webkit-focus-ring-color"))]), None);
        assert_eq!(style.inherited.color, Color::rgba(0x23, 0x82, 0xeb, 255));
    }

    /// `currentColor` is the element's own text colour, and it is what an undeclared border
    /// colour is: `td { border: solid; color: blue }` draws blue borders.
    #[test]
    fn current_color_follows_the_text_colour() {
        let blue = Color::rgba(0, 0, 255, 255);

        let style = computed_style(
            &map(&[("color", keyword("blue")), ("border-top-style", keyword("solid"))]),
            None,
        );
        assert_eq!(style.border.top_color, blue);
        assert!(!style.has(Prop::BorderTopColor));

        let style = computed_style(
            &map(&[
                ("color", keyword("blue")),
                ("border-left-color", keyword("currentcolor")),
                ("background-color", keyword("currentcolor")),
                ("outline-color", keyword("auto")),
            ]),
            None,
        );
        assert_eq!(style.border.left_color, blue);
        assert_eq!(style.background.color, blue);
        assert_eq!(style.outline.color, blue);
    }

    /// An undeclared border colour follows the *inherited* colour when the element declares no
    /// colour of its own.
    #[test]
    fn border_colour_follows_the_inherited_colour() {
        let parent = computed_style(&map(&[("color", keyword("red"))]), None);
        let child = computed_style(&map(&[("border-top-style", keyword("solid"))]), Some(&parent));
        assert_eq!(child.border.top_color, Color::rgba(255, 0, 0, 255));
    }

    /// A border whose style is `none` or `hidden` is zero wide whatever the declaration said,
    /// and one with a style but no width is `medium`.
    #[test]
    fn border_style_decides_whether_a_width_survives() {
        let style = computed_style(
            &map(&[("border-top-style", keyword("none")), ("border-top-width", px(10.0))]),
            None,
        );
        assert_eq!(style.border.top_width, 0.0);

        let style = computed_style(
            &map(&[("border-top-style", keyword("hidden")), ("border-top-width", px(10.0))]),
            None,
        );
        assert_eq!(style.border.top_width, 0.0);

        let style = computed_style(&map(&[("border-top-style", keyword("solid"))]), None);
        assert_eq!(style.border.top_width, MEDIUM_BORDER_WIDTH);

        let style = computed_style(
            &map(&[("border-top-style", keyword("solid")), ("border-top-width", px(4.0))]),
            None,
        );
        assert_eq!(style.border.top_width, 4.0);
    }

    /// The same rule governs the outline, and `outline-style: auto` - the user-agent focus ring
    /// - draws as a solid line rather than as nothing.
    #[test]
    fn outline_style_auto_is_a_visible_ring() {
        let style = computed_style(&map(&[("outline-style", keyword("auto"))]), None);
        assert_eq!(style.outline.style, BorderStyle::Solid);
        assert_eq!(style.outline.width, MEDIUM_BORDER_WIDTH);

        let style = computed_style(&map(&[("outline-width", px(2.0))]), None);
        assert_eq!(style.outline.width, 0.0, "no outline style means no ring");
    }

    /// The `text-decoration` shorthand keeps its own entry and is never expanded, so a page
    /// that writes `text-decoration: none` is only visible there.
    #[test]
    fn text_decoration_is_read_from_the_shorthand_too() {
        let style = computed_style(&map(&[("text-decoration", keyword("none"))]), None);
        assert_eq!(style.inherited.text_decoration_line, TextDecorationLine::NONE);
        assert!(style.has(Prop::TextDecorationLine));

        let style = computed_style(&map(&[("text-decoration", keyword("underline"))]), None);
        assert!(style.inherited.text_decoration_line.underline);

        let style = computed_style(&map(&[("text-decoration-line", keyword("line-through"))]), None);
        assert!(style.inherited.text_decoration_line.line_through);
    }

    /// The `background` shorthand is not expanded either, so its colour and its `url()` are
    /// read out of it.
    #[test]
    fn the_background_shorthand_gives_up_its_colour_and_image() {
        let shorthand = CssValue::List(vec![
            CssValue::String("#ffffff".to_string()),
            CssValue::Function(
                "url".to_string(),
                vec![CssValue::String("\"grayarrow.gif\"".to_string())],
            ),
            CssValue::String("no-repeat".to_string()),
        ]);
        let style = computed_style(&map(&[("background", shorthand)]), None);
        assert_eq!(style.background.color, Color::rgba(255, 255, 255, 255));
        assert_eq!(style.background.image.as_deref(), Some("grayarrow.gif"));
    }

    /// Pages write the physical `top`/`left`; the pipeline models the insets logically.
    #[test]
    fn physical_insets_alias_onto_the_logical_ones() {
        let style = computed_style(&map(&[("top", px(5.0)), ("left", CssValue::Percentage(50.0))]), None);
        assert_eq!(style.inset.block_start, LengthPercentageAuto::Px(5.0));
        assert_eq!(style.inset.inline_start, LengthPercentageAuto::Percent(50.0));
        assert!(style.has(Prop::InsetBlockStart));
        assert!(style.has(Prop::InsetInlineStart));
        assert_eq!(style.inset.block_end, LengthPercentageAuto::Auto);
    }

    /// A percentage `line-height` is a fraction of the element's own font-size; a unitless
    /// number stays a multiplier, so it inherits as one.
    #[test]
    fn line_height_forms() {
        let style = computed_style(
            &map_with(
                &[("font-size", px(20.0)), ("line-height", CssValue::Percentage(150.0))],
                16.0,
            ),
            None,
        );
        assert_eq!(style.inherited.line_height, LineHeight::Px(30.0));

        let style = computed_style(
            &map(&[(
                "line-height",
                CssValue::Number(1.5, crate::tokenizer::NumberKind::Number),
            )]),
            None,
        );
        assert_eq!(style.inherited.line_height, LineHeight::Number(1.5));

        let style = computed_style(&map(&[("line-height", keyword("normal"))]), None);
        assert_eq!(style.inherited.line_height, LineHeight::Normal);
    }

    /// `ch`, `ex`, `lh` and `ic` have no value without font metrics, so the computed stage
    /// leaves them alone and these approximations settle them.
    #[test]
    fn font_relative_units_without_metrics() {
        let style = computed_style(
            &map_with(
                &[
                    ("font-size", px(20.0)),
                    ("max-width", CssValue::Unit(17.0, "ch".to_string())),
                    ("min-width", CssValue::Unit(2.0, "ex".to_string())),
                ],
                16.0,
            ),
            None,
        );
        assert_eq!(style.size.max_width, LengthPercentageAuto::Px(17.0 * 0.55 * 20.0));
        assert_eq!(style.size.min_width, LengthPercentageAuto::Px(2.0 * 0.5 * 20.0));
    }

    /// The absolute `font-size` keywords are the CSS scale with `medium` at 16px; the relative
    /// ones step by the spec's suggested 1.2 factor, against the parent's size.
    #[test]
    fn font_size_keywords() {
        let style = computed_style(&map(&[("font-size", keyword("x-large"))]), None);
        assert_eq!(style.inherited.font_size, 24.0);

        let parent = computed_style(&map(&[("font-size", px(20.0))]), None);
        let child = computed_style(&map(&[("font-size", keyword("larger"))]), Some(&parent));
        assert_eq!(child.inherited.font_size, 24.0);

        let child = computed_style(&map(&[("font-size", keyword("smaller"))]), Some(&parent));
        assert!((child.inherited.font_size - 20.0 / 1.2).abs() < 0.001);
    }

    /// A bare generic `monospace` family defaults to 13px, but only where nothing anywhere up
    /// the chain said how big the text should be.
    #[test]
    fn the_monospace_default_size_quirk() {
        let style = computed_style(&map(&[("font-family", keyword("monospace"))]), None);
        assert_eq!(style.inherited.font_size, MONOSPACE_DEFAULT_FONT_SIZE);

        // A descendant of a page that set a size keeps that size, quirk or not.
        let parent = computed_style(&map(&[("font-size", px(20.0))]), None);
        let child = computed_style(&map(&[("font-family", keyword("monospace"))]), Some(&parent));
        assert_eq!(child.inherited.font_size, 20.0);

        // The family has to be the bare generic: `monospace, serif` is not it, but a second
        // family after it is.
        let style = computed_style(&map(&[("font-family", keyword("Consolas"))]), None);
        assert_eq!(style.inherited.font_size, 16.0);
    }

    /// A grid track list comes back as the CSS text the layouter's own parser takes.
    #[test]
    fn grid_track_lists_round_trip_to_text() {
        let repeat = CssValue::Function(
            "repeat".to_string(),
            vec![
                CssValue::Number(3.0, crate::tokenizer::NumberKind::Integer),
                CssValue::Comma,
                CssValue::Unit(1.0, "fr".to_string()),
            ],
        );
        let style = computed_style(&map(&[("grid-template-columns", repeat)]), None);
        assert_eq!(&*style.grid.template_columns, "repeat(3, 1fr)");

        let rows = CssValue::List(vec![
            CssValue::String("'a a'".to_string()),
            CssValue::String("'b c'".to_string()),
        ]);
        let style = computed_style(&map(&[("grid-template-areas", rows)]), None);
        assert_eq!(&*style.grid.template_areas, "a a\nb c");
    }

    /// An `em` is already pixels by the time a value gets here - the computed stage does it -
    /// so the conversion must not scale it a second time.
    #[test]
    fn em_arrives_already_resolved() {
        let style = computed_style(
            &map_with(&[("margin-top", CssValue::Unit(2.0, "em".to_string()))], 20.0),
            None,
        );
        assert_eq!(style.margin.top, LengthPercentageAuto::Px(40.0));
    }

    /// A percentage is not resolved here: it needs a containing block, which style resolution
    /// does not have.
    #[test]
    fn percentages_travel_on_to_layout() {
        let style = computed_style(&map(&[("width", CssValue::Percentage(50.0))]), None);
        assert_eq!(style.size.width, LengthPercentageAuto::Percent(50.0));
        assert_eq!(style.size.width.resolve(400.0), Some(200.0));
        assert_eq!(style.size.width.to_px(), None);
    }
}
