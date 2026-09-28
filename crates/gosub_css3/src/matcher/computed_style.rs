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

use std::sync::{Arc, LazyLock};

use gosub_interface::style::{
    AlignValue, BorderCollapse, BorderStyle, BoxSizing, CaptionSide, Clear, Color, ComputedStyle, Display,
    FlexDirection, FlexWrap, Float, FontStyle, FontWeight, GridAutoFlow, LengthPercentage, LengthPercentageAuto,
    LetterSpacing, LineHeight, Overflow, Position, Prop, TableLayout, TextAlign, TextDecorationLine, TextTransform,
    TextWrap, VerticalAlign, WhiteSpace, ZIndex,
};

use crate::colors::resolve::{resolve_color, ColorContext};
use crate::colors::{CssColor, RgbColor};
use crate::matcher::keywords::{
    BorderCollapseKeyword, BorderTopStyleKeyword, BoxSizingKeyword, CaptionSideKeyword, ClearKeyword,
    FlexDirectionKeyword, Keyword, OutlineStyleKeyword, OverflowXKeyword, TableLayoutKeyword, TextAlignKeyword,
    WhiteSpaceKeyword,
};
use crate::matcher::property_ids::{LonghandId, PropertyId, LONGHAND_COUNT, PROPERTY_COUNT};
use crate::matcher::styling::CssProperties;
use crate::stylesheet::CssValue;

const fn longhand(id: LonghandId) -> PropertyId {
    PropertyId::Longhand(id)
}

/// The value of one property on this element, or `None` when the map has no entry for it or the
/// entry never resolved to anything.
fn value(map: &CssProperties, id: PropertyId) -> Option<&CssValue> {
    match map.get_id(id) {
        Some(property) if !matches!(property.computed, CssValue::None) => Some(&property.computed),
        _ => None,
    }
}

/// The same, falling back to what the element inherits for `id`.
///
/// Only the properties that inherit are read this way, and only because this conversion has
/// always seen them: an element used to be given an entry for every inherited property any
/// ancestor had settled, so "what the cascade settled here" and "what this element inherits"
/// were one question. They are two now, and this keeps the answer the one it was.
///
/// What that answer is, for the relative `font-size` keywords, is wrong: `small { font-size:
/// smaller }` travels down as the keyword rather than as the length it computes to, and every
/// element below re-applies it. Fixing that is not this change - it moves pixels on real pages
/// - so it stays, and is reported.
fn value_or_inherited(map: &CssProperties, id: PropertyId) -> Option<&CssValue> {
    value(map, id).or_else(|| map.inherited_value(id))
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

/// A `<length-percentage>` in the element's own font-size context.
///
/// Every length unit is already pixels by the time a value gets here: the computed stage
/// converts them all, the font-relative ones against this element's font-size, which is where
/// css-values says they belong.
fn length_percentage(value: &CssValue, _font_size: f32) -> Option<LengthPercentage> {
    if as_unit(value).is_some() {
        return Some(LengthPercentage::Px(value.unit_to_px()));
    }
    if let Some(pct) = as_percentage(value) {
        return Some(LengthPercentage::Percent(pct));
    }
    if let Some(("calc", terms)) = as_function(value) {
        return linear_calc(terms);
    }
    // A bare number in a length slot is read as pixels, which is what `top: 0` and `margin: 0`
    // rely on.
    as_number(value).map(LengthPercentage::Px)
}

/// A `calc()` the computed stage has simplified to a sum of a percentage and px - the shape every
/// linear expression over lengths and a percentage comes down to (`calc(100% - 2rem)` arrives as
/// `calc(100% - 32px)`).
///
/// Anything else is `None`: a `min()`, `max()` or `clamp()` over a percentage cannot be a sum,
/// and needs the basis before it has a value at all. The property is left unset, as it always
/// was, rather than guessed.
fn linear_calc(terms: &[CssValue]) -> Option<LengthPercentage> {
    let (mut px, mut percent) = (0.0_f32, 0.0_f32);
    let mut sign = 1.0_f32;
    for term in terms {
        match term {
            CssValue::String(op) if op == "+" => sign = 1.0,
            CssValue::String(op) if op == "-" => sign = -1.0,
            CssValue::Percentage(value) => percent += sign * *value as f32,
            CssValue::Unit(_, unit) if unit == "px" => px += sign * term.unit_to_px(),
            CssValue::Zero => {}
            other => {
                log::debug!("calc() that is not a sum of a length and a percentage, left unset: {other:?}");
                return None;
            }
        }
    }
    Some(match (px, percent) {
        (px, 0.0) => LengthPercentage::Px(px),
        (0.0, percent) => LengthPercentage::Percent(percent),
        (px, percent) => LengthPercentage::Calc { px, percent },
    })
}

/// The same, plus `auto`. Any keyword that is not a length is `auto` here: none of the
/// consumers act on one, and `auto` is what each of them falls back to.
fn length_percentage_auto(value: &CssValue, font_size: f32) -> LengthPercentageAuto {
    match length_percentage(value, font_size) {
        Some(LengthPercentage::Px(px)) => LengthPercentageAuto::Px(px),
        Some(LengthPercentage::Percent(pct)) => LengthPercentageAuto::Percent(pct),
        Some(LengthPercentage::Calc { px, percent }) => LengthPercentageAuto::Calc { px, percent },
        None => LengthPercentageAuto::Auto,
    }
}

/// A plain px length, for the properties whose value space holds nothing else. A percentage is
/// not one of them, so it leaves the property unset rather than being read as pixels.
fn length_px(value: &CssValue, font_size: f32) -> Option<f32> {
    match length_percentage(value, font_size)? {
        LengthPercentage::Px(px) => Some(px),
        LengthPercentage::Percent(_) | LengthPercentage::Calc { .. } => None,
    }
}

// ── Colours ──────────────────────────────────────────────────────────────────

/// A resolved colour, as the typed style carries it.
#[expect(clippy::cast_possible_truncation, reason = "a colour channel is a byte")]
#[expect(clippy::cast_sign_loss, reason = "a colour channel is never negative")]
fn to_color(color: CssColor) -> Color {
    let rgb = color.to_rgb();
    Color::rgba(rgb.r as u8, rgb.g as u8, rgb.b as u8, rgb.a as u8)
}

/// The colour a value names, or `None` when it names none - in which case the property keeps
/// whatever it would have had. `currentcolor` does not name one here; the properties it may
/// stand in read their colour through [`color_or_current`].
fn color(value: &CssValue) -> Option<Color> {
    resolve_color(value, &ColorContext::default()).color().map(to_color)
}

/// The colour a value names on a property where `currentcolor` stands for `current`.
///
/// The computed value keeps `currentcolor` wherever it sits, so a colour function built on it -
/// `contrast-color(currentcolor)`, `color-mix(in srgb, currentcolor, red)` - arrives here still
/// a function. This is the used value, where the element's colour is known.
fn color_or_current(value: &CssValue, current: Color) -> Option<Color> {
    let current = CssColor::from(RgbColor::new(
        f32::from(current.r),
        f32::from(current.g),
        f32::from(current.b),
        f32::from(current.a),
    ));
    let context = ColorContext { current: Some(current) };
    resolve_color(value, &context).color().map(to_color)
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
    let text = grid_text(value);
    (!text.is_empty()).then_some(text)
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

/// A `<line-style>`. The four `border-*-style` longhands share that grammar, so the top side's
/// generated enum reads all four; a test holds the four keyword lists equal.
fn border_style_of(keyword: BorderTopStyleKeyword) -> BorderStyle {
    match keyword {
        BorderTopStyleKeyword::None => BorderStyle::None,
        BorderTopStyleKeyword::Hidden => BorderStyle::Hidden,
        BorderTopStyleKeyword::Solid => BorderStyle::Solid,
        BorderTopStyleKeyword::Dashed => BorderStyle::Dashed,
        BorderTopStyleKeyword::Dotted => BorderStyle::Dotted,
        BorderTopStyleKeyword::Double => BorderStyle::Double,
        BorderTopStyleKeyword::Groove => BorderStyle::Groove,
        BorderTopStyleKeyword::Ridge => BorderStyle::Ridge,
        BorderTopStyleKeyword::Inset => BorderStyle::Inset,
        BorderTopStyleKeyword::Outset => BorderStyle::Outset,
    }
}

/// The keyword a value spells, for a property whose grammar is a plain choice of keywords.
fn keyword<K: Keyword>(value: &CssValue) -> Option<K> {
    as_string(value).and_then(K::from_ident)
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

/// Read one longhand and, when it says something, write it to a field and record that the
/// element's own cascade had a value for it.
///
/// Which property that records comes from [`ENGINE_LONGHANDS`], and whether the value may be
/// the inherited one from the longhand's own definition: an inherited property reads what the
/// element settled or what it inherits, any other only what the element settled. Neither is
/// written at the call site, so neither can be written wrong there.
///
/// `$group` names the group's accessor rather than its field, because writing is what clones a
/// shared group: a group whose properties this element never declared is never reached for
/// writing and stays the one its parent or the initial style already holds.
macro_rules! read {
    ($map:expr, $style:expr, $id:ident, $group:ident, $field:ident, $read:expr) => {{
        let id = longhand(LonghandId::$id);
        let found = if id.inherited() {
            value_or_inherited($map, id)
        } else {
            value($map, id)
        };
        if let Some(read_value) = found.and_then($read) {
            $style.$group().$field = read_value;
            let prop = prop_of(LonghandId::$id);
            debug_assert!(prop.is_some(), "{} is read but not in ENGINE_LONGHANDS", id.name());
            if let Some(prop) = prop {
                $style.declared.set(prop);
            }
        }
    }};
}

// ── Which field group each property belongs to ───────────────────────────────

const G_INHERITED: u16 = 1 << 0;
const G_BOX: u16 = 1 << 1;
const G_SIZE: u16 = 1 << 2;
const G_MARGIN: u16 = 1 << 3;
const G_PADDING: u16 = 1 << 4;
const G_BORDER: u16 = 1 << 5;
const G_OUTLINE: u16 = 1 << 6;
const G_BACKGROUND: u16 = 1 << 7;
const G_INSET: u16 = 1 << 8;
const G_FLEX: u16 = 1 << 9;
const G_GRID: u16 = 1 << 10;

/// Every longhand the typed style reads: the [`Prop`] it answers for, and the field group it
/// lives in.
///
/// This is the one place the three are paired. The group table and [`prop_of`] are built from
/// it, and [`read!`] takes only the longhand. A longhand in the wrong group is never read - the
/// group's resolver does not run for an element that touched only other groups - so a test
/// declares each one at its initial value and checks its property is recorded.
///
/// A property can have more than one longhand (`inset-block-start` and `top`, the two halves of
/// `text-wrap`), and one longhand can feed two properties (`border-spacing`).
static ENGINE_LONGHANDS: &[(LonghandId, Prop, u16)] = &[
    // inherited
    (LonghandId::FontFamily, Prop::FontFamily, G_INHERITED),
    (LonghandId::FontSize, Prop::FontSize, G_INHERITED),
    (LonghandId::Color, Prop::Color, G_INHERITED),
    (LonghandId::FontStyle, Prop::FontStyle, G_INHERITED),
    (LonghandId::FontWeight, Prop::FontWeight, G_INHERITED),
    (LonghandId::LineHeight, Prop::LineHeight, G_INHERITED),
    (LonghandId::TextAlign, Prop::TextAlign, G_INHERITED),
    (LonghandId::TextTransform, Prop::TextTransform, G_INHERITED),
    (LonghandId::WhiteSpace, Prop::WhiteSpace, G_INHERITED),
    (LonghandId::LetterSpacing, Prop::LetterSpacing, G_INHERITED),
    (LonghandId::CaptionSide, Prop::CaptionSide, G_INHERITED),
    (LonghandId::BorderCollapse, Prop::BorderCollapse, G_INHERITED),
    (LonghandId::TextDecorationLine, Prop::TextDecorationLine, G_INHERITED),
    (LonghandId::BorderSpacing, Prop::BorderSpacingX, G_INHERITED),
    (LonghandId::BorderSpacing, Prop::BorderSpacingY, G_INHERITED),
    // box
    (LonghandId::Display, Prop::Display, G_BOX),
    (LonghandId::Position, Prop::Position, G_BOX),
    (LonghandId::Float, Prop::Float, G_BOX),
    (LonghandId::Clear, Prop::Clear, G_BOX),
    (LonghandId::BoxSizing, Prop::BoxSizing, G_BOX),
    (LonghandId::OverflowX, Prop::OverflowX, G_BOX),
    (LonghandId::OverflowY, Prop::OverflowY, G_BOX),
    (LonghandId::ZIndex, Prop::ZIndex, G_BOX),
    (LonghandId::Opacity, Prop::Opacity, G_BOX),
    (LonghandId::MixBlendMode, Prop::MixBlendMode, G_BOX),
    (LonghandId::Resize, Prop::Resize, G_BOX),
    (LonghandId::ScrollbarWidth, Prop::ScrollbarWidth, G_BOX),
    (LonghandId::AspectRatio, Prop::AspectRatio, G_BOX),
    (LonghandId::TextWrapMode, Prop::TextWrap, G_BOX),
    (LonghandId::TextWrapStyle, Prop::TextWrap, G_BOX),
    (LonghandId::TableLayout, Prop::TableLayout, G_BOX),
    (LonghandId::VerticalAlign, Prop::VerticalAlign, G_BOX),
    // size
    (LonghandId::Width, Prop::Width, G_SIZE),
    (LonghandId::Height, Prop::Height, G_SIZE),
    (LonghandId::MinWidth, Prop::MinWidth, G_SIZE),
    (LonghandId::MinHeight, Prop::MinHeight, G_SIZE),
    (LonghandId::MaxWidth, Prop::MaxWidth, G_SIZE),
    (LonghandId::MaxHeight, Prop::MaxHeight, G_SIZE),
    // margin
    (LonghandId::MarginTop, Prop::MarginTop, G_MARGIN),
    (LonghandId::MarginRight, Prop::MarginRight, G_MARGIN),
    (LonghandId::MarginBottom, Prop::MarginBottom, G_MARGIN),
    (LonghandId::MarginLeft, Prop::MarginLeft, G_MARGIN),
    // padding
    (LonghandId::PaddingTop, Prop::PaddingTop, G_PADDING),
    (LonghandId::PaddingRight, Prop::PaddingRight, G_PADDING),
    (LonghandId::PaddingBottom, Prop::PaddingBottom, G_PADDING),
    (LonghandId::PaddingLeft, Prop::PaddingLeft, G_PADDING),
    // border
    (LonghandId::BorderTopStyle, Prop::BorderTopStyle, G_BORDER),
    (LonghandId::BorderRightStyle, Prop::BorderRightStyle, G_BORDER),
    (LonghandId::BorderBottomStyle, Prop::BorderBottomStyle, G_BORDER),
    (LonghandId::BorderLeftStyle, Prop::BorderLeftStyle, G_BORDER),
    (LonghandId::BorderTopWidth, Prop::BorderTopWidth, G_BORDER),
    (LonghandId::BorderRightWidth, Prop::BorderRightWidth, G_BORDER),
    (LonghandId::BorderBottomWidth, Prop::BorderBottomWidth, G_BORDER),
    (LonghandId::BorderLeftWidth, Prop::BorderLeftWidth, G_BORDER),
    (LonghandId::BorderTopColor, Prop::BorderTopColor, G_BORDER),
    (LonghandId::BorderRightColor, Prop::BorderRightColor, G_BORDER),
    (LonghandId::BorderBottomColor, Prop::BorderBottomColor, G_BORDER),
    (LonghandId::BorderLeftColor, Prop::BorderLeftColor, G_BORDER),
    (LonghandId::BorderTopLeftRadius, Prop::BorderTopLeftRadius, G_BORDER),
    (LonghandId::BorderTopRightRadius, Prop::BorderTopRightRadius, G_BORDER),
    (
        LonghandId::BorderBottomLeftRadius,
        Prop::BorderBottomLeftRadius,
        G_BORDER,
    ),
    (
        LonghandId::BorderBottomRightRadius,
        Prop::BorderBottomRightRadius,
        G_BORDER,
    ),
    // outline
    (LonghandId::OutlineStyle, Prop::OutlineStyle, G_OUTLINE),
    (LonghandId::OutlineWidth, Prop::OutlineWidth, G_OUTLINE),
    (LonghandId::OutlineColor, Prop::OutlineColor, G_OUTLINE),
    (LonghandId::OutlineOffset, Prop::OutlineOffset, G_OUTLINE),
    // background
    (LonghandId::BackgroundColor, Prop::BackgroundColor, G_BACKGROUND),
    (LonghandId::BackgroundImage, Prop::BackgroundImage, G_BACKGROUND),
    // inset
    (LonghandId::InsetBlockStart, Prop::InsetBlockStart, G_INSET),
    (LonghandId::Top, Prop::InsetBlockStart, G_INSET),
    (LonghandId::InsetBlockEnd, Prop::InsetBlockEnd, G_INSET),
    (LonghandId::Bottom, Prop::InsetBlockEnd, G_INSET),
    (LonghandId::InsetInlineStart, Prop::InsetInlineStart, G_INSET),
    (LonghandId::Left, Prop::InsetInlineStart, G_INSET),
    (LonghandId::InsetInlineEnd, Prop::InsetInlineEnd, G_INSET),
    (LonghandId::Right, Prop::InsetInlineEnd, G_INSET),
    // flex
    (LonghandId::FlexBasis, Prop::FlexBasis, G_FLEX),
    (LonghandId::FlexDirection, Prop::FlexDirection, G_FLEX),
    (LonghandId::FlexGrow, Prop::FlexGrow, G_FLEX),
    (LonghandId::FlexShrink, Prop::FlexShrink, G_FLEX),
    (LonghandId::FlexWrap, Prop::FlexWrap, G_FLEX),
    (LonghandId::RowGap, Prop::RowGap, G_FLEX),
    (LonghandId::ColumnGap, Prop::ColumnGap, G_FLEX),
    (LonghandId::AlignItems, Prop::AlignItems, G_FLEX),
    (LonghandId::AlignSelf, Prop::AlignSelf, G_FLEX),
    (LonghandId::AlignContent, Prop::AlignContent, G_FLEX),
    (LonghandId::JustifyItems, Prop::JustifyItems, G_FLEX),
    (LonghandId::JustifySelf, Prop::JustifySelf, G_FLEX),
    (LonghandId::JustifyContent, Prop::JustifyContent, G_FLEX),
    // grid
    (LonghandId::GridTemplateRows, Prop::GridTemplateRows, G_GRID),
    (LonghandId::GridTemplateColumns, Prop::GridTemplateColumns, G_GRID),
    (LonghandId::GridAutoRows, Prop::GridAutoRows, G_GRID),
    (LonghandId::GridAutoColumns, Prop::GridAutoColumns, G_GRID),
    (LonghandId::GridRowStart, Prop::GridRowStart, G_GRID),
    (LonghandId::GridRowEnd, Prop::GridRowEnd, G_GRID),
    (LonghandId::GridColumnStart, Prop::GridColumnStart, G_GRID),
    (LonghandId::GridColumnEnd, Prop::GridColumnEnd, G_GRID),
    (LonghandId::GridTemplateAreas, Prop::GridTemplateAreas, G_GRID),
    (LonghandId::GridAutoFlow, Prop::GridAutoFlow, G_GRID),
];

/// One group bitmask per [`PropertyId::index`], so asking which group a declaration can reach
/// is an array index.
fn group_table() -> &'static [u16; PROPERTY_COUNT] {
    static TABLE: LazyLock<[u16; PROPERTY_COUNT]> = LazyLock::new(|| {
        let mut table = [0u16; PROPERTY_COUNT];
        for (id, _, group) in ENGINE_LONGHANDS {
            if let Some(slot) = table.get_mut(PropertyId::Longhand(*id).index()) {
                *slot |= *group;
            }
        }
        table
    });
    &TABLE
}

/// The property a longhand answers for: the first row [`ENGINE_LONGHANDS`] gives it, or `None`
/// for a longhand the table does not list.
fn prop_of(id: LonghandId) -> Option<Prop> {
    static TABLE: LazyLock<[Option<Prop>; LONGHAND_COUNT]> = LazyLock::new(|| {
        let mut table = [None; LONGHAND_COUNT];
        for (longhand, prop, _) in ENGINE_LONGHANDS {
            if let Some(slot) = table.get_mut(PropertyId::Longhand(*longhand).index()) {
                slot.get_or_insert(*prop);
            }
        }
        table
    });
    TABLE.get(PropertyId::Longhand(id).index()).copied().flatten()
}

/// The groups this element's own cascade can have changed.
///
/// A group nothing declared is exactly the group the parent already has, or the initial one,
/// so it is not built at all. Read off the map's entries rather than by comparing values, so
/// the answer costs one pass over what the element actually declared.
///
/// An entry that carries no declaration is skipped where the property inherits: what an
/// element inherits is not something it said, and the inherited group of an element that said
/// nothing is its parent's.
fn touched_groups(map: &CssProperties) -> u16 {
    let table = group_table();
    let mut touched = 0;
    for (id, property) in map.iter_ids() {
        if property.declared.is_empty() && id.inherited() {
            continue;
        }
        touched |= table.get(id.index()).copied().unwrap_or(0);
    }
    touched
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

    let touched = touched_groups(map);
    let mut style = ComputedStyle::inherit_from(parent);

    if touched & G_INHERITED != 0 {
        resolve_font(map, &mut style, parent);
        let font_size = style.inherited.font_size;
        resolve_inherited(map, &mut style, font_size);
        // The border colours name `currentColor`, and the colour they name only just became
        // known. Where the borders are built below they are resolved there instead.
        if touched & G_BORDER == 0 {
            style.default_border_colors();
        }
    }
    let font_size = style.inherited.font_size;

    if touched & G_BOX != 0 {
        resolve_box(map, &mut style);
    }
    if touched & G_SIZE != 0 {
        resolve_sizes(map, &mut style, font_size);
    }
    if touched & G_MARGIN != 0 {
        resolve_margin(map, &mut style, font_size);
    }
    if touched & G_PADDING != 0 {
        resolve_padding(map, &mut style, font_size);
    }
    if touched & G_BORDER != 0 {
        resolve_borders(map, &mut style, font_size);
    }
    if touched & G_OUTLINE != 0 {
        resolve_outline(map, &mut style, font_size);
    }
    if touched & G_BACKGROUND != 0 {
        resolve_background(map, &mut style);
    }
    if touched & G_INSET != 0 {
        resolve_insets(map, &mut style, font_size);
    }
    if touched & G_FLEX != 0 {
        resolve_flex(map, &mut style, font_size);
    }
    if touched & G_GRID != 0 {
        resolve_grid(map, &mut style);
    }

    style
}

/// `font-family` and `font-size`, which everything else is measured against.
fn resolve_font(map: &CssProperties, style: &mut ComputedStyle, parent: Option<&ComputedStyle>) {
    read!(map, style, FontFamily, inherited_mut, font_family, font_family);

    let parent_font_size = parent.map_or(16.0, |parent| parent.inherited.font_size);

    let Some(declared) = value_or_inherited(map, longhand(LonghandId::FontSize)) else {
        // Nothing anywhere up the chain declared a size, so a bare generic `monospace` family
        // gets the smaller default browsers give it.
        if !style.inherited.font_size_declared_in_chain && family_is_monospace(&style.inherited.font_family) {
            style.inherited_mut().font_size = MONOSPACE_DEFAULT_FONT_SIZE;
        }
        return;
    };
    style.inherited_mut().font_size_declared_in_chain = true;
    style.declared.set(Prop::FontSize);

    // An `em` (or `ex`, `ch`) on `font-size` is a multiple of the *parent's* size, which is the
    // basis the cascade already resolved it against. What is left is the keywords, and a
    // percentage on the off chance the computed stage did not settle it.
    style.inherited_mut().font_size = if as_unit(declared).is_some() {
        declared.unit_to_px()
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
    read!(map, style, Color, inherited_mut, color, color);

    read!(map, style, FontStyle, inherited_mut, font_style, |value| as_string(
        value
    )
    .map(|keyword| match keyword {
        "italic" => FontStyle::Italic,
        "oblique" => FontStyle::Oblique,
        _ => FontStyle::Normal,
    }));

    read!(map, style, FontWeight, inherited_mut, font_weight, |value| {
        if let Some(number) = as_number(value) {
            return Some(FontWeight::Number(number));
        }
        as_string(value).map(|keyword| match keyword {
            "bold" => FontWeight::Bold,
            "bolder" => FontWeight::Bolder,
            "lighter" => FontWeight::Lighter,
            _ => FontWeight::Normal,
        })
    });

    // A percentage `line-height` is a fraction of the element's own font-size, which is exactly
    // what an `em` means here, so both land as pixels. A unitless number stays a number: it
    // inherits as a multiplier rather than as the length it happens to be worth here.
    read!(map, style, LineHeight, inherited_mut, line_height, |value| {
        if as_unit(value).is_some() {
            return Some(LineHeight::Px(value.unit_to_px()));
        }
        if let Some(pct) = as_percentage(value) {
            return Some(LineHeight::Px(font_size * pct / 100.0));
        }
        if let Some(number) = as_number(value) {
            return Some(LineHeight::Number(number));
        }
        // `normal`, and anything else that is not a length: the font metrics decide.
        as_string(value).map(|_| LineHeight::Normal)
    });

    // The two keywords that are defined by the parent's value read it here, while the style still
    // holds what it inherited. `match-parent` computes to the parent's value (CSS Text 3 §6.1),
    // and `-internal-center` is the HTML rendering section's `<th>` rule: centre, but only where
    // the parent left `text-align` at its initial value.
    let parent_align = style.inherited.text_align;
    read!(map, style, TextAlign, inherited_mut, text_align, |value| as_string(
        value
    )
    .and_then(TextAlignKeyword::from_ident)
    .map(|keyword| match keyword {
        TextAlignKeyword::Left => TextAlign::Left,
        TextAlignKeyword::Right => TextAlign::Right,
        TextAlignKeyword::Center => TextAlign::Center,
        TextAlignKeyword::Justify => TextAlign::Justify,
        // `justify-all` also justifies the last line (css-text-3 §7.1); the line boxes know only
        // one justification, and `justify` is the nearer of the two.
        TextAlignKeyword::JustifyAll => TextAlign::Justify,
        TextAlignKeyword::Start => TextAlign::Start,
        TextAlignKeyword::End => TextAlign::End,
        TextAlignKeyword::MatchParent | TextAlignKeyword::WebkitMatchParent => parent_align,
        TextAlignKeyword::InternalCenter if parent_align == TextAlign::Start => TextAlign::Center,
        TextAlignKeyword::InternalCenter => parent_align,
        TextAlignKeyword::WebkitLeft => TextAlign::WebkitLeft,
        TextAlignKeyword::WebkitRight => TextAlign::WebkitRight,
        TextAlignKeyword::WebkitCenter => TextAlign::WebkitCenter,
    }));

    read!(map, style, TextTransform, inherited_mut, text_transform, |value| {
        as_string(value).map(|keyword| match keyword {
            "uppercase" => TextTransform::Uppercase,
            "lowercase" => TextTransform::Lowercase,
            "capitalize" => TextTransform::Capitalize,
            _ => TextTransform::None,
        })
    });

    resolve_text_decoration(map, style);

    read!(map, style, WhiteSpace, inherited_mut, white_space, |value| as_string(
        value
    )
    .and_then(WhiteSpaceKeyword::from_ident)
    .map(|keyword| match keyword {
        WhiteSpaceKeyword::Normal => WhiteSpace::Normal,
        WhiteSpaceKeyword::Pre => WhiteSpace::Pre,
        WhiteSpaceKeyword::Nowrap => WhiteSpace::NoWrap,
        WhiteSpaceKeyword::PreWrap => WhiteSpace::PreWrap,
        WhiteSpaceKeyword::PreLine => WhiteSpace::PreLine,
        WhiteSpaceKeyword::BreakSpaces => WhiteSpace::BreakSpaces,
    }));

    read!(map, style, LetterSpacing, inherited_mut, letter_spacing, |value| Some(
        match length_percentage(value, font_size) {
            Some(length) => LetterSpacing::Length(length),
            None => LetterSpacing::Normal,
        }
    ));

    read!(map, style, CaptionSide, inherited_mut, caption_side, |value| as_string(
        value
    )
    .and_then(CaptionSideKeyword::from_ident)
    .map(|keyword| match keyword {
        CaptionSideKeyword::Top => CaptionSide::Top,
        CaptionSideKeyword::Bottom => CaptionSide::Bottom,
    }));

    read!(map, style, BorderCollapse, inherited_mut, border_collapse, |value| {
        keyword(value).map(|keyword| match keyword {
            BorderCollapseKeyword::Collapse => BorderCollapse::Collapse,
            BorderCollapseKeyword::Separate => BorderCollapse::Separate,
        })
    });

    resolve_border_spacing(map, style, font_size);
}

/// `text-decoration-line`, from the longhand the `text-decoration` shorthand expands to.
///
/// A `text-decoration` that names no line resets the longhand to its initial `none`, which is a
/// declaration like any other: it takes the underline away (`a { text-decoration: red }` is not
/// underlined).
fn resolve_text_decoration(map: &CssProperties, style: &mut ComputedStyle) {
    let Some(property) = map.get_id(longhand(LonghandId::TextDecorationLine)) else {
        return;
    };
    let mut line = TextDecorationLine::NONE;
    let mut read = |keyword: &str| match keyword {
        "underline" => line.underline = true,
        "line-through" => line.line_through = true,
        _ => {}
    };
    match &property.computed {
        CssValue::String(keyword) => read(keyword),
        CssValue::List(values) => values.iter().filter_map(as_string).for_each(read),
        _ => {}
    }
    style.inherited_mut().text_decoration_line = line;
    style.declared.set(Prop::TextDecorationLine);
}

/// `border-spacing` is one declaration feeding two axes: one length applies to both, two are
/// horizontal then vertical (CSS 2 §17.6.1).
fn resolve_border_spacing(map: &CssProperties, style: &mut ComputedStyle, font_size: f32) {
    let Some(declared) = value_or_inherited(map, longhand(LonghandId::BorderSpacing)) else {
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
        style.inherited_mut().border_spacing_x = x;
        style.declared.set(Prop::BorderSpacingX);
    }
    // A single length applies to both axes; with two, the second is the vertical one.
    if let Some(y) = y.or(x) {
        style.inherited_mut().border_spacing_y = y;
        style.declared.set(Prop::BorderSpacingY);
    }
}

fn resolve_box(map: &CssProperties, style: &mut ComputedStyle) {
    read!(map, style, Display, box_mut, display, |value| as_string(value)
        .map(display_of));

    read!(map, style, Position, box_mut, position, |value| as_string(value).map(
        |keyword| match keyword {
            "relative" => Position::Relative,
            "absolute" => Position::Absolute,
            "fixed" => Position::Fixed,
            "sticky" => Position::Sticky,
            _ => Position::Static,
        }
    ));

    read!(map, style, Float, box_mut, float, |value| {
        as_string(value).map(|keyword| match keyword {
            "left" => Float::Left,
            "right" => Float::Right,
            _ => Float::None,
        })
    });

    read!(map, style, Clear, box_mut, clear, |value| {
        keyword(value).map(|keyword| match keyword {
            // The logical sides are the physical ones in the horizontal, left-to-right writing
            // mode the engine lays out in (css-logical-1 §2.2).
            ClearKeyword::Left | ClearKeyword::InlineStart => Clear::Left,
            ClearKeyword::Right | ClearKeyword::InlineEnd => Clear::Right,
            ClearKeyword::Both | ClearKeyword::BothInline => Clear::Both,
            // The block-axis values clear page floats (css-page-floats-3), which the engine does
            // not lay out, so there is nothing for them to clear.
            ClearKeyword::None
            | ClearKeyword::BlockStart
            | ClearKeyword::BlockEnd
            | ClearKeyword::Top
            | ClearKeyword::Bottom
            | ClearKeyword::BothBlock => Clear::None,
        })
    });

    read!(map, style, BoxSizing, box_mut, box_sizing, |value| keyword(value).map(
        |keyword| match keyword {
            BoxSizingKeyword::BorderBox => BoxSizing::BorderBox,
            BoxSizingKeyword::ContentBox => BoxSizing::ContentBox,
        }
    ));

    // `overflow-x` and `overflow-y` share one grammar, so one enum reads both.
    read!(map, style, OverflowX, box_mut, overflow_x, |value| keyword(value)
        .map(overflow_of));
    read!(map, style, OverflowY, box_mut, overflow_y, |value| keyword(value)
        .map(overflow_of));

    read!(map, style, ZIndex, box_mut, z_index, |value| {
        if let Some(number) = as_number(value) {
            return Some(ZIndex::Index(number));
        }
        as_string(value).map(|_| ZIndex::Auto)
    });

    read!(map, style, Opacity, box_mut, opacity, as_number);

    read!(map, style, MixBlendMode, box_mut, mix_blend_mode, |value| as_string(
        value
    )
    .map(Arc::from));

    read!(map, style, Resize, box_mut, resize, |value| {
        as_string(value).map(Arc::from)
    });

    read!(map, style, ScrollbarWidth, box_mut, scrollbar_width, |value| as_number(
        value
    )
    .map(Some));

    read!(map, style, AspectRatio, box_mut, aspect_ratio, |value| as_number(value)
        .map(Some));

    // `text-wrap` is a shorthand for these two (css-text-4 §5.1); one field holds both, since a
    // line that does not wrap has no style of wrapping.
    let mode = value(map, longhand(LonghandId::TextWrapMode)).and_then(as_string);
    let wrap_style = value(map, longhand(LonghandId::TextWrapStyle)).and_then(as_string);
    if mode.is_some() || wrap_style.is_some() {
        style.box_mut().text_wrap = match (mode, wrap_style) {
            (Some("nowrap"), _) => TextWrap::NoWrap,
            (_, Some("balance")) => TextWrap::Balance,
            (_, Some("pretty")) => TextWrap::Pretty,
            (_, Some("stable")) => TextWrap::Stable,
            _ => TextWrap::Wrap,
        };
        style.declared.set(Prop::TextWrap);
    }

    read!(map, style, TableLayout, box_mut, table_layout, |value| keyword(value)
        .map(|keyword| match keyword {
            TableLayoutKeyword::Fixed => TableLayout::Fixed,
            TableLayoutKeyword::Auto => TableLayout::Auto,
        }));

    read!(map, style, VerticalAlign, box_mut, vertical_align, |value| as_string(
        value
    )
    .map(|keyword| match keyword {
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
    }));
}

fn overflow_of(keyword: OverflowXKeyword) -> Overflow {
    match keyword {
        OverflowXKeyword::Visible => Overflow::Visible,
        OverflowXKeyword::Hidden => Overflow::Hidden,
        OverflowXKeyword::Clip => Overflow::Clip,
        OverflowXKeyword::Scroll => Overflow::Scroll,
        OverflowXKeyword::Auto => Overflow::Auto,
    }
}

fn resolve_sizes(map: &CssProperties, style: &mut ComputedStyle, font_size: f32) {
    let lpa = |value: &CssValue| Some(length_percentage_auto(value, font_size));

    read!(map, style, Width, size_mut, width, lpa);
    read!(map, style, Height, size_mut, height, lpa);
    read!(map, style, MinWidth, size_mut, min_width, lpa);
    read!(map, style, MinHeight, size_mut, min_height, lpa);
    read!(map, style, MaxWidth, size_mut, max_width, lpa);
    read!(map, style, MaxHeight, size_mut, max_height, lpa);
}

fn resolve_margin(map: &CssProperties, style: &mut ComputedStyle, font_size: f32) {
    let lpa = |value: &CssValue| Some(length_percentage_auto(value, font_size));

    read!(map, style, MarginTop, margin_mut, top, lpa);
    read!(map, style, MarginRight, margin_mut, right, lpa);
    read!(map, style, MarginBottom, margin_mut, bottom, lpa);
    read!(map, style, MarginLeft, margin_mut, left, lpa);
}

fn resolve_padding(map: &CssProperties, style: &mut ComputedStyle, font_size: f32) {
    let lp = |value: &CssValue| length_percentage(value, font_size);

    read!(map, style, PaddingTop, padding_mut, top, lp);
    read!(map, style, PaddingRight, padding_mut, right, lp);
    read!(map, style, PaddingBottom, padding_mut, bottom, lp);
    read!(map, style, PaddingLeft, padding_mut, left, lp);
}

fn resolve_borders(map: &CssProperties, style: &mut ComputedStyle, font_size: f32) {
    let bstyle = |value: &CssValue| keyword(value).map(border_style_of);
    read!(map, style, BorderTopStyle, border_mut, top_style, bstyle);
    read!(map, style, BorderRightStyle, border_mut, right_style, bstyle);
    read!(map, style, BorderBottomStyle, border_mut, bottom_style, bstyle);
    read!(map, style, BorderLeftStyle, border_mut, left_style, bstyle);

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
    style.border_mut().top_width = width_or_zero(style.border.top_style.is_visible(), LonghandId::BorderTopWidth);
    style.border_mut().right_width = width_or_zero(style.border.right_style.is_visible(), LonghandId::BorderRightWidth);
    style.border_mut().bottom_width =
        width_or_zero(style.border.bottom_style.is_visible(), LonghandId::BorderBottomWidth);
    style.border_mut().left_width = width_or_zero(style.border.left_style.is_visible(), LonghandId::BorderLeftWidth);

    // `currentColor` is the initial value of every border colour, so an undeclared one renders
    // in the element's own text colour: `td { border: solid; color: blue }` draws blue borders.
    let current = style.inherited.color;
    style.border_mut().top_color = current;
    style.border_mut().right_color = current;
    style.border_mut().bottom_color = current;
    style.border_mut().left_color = current;
    let border_color = move |value: &CssValue| color_or_current(value, current);
    read!(map, style, BorderTopColor, border_mut, top_color, border_color);
    read!(map, style, BorderRightColor, border_mut, right_color, border_color);
    read!(map, style, BorderBottomColor, border_mut, bottom_color, border_color);
    read!(map, style, BorderLeftColor, border_mut, left_color, border_color);

    let lp = |value: &CssValue| length_percentage(value, font_size);
    read!(map, style, BorderTopLeftRadius, border_mut, top_left_radius, lp);
    read!(map, style, BorderTopRightRadius, border_mut, top_right_radius, lp);
    read!(map, style, BorderBottomLeftRadius, border_mut, bottom_left_radius, lp);
    read!(map, style, BorderBottomRightRadius, border_mut, bottom_right_radius, lp);
}

fn resolve_outline(map: &CssProperties, style: &mut ComputedStyle, font_size: f32) {
    read!(map, style, OutlineStyle, outline_mut, style, |value| keyword(value)
        .map(|keyword| match keyword {
            // `auto`, the user-agent focus ring, paints as a solid line.
            OutlineStyleKeyword::Auto | OutlineStyleKeyword::Solid => BorderStyle::Solid,
            OutlineStyleKeyword::None => BorderStyle::None,
            OutlineStyleKeyword::Dotted => BorderStyle::Dotted,
            OutlineStyleKeyword::Dashed => BorderStyle::Dashed,
            OutlineStyleKeyword::Double => BorderStyle::Double,
            OutlineStyleKeyword::Groove => BorderStyle::Groove,
            OutlineStyleKeyword::Ridge => BorderStyle::Ridge,
            OutlineStyleKeyword::Inset => BorderStyle::Inset,
            OutlineStyleKeyword::Outset => BorderStyle::Outset,
        }));

    let declared_width = value(map, longhand(LonghandId::OutlineWidth));
    if declared_width.is_some() {
        style.declared.set(Prop::OutlineWidth);
    }
    let width = declared_width
        .and_then(|value| length_px(value, font_size))
        .unwrap_or(MEDIUM_BORDER_WIDTH);
    style.outline_mut().width = if style.outline.style.is_visible() { width } else { 0.0 };

    // Unlike the border colours, an undeclared `outline-color` has always been plain black
    // here. Only the keywords follow the text colour: `currentColor`, and the `auto` that is
    // the property's real initial value.
    let current = style.inherited.color;
    read!(map, style, OutlineColor, outline_mut, color, move |value: &CssValue| {
        if as_string(value).is_some_and(|k| k.eq_ignore_ascii_case("auto")) {
            return Some(current);
        }
        color_or_current(value, current)
    });

    read!(map, style, OutlineOffset, outline_mut, offset, |value| length_px(
        value, font_size
    ));
}

fn resolve_background(map: &CssProperties, style: &mut ComputedStyle) {
    let current = style.inherited.color;
    read!(
        map,
        style,
        BackgroundColor,
        background_mut,
        color,
        move |value: &CssValue| color_or_current(value, current)
    );
    if let Some(url) = value(map, longhand(LonghandId::BackgroundImage)).and_then(first_url) {
        style.background_mut().image = Some(Arc::from(url));
        style.declared.set(Prop::BackgroundImage);
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
            0 => style.inset_mut().block_start = resolved,
            1 => style.inset_mut().block_end = resolved,
            2 => style.inset_mut().inline_start = resolved,
            _ => style.inset_mut().inline_end = resolved,
        }
        style.declared.set(prop);
    }
}

fn resolve_flex(map: &CssProperties, style: &mut ComputedStyle, font_size: f32) {
    read!(map, style, FlexBasis, flex_mut, basis, |value| Some(
        length_percentage_auto(value, font_size)
    ));

    read!(map, style, FlexDirection, flex_mut, direction, |value| keyword(value)
        .map(|keyword| match keyword {
            FlexDirectionKeyword::Row => FlexDirection::Row,
            FlexDirectionKeyword::RowReverse => FlexDirection::RowReverse,
            FlexDirectionKeyword::Column => FlexDirection::Column,
            FlexDirectionKeyword::ColumnReverse => FlexDirection::ColumnReverse,
        }));

    read!(map, style, FlexGrow, flex_mut, grow, as_number);
    read!(map, style, FlexShrink, flex_mut, shrink, as_number);

    read!(map, style, FlexWrap, flex_mut, wrap, |value| as_string(value).map(
        |keyword| match keyword {
            "wrap" => FlexWrap::Wrap,
            "wrap-reverse" => FlexWrap::WrapReverse,
            _ => FlexWrap::NoWrap,
        }
    ));

    // `normal` is 0 in flex and grid layout, which is all that reads these, so it is left to the
    // initial value rather than mapped.
    read!(map, style, RowGap, flex_mut, row_gap, |value| {
        length_percentage(value, font_size)
    });
    read!(map, style, ColumnGap, flex_mut, column_gap, |value| length_percentage(
        value, font_size
    ));

    let align = |value: &CssValue| as_string(value).map(align_of);
    read!(map, style, AlignItems, flex_mut, align_items, align);
    read!(map, style, AlignSelf, flex_mut, align_self, align);
    read!(map, style, AlignContent, flex_mut, align_content, align);
    read!(map, style, JustifyItems, flex_mut, justify_items, align);
    read!(map, style, JustifySelf, flex_mut, justify_self, align);
    read!(map, style, JustifyContent, flex_mut, justify_content, align);
}

fn resolve_grid(map: &CssProperties, style: &mut ComputedStyle) {
    let track_list = |value: &CssValue| grid_track_list(value).map(Arc::from);
    read!(map, style, GridTemplateRows, grid_mut, template_rows, track_list);
    read!(map, style, GridTemplateColumns, grid_mut, template_columns, track_list);
    read!(map, style, GridAutoRows, grid_mut, auto_rows, track_list);
    read!(map, style, GridAutoColumns, grid_mut, auto_columns, track_list);

    for (id, prop) in [
        (LonghandId::GridRowStart, Prop::GridRowStart),
        (LonghandId::GridRowEnd, Prop::GridRowEnd),
        (LonghandId::GridColumnStart, Prop::GridColumnStart),
        (LonghandId::GridColumnEnd, Prop::GridColumnEnd),
    ] {
        let Some(line) = value(map, longhand(id)).and_then(grid_placement) else {
            continue;
        };
        let line: Arc<str> = Arc::from(line);
        let grid = style.grid_mut();
        match prop {
            Prop::GridRowStart => grid.row_start = line,
            Prop::GridRowEnd => grid.row_end = line,
            Prop::GridColumnStart => grid.column_start = line,
            _ => grid.column_end = line,
        }
        style.declared.set(prop);
    }

    read!(map, style, GridTemplateAreas, grid_mut, template_areas, |value| {
        grid_areas(value).map(Arc::from)
    });

    read!(map, style, GridAutoFlow, grid_mut, auto_flow, |value| as_string(value)
        .map(|keyword| match keyword {
            "column" => GridAutoFlow::Column,
            "row dense" => GridAutoFlow::RowDense,
            "column dense" => GridAutoFlow::ColumnDense,
            _ => GridAutoFlow::Row,
        }));
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
                value: std::sync::Arc::new(value.clone()),
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

    /// The cascaded map for a declaration block written as CSS, the way a stylesheet produces
    /// one: parsed, validated and expanded, with later declarations winning. This is the helper
    /// for anything a shorthand's expansion decides.
    fn map_of(css: &str) -> CssProperties {
        let config = gosub_shared::config::ParserConfig {
            ignore_errors: true,
            ..Default::default()
        };
        let sheet = crate::Css3::parse_str(&format!("a {{ {css} }}"), config, CssOrigin::Author, "test")
            .expect("the test stylesheet parses");
        let mut map = CssProperties::new();
        let mut order = 0;
        for expanded in sheet.rules[0].expanded() {
            let crate::matcher::expansion::ExpandedDeclaration::Resolved { entries, important } = expanded else {
                continue;
            };
            for (id, value) in entries {
                order += 1;
                if map.get_id(*id).is_none() {
                    map.insert_id(*id, CssProperty::new(*id));
                }
                let property = map.get_id_mut(*id).expect("just inserted");
                property.declared.push(DeclarationProperty {
                    value: std::sync::Arc::clone(value),
                    origin: CssOrigin::Author,
                    important: *important,
                    location: no_location(),
                    specificity: Specificity::new(0, 0, 1),
                    shadow_depth: 0,
                    order,
                    layer: None,
                    attached: false,
                });
            }
        }
        for (_, property) in map.iter_ids_mut() {
            property.font_size_basis = 16.0;
            property.root_font_size_basis = 16.0;
            property.mark_dirty();
            property.compute_value();
        }
        map
    }

    fn style_of(css: &str) -> ComputedStyle {
        computed_style(&map_of(css), None)
    }

    /// The typed style reads the longhands a shorthand expands to, so the later declaration wins
    /// whichever of the two it is written as.
    #[test]
    fn a_longhand_after_its_shorthand_wins() {
        let underline = TextDecorationLine {
            underline: true,
            line_through: false,
        };
        assert_eq!(
            style_of("text-decoration: underline").inherited.text_decoration_line,
            underline
        );
        // The user-agent `a` rule against an author override, in one block.
        assert_eq!(
            style_of("text-decoration: underline; text-decoration-line: none")
                .inherited
                .text_decoration_line,
            TextDecorationLine::NONE
        );
        // A shorthand naming no line resets it, so this is not underlined either.
        let red = style_of("text-decoration: red");
        assert_eq!(red.inherited.text_decoration_line, TextDecorationLine::NONE);
        assert!(red.has(Prop::TextDecorationLine));
        assert_eq!(
            style_of("text-decoration: underline line-through")
                .inherited
                .text_decoration_line,
            TextDecorationLine {
                underline: true,
                line_through: true,
            }
        );

        let red = Color::rgba(255, 0, 0, 255);
        let blue = Color::rgba(0, 0, 255, 255);
        assert_eq!(
            style_of("background: red; background-color: blue").background.color,
            blue
        );
        assert_eq!(
            style_of("background-color: blue; background: red").background.color,
            red
        );
        let layers = style_of("background: url(a.png), linear-gradient(red, blue) green");
        assert_eq!(layers.background.color, Color::rgba(0, 128, 0, 255));
        assert_eq!(layers.background.image.as_deref(), Some("a.png"));
    }

    /// A CSS-wide keyword on a shorthand reaches every longhand (css-cascade-5 §7.3), so the
    /// cascade resolves each of them - and a later shorthand `inherit` beats an earlier longhand.
    #[test]
    fn a_css_wide_keyword_on_a_shorthand_sets_its_longhands() {
        let declared = |css: &str, name: &str| {
            let map = map_of(css);
            let id = PropertyId::from_name(name).expect("a longhand");
            map.get_id(id)
                .and_then(|property| property.declared.last().map(|d| (*d.value).clone()))
        };
        let inherit = Some(CssValue::String("inherit".to_string()));
        assert_eq!(
            declared(
                "text-decoration-line: underline; text-decoration: inherit",
                "text-decoration-line"
            ),
            inherit
        );
        for name in ["margin-top", "margin-right", "margin-bottom", "margin-left"] {
            assert_eq!(declared("margin: inherit", name), inherit, "{name}");
        }
        // Through a nested shorthand as well: `border` holds `border-top`, which holds these.
        assert_eq!(
            declared("border: initial", "border-top-width"),
            Some(CssValue::String("initial".to_string()))
        );
    }

    /// A system colour is a colour like any other: it can be the origin of a relative colour
    /// and a colour in a mix, and a deprecated one takes the value css-color-4 §6.3 maps it to.
    #[test]
    fn system_colours_resolve_everywhere_a_colour_does() {
        let face = Color::rgba(240, 240, 240, 255);
        assert_eq!(style_of("color: ButtonShadow").inherited.color, face);
        assert_eq!(
            style_of("background-color: rgb(from ButtonFace r g b)")
                .background
                .color,
            face
        );
        assert_eq!(
            style_of("background-color: color-mix(in srgb, ButtonShadow, black)")
                .background
                .color,
            Color::rgba(120, 120, 120, 255)
        );
        assert_eq!(
            style_of("color: GrayText").inherited.color,
            Color::rgba(128, 128, 128, 255)
        );
    }

    /// `initial` is the property's initial value even where that is `none` or left to the UA,
    /// so it resets an inherited property instead of keeping what the parent had.
    #[test]
    fn initial_resets_an_inherited_property() {
        let parent = style_of("text-transform: uppercase; font-family: monospace");
        let child = computed_style(&map_of("text-transform: initial; font-family: initial"), Some(&parent));
        assert_eq!(child.inherited.text_transform, TextTransform::None);
        assert_eq!(&*child.inherited.font_family, "serif");
    }

    /// Every property the typed style records has a row in [`ENGINE_LONGHANDS`], and no
    /// longhand is in two groups.
    #[test]
    fn the_property_table_covers_every_prop_once() {
        for prop in Prop::ALL {
            assert!(
                ENGINE_LONGHANDS.iter().any(|(_, row, _)| row == prop),
                "{prop:?} has no row in ENGINE_LONGHANDS"
            );
        }
        for (id, _, group) in ENGINE_LONGHANDS {
            assert!(
                ENGINE_LONGHANDS
                    .iter()
                    .all(|(other, _, other_group)| other != id || other_group == group),
                "{} is in two groups",
                id.name()
            );
        }
    }

    /// Declaring each longhand in the table records its property. A longhand listed in the
    /// wrong group fails here: the group's resolver never runs for an element that touched only
    /// that longhand, so the value is silently not read.
    #[test]
    fn every_listed_longhand_is_read() {
        // Where the initial value is one the reader deliberately leaves to the default (`normal`
        // gaps, no image), a value it does record.
        let samples = [
            ("row-gap", "1px"),
            ("column-gap", "1px"),
            ("background-image", "url(a.png)"),
            ("aspect-ratio", "2"),
        ];
        // Read, but never recorded, whatever is declared. Each is a bug in its reader.
        let known_unread = [
            // The reader takes a number, which `auto | thin | none` never is: the field is always
            // unset.
            "scrollbar-width",
        ];
        let mut unread = Vec::new();
        for (id, prop, _) in ENGINE_LONGHANDS {
            if known_unread.contains(&id.name()) {
                continue;
            }
            let value = samples
                .iter()
                .find(|(name, _)| *name == id.name())
                .map_or("initial", |(_, value)| value);
            let style = style_of(&format!("{}: {value}", id.name()));
            if !style.has(*prop) {
                unread.push(format!("{} -> {prop:?}", id.name()));
            }
        }
        assert!(unread.is_empty(), "declared but not recorded:\n{}", unread.join("\n"));
    }

    /// One generated enum reads the four `border-*-style` longhands and both `overflow-*`, which
    /// is only right while their grammars agree.
    #[test]
    fn longhands_read_through_one_enum_share_its_keywords() {
        use crate::matcher::keywords::{
            BorderBottomStyleKeyword, BorderLeftStyleKeyword, BorderRightStyleKeyword, OverflowYKeyword,
        };
        fn names<K: Keyword>() -> Vec<&'static str> {
            K::ALL.iter().map(|keyword| keyword.name()).collect()
        }
        let top = names::<BorderTopStyleKeyword>();
        assert_eq!(names::<BorderRightStyleKeyword>(), top);
        assert_eq!(names::<BorderBottomStyleKeyword>(), top);
        assert_eq!(names::<BorderLeftStyleKeyword>(), top);
        assert_eq!(names::<OverflowYKeyword>(), names::<OverflowXKeyword>());
    }

    /// The logical `clear` values are the physical sides in the left-to-right, horizontal writing
    /// mode the engine lays out in, and `justify-all` justifies.
    #[test]
    fn logical_and_newer_keywords_mean_what_they_say() {
        assert_eq!(style_of("clear: inline-start").box_group.clear, Clear::Left);
        assert_eq!(style_of("clear: inline-end").box_group.clear, Clear::Right);
        assert_eq!(style_of("clear: both-inline").box_group.clear, Clear::Both);
        assert_eq!(
            style_of("text-align: justify-all").inherited.text_align,
            TextAlign::Justify
        );
    }

    /// A linear `calc()` is carried as a length plus a percentage; anything else is left unset.
    #[test]
    fn a_linear_calc_is_a_length_and_a_percentage() {
        let calc = |px, percent| LengthPercentageAuto::Calc { px, percent };
        assert_eq!(style_of("width: calc(100% - 20px)").size.width, calc(-20.0, 100.0));
        assert_eq!(style_of("width: calc(50% - 1rem)").size.width, calc(-16.0, 50.0));
        assert_eq!(style_of("width: calc(-5vw + 50%)").size.width, calc(-64.0, 50.0));
        assert_eq!(
            style_of("padding-left: calc(10% + 4px)").padding.left,
            LengthPercentage::Calc { px: 4.0, percent: 10.0 }
        );
        // A vendor-prefixed spelling is the same function.
        assert_eq!(style_of("width: -moz-calc(50% + 10px)").size.width, calc(10.0, 50.0));
        assert_eq!(
            style_of("width: -webkit-calc(10px + 2px)").size.width,
            LengthPercentageAuto::Px(12.0)
        );
        // A calc that reduces to one term is that term.
        assert_eq!(
            style_of("width: calc(10px + 2rem)").size.width,
            LengthPercentageAuto::Px(42.0)
        );
        assert_eq!(
            style_of("width: calc(50%)").size.width,
            LengthPercentageAuto::Percent(50.0)
        );
        // Not a sum: it needs the basis before it is a length at all, and reads as `auto`, the
        // value every non-length in a size slot has always had.
        assert_eq!(
            style_of("width: min(100%, 600px)").size.width,
            LengthPercentageAuto::Auto
        );
        // Resolved against a basis, as layout resolves a percentage.
        assert_eq!(calc(-20.0, 100.0).resolve(300.0), Some(280.0));
    }

    #[test]
    fn text_wrap_is_read_from_its_two_longhands() {
        assert_eq!(style_of("text-wrap-mode: nowrap").box_group.text_wrap, TextWrap::NoWrap);
        assert_eq!(style_of("text-wrap: balance").box_group.text_wrap, TextWrap::Balance);
        assert_eq!(
            style_of("text-wrap: nowrap balance").box_group.text_wrap,
            TextWrap::NoWrap
        );
        assert_eq!(style_of("text-wrap: wrap").box_group.text_wrap, TextWrap::Wrap);
    }

    /// `gap` and the `place-*` shorthands copy an omitted second value from the first
    /// (css-align-3 §5.5, §6.2, §6.3, §8.3), and each axis can be set on its own.
    #[test]
    fn the_second_axis_copies_the_first_when_omitted() {
        let gaps = |css: &str| {
            let style = style_of(css);
            (style.flex.row_gap, style.flex.column_gap)
        };
        let px = LengthPercentage::Px;
        assert_eq!(gaps("gap: 5px"), (px(5.0), px(5.0)));
        assert_eq!(gaps("gap: 10px 20px"), (px(10.0), px(20.0)));
        assert_eq!(gaps("column-gap: 12px"), (px(0.0), px(12.0)));
        assert_eq!(gaps("gap: 4px; row-gap: 9px"), (px(9.0), px(4.0)));

        let items = style_of("place-items: center");
        assert_eq!(
            (items.flex.align_items, items.flex.justify_items),
            (AlignValue::Center, AlignValue::Center)
        );
        let content = style_of("place-content: space-between");
        assert_eq!(
            (content.flex.align_content, content.flex.justify_content),
            (AlignValue::SpaceBetween, AlignValue::SpaceBetween)
        );
        let baseline = style_of("place-content: baseline");
        assert_eq!(
            (baseline.flex.align_content, baseline.flex.justify_content),
            (AlignValue::Baseline, AlignValue::Start)
        );
        let own = style_of("place-self: end");
        assert_eq!(
            (own.flex.align_self, own.flex.justify_self),
            (AlignValue::End, AlignValue::End)
        );
    }

    /// The grid shorthands arrive as their four lines, in source order: a later `grid-row` after
    /// `grid-area` now moves the item, which the layouter's own shorthand reader could not do.
    #[test]
    fn grid_placement_is_read_from_the_four_lines() {
        let lines = |css: &str| {
            let style = style_of(css);
            let grid = &style.grid;
            [
                grid.row_start.to_string(),
                grid.row_end.to_string(),
                grid.column_start.to_string(),
                grid.column_end.to_string(),
            ]
        };
        assert_eq!(
            lines("grid-area: content"),
            ["content", "content", "content", "content"]
        );
        assert_eq!(lines("grid-area: 2 / 1 / 4 / 3"), ["2", "4", "1", "3"]);
        assert_eq!(lines("grid-row: span 2"), ["span 2", "auto", "auto", "auto"]);
        assert_eq!(
            lines("grid-area: content; grid-row: 2"),
            ["2", "auto", "content", "content"]
        );
        assert_eq!(
            lines("grid-row: 1 / 3; grid-area: auto"),
            ["auto", "auto", "auto", "auto"]
        );
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

    /// A group the element declared nothing in is not built: it is the group the parent has, or
    /// the one process-wide group holding that group's initial values. Asserted by pointer,
    /// because equal values would pass whether or not any of this works.
    #[test]
    fn an_untouched_group_is_the_one_above_it() {
        let parent = computed_style(&map(&[("color", keyword("red")), ("width", px(100.0))]), None);
        let child = computed_style(&map(&[("width", px(50.0))]), Some(&parent));

        // Nothing inherited was declared, so the inherited group is the parent's own.
        assert!(Arc::ptr_eq(&child.inherited, &parent.inherited));
        // The size group was, so it is this element's.
        assert!(!Arc::ptr_eq(&child.size, &parent.size));
        assert_eq!(child.size.width, LengthPercentageAuto::Px(50.0));
        // Neither element declared a margin, so both point at the initial one.
        assert!(Arc::ptr_eq(&child.margin, &ComputedStyle::initial().margin));
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

    /// A colour function built on `currentcolor` stays a function in the computed value, and
    /// folds here once the element's colour is known.
    #[test]
    fn current_color_inside_a_colour_function_folds() {
        let contrast = || CssValue::Function("contrast-color".to_string(), vec![keyword("currentcolor")]);

        let style = computed_style(
            &map(&[
                ("color", keyword("white")),
                ("background-color", contrast()),
                ("border-left-color", contrast()),
                ("outline-color", contrast()),
            ]),
            None,
        );
        let black = Color::rgba(0, 0, 0, 255);
        assert_eq!(style.background.color, black);
        assert_eq!(style.border.left_color, black);
        assert_eq!(style.outline.color, black);

        // Through the `background` shorthand, on its own and among the other layer tokens.
        let white = Color::rgba(255, 255, 255, 255);
        for (css, expected) in [
            ("color: white; background: contrast-color(currentcolor)", black),
            (
                "color: white; background: url(\"x.gif\") contrast-color(currentcolor)",
                black,
            ),
            ("color: white; background: url(\"x.gif\") currentcolor", white),
        ] {
            assert_eq!(style_of(css).background.color, expected, "{css}");
        }

        // On `color` itself `currentcolor` is the inherited colour, initial black here.
        let style = computed_style(&map(&[("color", contrast())]), None);
        assert_eq!(style.inherited.color, Color::rgba(255, 255, 255, 255));
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

    /// `text-decoration` reaches the typed style through the longhand it expands to.
    #[test]
    fn text_decoration_is_read_through_its_longhand() {
        let style = style_of("text-decoration: none");
        assert_eq!(style.inherited.text_decoration_line, TextDecorationLine::NONE);
        assert!(style.has(Prop::TextDecorationLine));

        assert!(
            style_of("text-decoration: underline")
                .inherited
                .text_decoration_line
                .underline
        );
        assert!(
            style_of("text-decoration-line: line-through")
                .inherited
                .text_decoration_line
                .line_through
        );
    }

    /// The `background` shorthand's colour and `url()` reach the typed style through its
    /// longhands.
    #[test]
    fn the_background_shorthand_gives_up_its_colour_and_image() {
        let style = style_of("background: #ffffff url(\"grayarrow.gif\") no-repeat");
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
                // The cascade measures every property but `font-size` against the element's own
                // font-size.
                20.0,
            ),
            None,
        );
        assert_eq!(style.size.max_width, LengthPercentageAuto::Px(17.0 * 0.55 * 20.0));
        assert_eq!(style.size.min_width, LengthPercentageAuto::Px(2.0 * 0.5 * 20.0));
    }

    /// Every length unit the grammar accepts has a conversion. These all used to be read as
    /// their bare number of pixels: `50cqw` was 50px.
    #[test]
    fn every_accepted_length_unit_converts() {
        crate::stylesheet::set_layout_viewport(1280.0, 800.0);
        let width = |value: f64, unit: &str| {
            let style = computed_style(
                &map_with(&[("width", CssValue::Unit(value, unit.to_string()))], 16.0),
                None,
            );
            style.size.width
        };
        let px = LengthPercentageAuto::Px;
        // No element is a query container, so container units are the small viewport's.
        assert_eq!(width(50.0, "cqw"), px(640.0));
        assert_eq!(width(10.0, "cqmin"), px(80.0));
        // The logical viewport units, in the horizontal writing mode.
        assert_eq!(width(10.0, "vi"), px(128.0));
        assert_eq!(width(10.0, "svb"), px(80.0));
        assert_eq!(width(10.0, "dvmin"), px(80.0));
        assert_eq!(width(10.0, "lvmax"), px(128.0));
        // The root font-metric units, against the root's 16px here.
        assert_eq!(width(2.0, "rlh"), px(2.0 * 1.4 * 16.0));
        assert_eq!(width(10.0, "rch"), px(10.0 * 0.55 * 16.0));
        assert_eq!(width(1.0, "cap"), px(0.7 * 16.0));
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
