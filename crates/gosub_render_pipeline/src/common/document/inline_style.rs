use crate::common::document::style::{
    intern, BorderStyle, Display, FontWeight, NodeStyle, StyleProperty, TextAlign, TextWrap, Unit, Value,
};
use cow_utils::CowUtils;
use std::collections::HashMap;

/// Parses a CSS inline `style` attribute value (e.g. `"color: red; width: 100px"`)
/// into a `NodeStyle`.
pub fn parse_inline_style_attr(style_attr: &str) -> NodeStyle {
    let mut style = NodeStyle::new();
    for declaration in style_attr.split(';') {
        let declaration = declaration.trim();
        if declaration.is_empty() {
            continue;
        }
        if let Some((key, value)) = declaration.split_once(':') {
            apply_style_kv(&mut style, key.trim(), value.trim());
        }
    }
    style
}

/// Extract the target of a `url(...)` token from a CSS value string, stripping surrounding
/// quotes. Returns `None` when there is no `url()` (e.g. a gradient or plain color).
fn parse_css_url(value: &str) -> Option<String> {
    let start = value.find("url(")? + "url(".len();
    let rest = &value[start..];
    let end = rest.find(')')?;
    let inner = rest[..end].trim().trim_matches(['"', '\'']).trim();
    (!inner.is_empty()).then(|| inner.to_string())
}

fn parse_background_color_token(value: &str) -> Option<Value> {
    for token in value.split_whitespace() {
        let v = parse_named_color(token);
        if matches!(v, Value::Color(..)) {
            return Some(v);
        }
    }
    None
}

fn parse_named_color(value: &str) -> Value {
    match value {
        "black" => Value::Color(0, 0, 0, 255),
        "white" => Value::Color(255, 255, 255, 255),
        "red" => Value::Color(255, 0, 0, 255),
        "green" => Value::Color(0, 128, 0, 255),
        "blue" => Value::Color(0, 0, 255, 255),
        "yellow" => Value::Color(255, 255, 0, 255),
        "orange" => Value::Color(255, 165, 0, 255),
        "purple" => Value::Color(128, 0, 128, 255),
        "pink" => Value::Color(255, 192, 203, 255),
        "gray" | "grey" => Value::Color(128, 128, 128, 255),
        "silver" => Value::Color(192, 192, 192, 255),
        "maroon" => Value::Color(128, 0, 0, 255),
        "navy" => Value::Color(0, 0, 128, 255),
        "teal" => Value::Color(0, 128, 128, 255),
        "aqua" | "cyan" => Value::Color(0, 255, 255, 255),
        "fuchsia" | "magenta" => Value::Color(255, 0, 255, 255),
        "lime" => Value::Color(0, 255, 0, 255),
        "olive" => Value::Color(128, 128, 0, 255),
        "transparent" => Value::Color(0, 0, 0, 0),
        s if s.starts_with("rgb(") => parse_rgb(s),
        s if s.starts_with("rgba(") => parse_rgba(s),
        s if s.starts_with('#') => parse_hex_color(s),
        _ => Value::Keyword(intern(value)),
    }
}

fn parse_rgb(s: &str) -> Value {
    let inner = s.trim_start_matches("rgb(").trim_end_matches(')');
    let parts: Vec<&str> = inner.split(',').collect();
    if parts.len() == 3 {
        let r = parts[0].trim().parse::<u8>().unwrap_or(0);
        let g = parts[1].trim().parse::<u8>().unwrap_or(0);
        let b = parts[2].trim().parse::<u8>().unwrap_or(0);
        return Value::Color(r, g, b, 255);
    }
    Value::Keyword(intern(s))
}

fn parse_rgba(s: &str) -> Value {
    let inner = s.trim_start_matches("rgba(").trim_end_matches(')');
    let parts: Vec<&str> = inner.split(',').collect();
    if parts.len() == 4 {
        let r = parts[0].trim().parse::<u8>().unwrap_or(0);
        let g = parts[1].trim().parse::<u8>().unwrap_or(0);
        let b = parts[2].trim().parse::<u8>().unwrap_or(0);
        let a = (parts[3].trim().parse::<f32>().unwrap_or(1.0) * 255.0) as u8;
        return Value::Color(r, g, b, a);
    }
    Value::Keyword(intern(s))
}

fn parse_hex_color(s: &str) -> Value {
    let hex = s.trim_start_matches('#');
    match hex.len() {
        6 => {
            if let (Ok(r), Ok(g), Ok(b)) = (
                u8::from_str_radix(&hex[0..2], 16),
                u8::from_str_radix(&hex[2..4], 16),
                u8::from_str_radix(&hex[4..6], 16),
            ) {
                return Value::Color(r, g, b, 255);
            }
        }
        8 => {
            if let (Ok(r), Ok(g), Ok(b), Ok(a)) = (
                u8::from_str_radix(&hex[0..2], 16),
                u8::from_str_radix(&hex[2..4], 16),
                u8::from_str_radix(&hex[4..6], 16),
                u8::from_str_radix(&hex[6..8], 16),
            ) {
                return Value::Color(r, g, b, a);
            }
        }
        3 => {
            if let (Ok(r), Ok(g), Ok(b)) = (
                u8::from_str_radix(&hex[0..1].repeat(2), 16),
                u8::from_str_radix(&hex[1..2].repeat(2), 16),
                u8::from_str_radix(&hex[2..3].repeat(2), 16),
            ) {
                return Value::Color(r, g, b, 255);
            }
        }
        _ => {}
    }
    Value::Keyword(intern(s))
}

fn parse_box_shorthand(value: &str) -> Vec<Value> {
    let parts: Vec<&str> = value.split_whitespace().collect();
    match parts.len() {
        2 => vec![
            parse_style_value(parts[0]),
            parse_style_value(parts[1]),
            parse_style_value(parts[0]),
            parse_style_value(parts[1]),
        ],
        3 => vec![
            parse_style_value(parts[0]),
            parse_style_value(parts[1]),
            parse_style_value(parts[2]),
            parse_style_value(parts[1]),
        ],
        4 => vec![
            parse_style_value(parts[0]),
            parse_style_value(parts[1]),
            parse_style_value(parts[2]),
            parse_style_value(parts[3]),
        ],
        _ => {
            let v = parse_style_value(parts.first().copied().unwrap_or(value));
            vec![v.clone(), v.clone(), v.clone(), v]
        }
    }
}

fn is_border_style_keyword(s: &str) -> bool {
    matches!(
        s,
        "solid" | "dashed" | "dotted" | "double" | "groove" | "ridge" | "inset" | "outset" | "hidden" | "none"
    )
}

fn apply_border_shorthand(style: &mut NodeStyle, value: &str) {
    let mut width = Value::Unit(3.0, Unit::Px);
    let mut bstyle = Value::BorderStyle(BorderStyle::None);
    let mut color = Value::Color(0, 0, 0, 255);

    for part in value.split_whitespace() {
        match parse_style_value(part) {
            v @ Value::Unit(_, _) => width = v,
            _ if is_border_style_keyword(part) => bstyle = parse_border_style(part),
            _ => color = parse_named_color(part),
        }
    }

    for prop in &[
        StyleProperty::BorderTopWidth,
        StyleProperty::BorderRightWidth,
        StyleProperty::BorderBottomWidth,
        StyleProperty::BorderLeftWidth,
    ] {
        style.set(prop.clone(), width.clone());
    }
    for prop in &[
        StyleProperty::BorderTopStyle,
        StyleProperty::BorderRightStyle,
        StyleProperty::BorderBottomStyle,
        StyleProperty::BorderLeftStyle,
    ] {
        style.set(prop.clone(), bstyle.clone());
    }
    for prop in &[
        StyleProperty::BorderTopColor,
        StyleProperty::BorderRightColor,
        StyleProperty::BorderBottomColor,
        StyleProperty::BorderLeftColor,
    ] {
        style.set(prop.clone(), color.clone());
    }
}

fn apply_style_kv(style: &mut NodeStyle, key: &str, value: &str) {
    match key {
        "display" => style.set(StyleProperty::Display, parse_display(value)),
        "position" => style.set(StyleProperty::Position, parse_position(value)),

        "width" => style.set(StyleProperty::Width, parse_style_value(value)),
        "height" => style.set(StyleProperty::Height, parse_style_value(value)),
        "max-width" => style.set(StyleProperty::MaxWidth, parse_style_value(value)),
        "min-width" => style.set(StyleProperty::MinWidth, parse_style_value(value)),
        "max-height" => style.set(StyleProperty::MaxHeight, parse_style_value(value)),
        "min-height" => style.set(StyleProperty::MinHeight, parse_style_value(value)),

        "border-top-width" => style.set(StyleProperty::BorderTopWidth, parse_style_value(value)),
        "border-left-width" => style.set(StyleProperty::BorderLeftWidth, parse_style_value(value)),
        "border-right-width" => style.set(StyleProperty::BorderRightWidth, parse_style_value(value)),
        "border-bottom-width" => style.set(StyleProperty::BorderBottomWidth, parse_style_value(value)),
        "border-bottom-left-radius" => style.set(StyleProperty::BorderBottomLeftRadius, parse_style_value(value)),
        "border-bottom-right-radius" => style.set(StyleProperty::BorderBottomRightRadius, parse_style_value(value)),
        "border-top-left-radius" => style.set(StyleProperty::BorderTopLeftRadius, parse_style_value(value)),
        "border-top-right-radius" => style.set(StyleProperty::BorderTopRightRadius, parse_style_value(value)),
        "border-radius" => {
            let radii_part = value.split('/').next().unwrap_or(value).trim();
            let v = parse_box_shorthand(radii_part);
            style.set(StyleProperty::BorderTopLeftRadius, v[0].clone());
            style.set(StyleProperty::BorderTopRightRadius, v[1].clone());
            style.set(StyleProperty::BorderBottomRightRadius, v[2].clone());
            style.set(StyleProperty::BorderBottomLeftRadius, v[3].clone());
        }
        "border-top-style" => style.set(StyleProperty::BorderTopStyle, parse_border_style(value)),
        "border-right-style" => style.set(StyleProperty::BorderRightStyle, parse_border_style(value)),
        "border-bottom-style" => style.set(StyleProperty::BorderBottomStyle, parse_border_style(value)),
        "border-left-style" => style.set(StyleProperty::BorderLeftStyle, parse_border_style(value)),
        "border-top-color" => style.set(StyleProperty::BorderTopColor, parse_named_color(value)),
        "border-left-color" => style.set(StyleProperty::BorderLeftColor, parse_named_color(value)),
        "border-right-color" => style.set(StyleProperty::BorderRightColor, parse_named_color(value)),
        "border-bottom-color" => style.set(StyleProperty::BorderBottomColor, parse_named_color(value)),

        "margin" => {
            let v = parse_box_shorthand(value);
            style.set(StyleProperty::MarginTop, v[0].clone());
            style.set(StyleProperty::MarginRight, v[1].clone());
            style.set(StyleProperty::MarginBottom, v[2].clone());
            style.set(StyleProperty::MarginLeft, v[3].clone());
        }
        "margin-top" | "margin-block-start" => style.set(StyleProperty::MarginTop, parse_style_value(value)),
        "margin-left" | "margin-inline-start" => style.set(StyleProperty::MarginLeft, parse_style_value(value)),
        "margin-right" | "margin-inline-end" => style.set(StyleProperty::MarginRight, parse_style_value(value)),
        "margin-bottom" | "margin-block-end" => style.set(StyleProperty::MarginBottom, parse_style_value(value)),

        "padding" => {
            let v = parse_box_shorthand(value);
            style.set(StyleProperty::PaddingTop, v[0].clone());
            style.set(StyleProperty::PaddingRight, v[1].clone());
            style.set(StyleProperty::PaddingBottom, v[2].clone());
            style.set(StyleProperty::PaddingLeft, v[3].clone());
        }
        "padding-top" | "padding-block-start" => style.set(StyleProperty::PaddingTop, parse_style_value(value)),
        "padding-left" | "padding-inline-start" => style.set(StyleProperty::PaddingLeft, parse_style_value(value)),
        "padding-right" | "padding-inline-end" => style.set(StyleProperty::PaddingRight, parse_style_value(value)),
        "padding-bottom" | "padding-block-end" => style.set(StyleProperty::PaddingBottom, parse_style_value(value)),

        "border" => apply_border_shorthand(style, value),

        "color" => style.set(StyleProperty::Color, parse_named_color(value)),
        "background-color" => style.set(StyleProperty::BackgroundColor, parse_named_color(value)),
        "background" => {
            if let Some(color) = parse_background_color_token(value) {
                style.set(StyleProperty::BackgroundColor, color);
            }
            if let Some(url) = parse_css_url(value) {
                style.set(StyleProperty::BackgroundImage, Value::Keyword(intern(&url)));
            }
        }
        "background-image" => {
            if let Some(url) = parse_css_url(value) {
                style.set(StyleProperty::BackgroundImage, Value::Keyword(intern(&url)));
            }
        }

        "font-weight" => style.set(StyleProperty::FontWeight, parse_font_weight(value)),
        "font-style" => style.set(StyleProperty::FontStyle, Value::Keyword(intern(value))),
        "font-size" => style.set(StyleProperty::FontSize, parse_style_value(value)),
        "font-family" => style.set(StyleProperty::FontFamily, Value::Keyword(intern(value))),

        "flex-basis" => style.set(StyleProperty::FlexBasis, parse_style_value(value)),
        "flex-direction" => style.set(StyleProperty::FlexDirection, parse_style_str(value)),
        "flex-grow" => style.set(StyleProperty::FlexGrow, parse_style_num(value)),
        "flex-shrink" => style.set(StyleProperty::FlexShrink, parse_style_num(value)),
        "flex-wrap" => style.set(StyleProperty::FlexWrap, parse_style_str(value)),

        "grid-template-columns" => style.set(StyleProperty::GridTemplateColumns, Value::Keyword(intern(value))),
        "grid-template-rows" => style.set(StyleProperty::GridTemplateRows, Value::Keyword(intern(value))),
        "grid-auto-columns" => style.set(StyleProperty::GridAutoColumns, Value::Keyword(intern(value))),
        "grid-auto-rows" => style.set(StyleProperty::GridAutoRows, Value::Keyword(intern(value))),
        "grid-auto-flow" => style.set(StyleProperty::GridAutoFlow, Value::Keyword(intern(value))),
        "grid-column" => style.set(StyleProperty::GridColumn, Value::Keyword(intern(value))),
        "grid-row" => style.set(StyleProperty::GridRow, Value::Keyword(intern(value))),

        "aspect-ratio" => style.set(StyleProperty::AspectRatio, parse_style_num(value)),
        "gap" => style.set(StyleProperty::Gap, parse_style_value(value)),
        "align-items" => style.set(StyleProperty::AlignItems, parse_style_str(value)),
        "align-self" => style.set(StyleProperty::AlignSelf, parse_style_str(value)),
        "align-content" => style.set(StyleProperty::AlignContent, parse_style_str(value)),
        "text-align" => style.set(StyleProperty::TextAlign, parse_text_align(value)),
        "line-height" => style.set(StyleProperty::LineHeight, parse_line_height(value)),
        "text-wrap" => style.set(StyleProperty::TextWrap, parse_text_wrap(value)),

        "top" | "inset-block-start" => style.set(StyleProperty::InsetBlockStart, parse_style_value(value)),
        "bottom" | "inset-block-end" => style.set(StyleProperty::InsetBlockEnd, parse_style_value(value)),
        "left" | "inset-inline-start" => style.set(StyleProperty::InsetInlineStart, parse_style_value(value)),
        "right" | "inset-inline-end" => style.set(StyleProperty::InsetInlineEnd, parse_style_value(value)),

        "justify-items" => style.set(StyleProperty::JustifyItems, parse_style_str(value)),
        "justify-self" => style.set(StyleProperty::JustifySelf, parse_style_str(value)),
        "justify-content" => style.set(StyleProperty::JustifyContent, parse_style_str(value)),

        "overflow-x" => style.set(StyleProperty::OverflowX, parse_style_str(value)),
        "overflow-y" => style.set(StyleProperty::OverflowY, parse_style_str(value)),
        "box-sizing" => style.set(StyleProperty::BoxSizing, parse_style_str(value)),
        "white-space" => style.set(StyleProperty::WhiteSpace, parse_style_str(value)),
        "text-decoration" | "text-decoration-line" => {
            let has_underline = value.contains("underline");
            let has_line_through = value.contains("line-through");
            let kw = if has_underline && has_line_through {
                "underline line-through"
            } else if has_underline {
                "underline"
            } else if has_line_through {
                "line-through"
            } else {
                "none"
            };
            style.set(StyleProperty::TextDecorationLine, Value::Keyword(intern(kw)));
        }

        _ => {}
    }
}

fn parse_text_wrap(value: &str) -> Value {
    match value {
        "wrap" => Value::TextWrap(TextWrap::Wrap),
        "nowrap" => Value::TextWrap(TextWrap::NoWrap),
        "balance" => Value::TextWrap(TextWrap::Balance),
        "pretty" => Value::TextWrap(TextWrap::Pretty),
        "stable" => Value::TextWrap(TextWrap::Stable),
        "initial" => Value::TextWrap(TextWrap::Initial),
        "inherit" => Value::TextWrap(TextWrap::Inherit),
        "revert" => Value::TextWrap(TextWrap::Revert),
        "revert-layer" => Value::TextWrap(TextWrap::RevertLayer),
        "unset" => Value::TextWrap(TextWrap::Unset),
        _ => Value::TextWrap(TextWrap::Wrap),
    }
}

fn parse_position(position: &str) -> Value {
    Value::Keyword(intern(position))
}

fn parse_style_str(val: &str) -> Value {
    Value::Keyword(intern(val))
}

fn parse_text_align(val: &str) -> Value {
    match val {
        "left" => Value::TextAlign(TextAlign::Start),
        "right" => Value::TextAlign(TextAlign::End),
        "start" => Value::TextAlign(TextAlign::Start),
        "end" => Value::TextAlign(TextAlign::End),
        "center" => Value::TextAlign(TextAlign::Center),
        "justify" => Value::TextAlign(TextAlign::Justify),
        _ => Value::TextAlign(TextAlign::Start),
    }
}

fn parse_style_num(val: &str) -> Value {
    if let Ok(num) = val.parse::<f32>() {
        Value::Number(num)
    } else {
        Value::Keyword(intern(val))
    }
}

fn parse_display(value: &str) -> Value {
    match value {
        "block" => Value::Display(Display::Block),
        "inline" => Value::Display(Display::Inline),
        "inline-block" => Value::Display(Display::InlineBlock),
        "none" => Value::Display(Display::None),
        "flex" => Value::Display(Display::Flex),
        "inline-flex" => Value::Display(Display::InlineFlex),
        "grid" => Value::Display(Display::Grid),
        "inline-grid" => Value::Display(Display::InlineGrid),
        "table" => Value::Display(Display::Table),
        "table-caption" => Value::Display(Display::TableCaption),
        "table-cell" => Value::Display(Display::TableCell),
        "table-footer-group" => Value::Display(Display::TableFooterGroup),
        "table-header-group" => Value::Display(Display::TableHeaderGroup),
        "table-row" => Value::Display(Display::TableRow),
        "table-row-group" => Value::Display(Display::TableRowGroup),
        _ => Value::Keyword(intern(value)),
    }
}

fn parse_line_height(value: &str) -> Value {
    if !value.ends_with("px") && !value.ends_with("em") && !value.ends_with("rem") && !value.ends_with('%') {
        if let Ok(n) = value.parse::<f32>() {
            return Value::Number(n);
        }
    }
    parse_style_value(value)
}

fn parse_style_value(value: &str) -> Value {
    if let Ok(px_value) = value.cow_replace("px", "").parse::<f32>() {
        Value::Unit(px_value, Unit::Px)
    } else if let Ok(em_value) = value.cow_replace("__qem", "").parse::<f32>() {
        Value::Unit(em_value, Unit::Em)
    } else if let Ok(em_value) = value.cow_replace("rem", "").parse::<f32>() {
        Value::Unit(em_value, Unit::Rem)
    } else if let Ok(em_value) = value.cow_replace("em", "").parse::<f32>() {
        Value::Unit(em_value, Unit::Em)
    } else {
        Value::Keyword(intern(value))
    }
}

fn parse_border_style(value: &str) -> Value {
    let bs = match value {
        "solid" => BorderStyle::Solid,
        "dashed" => BorderStyle::Dashed,
        "dotted" => BorderStyle::Dotted,
        "double" => BorderStyle::Double,
        "groove" => BorderStyle::Groove,
        "ridge" => BorderStyle::Ridge,
        "inset" => BorderStyle::Inset,
        "outset" => BorderStyle::Outset,
        "hidden" => BorderStyle::Hidden,
        _ => BorderStyle::None,
    };
    Value::BorderStyle(bs)
}

fn parse_font_weight(value: &str) -> Value {
    match value {
        "bold" => Value::FontWeight(FontWeight::Bold),
        "bolder" => Value::FontWeight(FontWeight::Bolder),
        "lighter" => Value::FontWeight(FontWeight::Lighter),
        "normal" => Value::FontWeight(FontWeight::Normal),
        _ => {
            if let Ok(num) = value.parse::<f32>() {
                Value::FontWeight(FontWeight::Number(num))
            } else {
                Value::Keyword(intern(value))
            }
        }
    }
}

/// Map HTML presentation attributes (e.g. `bgcolor`, `width`) to CSS `Value`s.
/// These have lower specificity than any real CSS rule and are only consulted
/// when neither the `style` attribute nor the stylesheet provides a value.
pub fn html_presentation_attr(attrs: &HashMap<String, String>, prop: &StyleProperty) -> Option<Value> {
    match prop {
        StyleProperty::BackgroundColor => {
            let v = attrs.get("bgcolor")?;
            let color = parse_named_color(v.trim());
            if matches!(color, Value::Color(..)) {
                Some(color)
            } else {
                None
            }
        }
        StyleProperty::Width => {
            let v = attrs.get("width")?;
            let v = v.trim();
            if let Some(pct) = v.strip_suffix('%') {
                pct.trim().parse::<f32>().ok().map(|n| Value::Unit(n, Unit::Percent))
            } else {
                v.parse::<f32>().ok().map(|n| Value::Unit(n, Unit::Px))
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_url_from_value() {
        assert_eq!(parse_css_url("url(grayarrow.gif)").as_deref(), Some("grayarrow.gif"));
        assert_eq!(parse_css_url("url('a b.png')").as_deref(), Some("a b.png"));
        assert_eq!(parse_css_url(r#"url("x.gif") no-repeat"#).as_deref(), Some("x.gif"));
        assert_eq!(parse_css_url("#fff no-repeat"), None);
        assert_eq!(parse_css_url("url()"), None);
    }

    #[test]
    fn background_shorthand_sets_image_and_color() {
        let style = parse_inline_style_attr("background: #fff url(grayarrow.gif) no-repeat");
        assert!(matches!(
            style.get_own(&StyleProperty::BackgroundImage),
            Some(Value::Keyword(_))
        ));
        assert!(matches!(
            style.get_own(&StyleProperty::BackgroundColor),
            Some(Value::Color(255, 255, 255, 255))
        ));
    }

    #[test]
    fn background_image_longhand() {
        let style = parse_inline_style_attr("background-image: url(pic.png)");
        assert!(matches!(
            style.get_own(&StyleProperty::BackgroundImage),
            Some(Value::Keyword(_))
        ));
    }
}
