use crate::common::document::document::Document;
use crate::common::document::node::{AttrMap, NodeId};
use crate::common::document::style::{
    intern, BorderStyle, Display, FontWeight, NodeStyle, StyleProperty, TextAlign, TextWrap, Unit,
    Value,
};
use cow_utils::CowUtils;
use regex::Regex;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::OnceLock;
// This parses uses the tools/souper.py to load a JSON file and create a DOM from it. This allows us to render
// a webpage with minimal effort, and without connecting a whole html5 and css parser to it.

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DomNode {
    #[serde(default)]
    comment: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    tag: Option<String>,
    #[serde(default)]
    self_closing: bool,
    #[serde(default)]
    attributes: HashMap<String, String>,
    #[serde(default)]
    styles: HashMap<String, String>,
    #[serde(default)]
    children: Vec<DomNode>,
}

#[allow(unused)]
#[derive(Debug, Deserialize)]
struct DomRoot {
    tag: String,
    #[serde(default)]
    attributes: HashMap<String, String>,
    #[serde(default)]
    styles: HashMap<String, String>,
    children: Vec<DomNode>,
}

static SPACE_REGEX: OnceLock<Regex> = OnceLock::new();

// Text is "as-is" from the JSON, but we don't want text with multiple spaces and newlines.
fn clean_text(input: &str) -> String {
    let no_newlines = input.cow_replace('\n', " ");
    let space_regex = SPACE_REGEX.get_or_init(|| Regex::new(r"\s{2,}").unwrap());
    space_regex.replace_all(&no_newlines, " ").to_string()
}

fn create_dom_from_json(doc: &mut Document, node: &DomNode, parent_id: Option<NodeId>) -> Option<NodeId> {
    let mut attrs = AttrMap::new();
    for (key, value) in &node.attributes {
        attrs.set(key, value);
    }

    if let Some(text) = &node.text {
        // Text nodes carry no own style; inheritance is handled by get_style() parent chain.
        return Some(doc.new_text(parent_id, clean_text(text).as_str()));
    }

    if let Some(comment) = &node.comment {
        return Some(doc.new_comment(parent_id, comment));
    }

    let Some(tag) = &node.tag else {
        log::warn!("Encountered node without a tag! {:?}", node);
        return None;
    };

    let style = get_style_from_node(node);
    let node_id = doc.new_element(parent_id, tag, Some(attrs), node.self_closing, Some(style.clone()));

    for child in &node.children {
        if let Some(child_node_id) = create_dom_from_json(doc, child, Some(node_id)) {
            doc.add_child(node_id, child_node_id);
        }
    }

    Some(node_id)
}

fn get_style_from_node(node: &DomNode) -> NodeStyle {
    let mut style = NodeStyle::new();

    for (key, value) in &node.styles {
        match key.as_str() {
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
            "border-bottom-left-radius" => {
                style.set(StyleProperty::BorderBottomLeftRadius, parse_style_value(value))
            }
            "border-bottom-right-radius" => {
                style.set(StyleProperty::BorderBottomRightRadius, parse_style_value(value))
            }
            "border-top-left-radius" => {
                style.set(StyleProperty::BorderTopLeftRadius, parse_style_value(value))
            }
            "border-top-right-radius" => {
                style.set(StyleProperty::BorderTopRightRadius, parse_style_value(value))
            }
            "border-top-style" => style.set(StyleProperty::BorderTopStyle, parse_border_style(value)),
            "border-right-style" => style.set(StyleProperty::BorderRightStyle, parse_border_style(value)),
            "border-bottom-style" => style.set(StyleProperty::BorderBottomStyle, parse_border_style(value)),
            "border-left-style" => style.set(StyleProperty::BorderLeftStyle, parse_border_style(value)),
            "border-top-color" => style.set(StyleProperty::BorderTopColor, parse_named_color(value)),
            "border-left-color" => style.set(StyleProperty::BorderLeftColor, parse_named_color(value)),
            "border-right-color" => style.set(StyleProperty::BorderRightColor, parse_named_color(value)),
            "border-bottom-color" => style.set(StyleProperty::BorderBottomColor, parse_named_color(value)),

            "margin-top" | "margin-block-start" => style.set(StyleProperty::MarginTop, parse_style_value(value)),
            "margin-left" | "margin-inline-start" => style.set(StyleProperty::MarginLeft, parse_style_value(value)),
            "margin-right" | "margin-inline-end" => style.set(StyleProperty::MarginRight, parse_style_value(value)),
            "margin-bottom" | "margin-block-end" => style.set(StyleProperty::MarginBottom, parse_style_value(value)),

            "padding-top" | "padding-block-start" => style.set(StyleProperty::PaddingTop, parse_style_value(value)),
            "padding-left" | "padding-inline-start" => style.set(StyleProperty::PaddingLeft, parse_style_value(value)),
            "padding-right" | "padding-inline-end" => style.set(StyleProperty::PaddingRight, parse_style_value(value)),
            "padding-bottom" | "padding-block-end" => style.set(StyleProperty::PaddingBottom, parse_style_value(value)),

            "color" => style.set(StyleProperty::Color, parse_named_color(value)),
            "background-color" => style.set(StyleProperty::BackgroundColor, parse_named_color(value)),

            "font-weight" => style.set(StyleProperty::FontWeight, parse_font_weight(value)),
            "font-size" => style.set(StyleProperty::FontSize, parse_style_value(value)),
            "font-family" => style.set(StyleProperty::FontFamily, Value::Keyword(intern(value))),

            "flex-basis" => style.set(StyleProperty::FlexBasis, parse_style_str(value)),
            "flex-direction" => style.set(StyleProperty::FlexDirection, parse_style_str(value)),
            "flex-grow" => style.set(StyleProperty::FlexGrow, parse_style_num(value)),
            "flex-shrink" => style.set(StyleProperty::FlexShrink, parse_style_num(value)),
            "flex-wrap" => style.set(StyleProperty::FlexWrap, parse_style_str(value)),

            "aspect-ratio" => style.set(StyleProperty::AspectRatio, parse_style_num(value)),
            "gap" => style.set(StyleProperty::Gap, parse_style_value(value)),
            "align-items" => style.set(StyleProperty::AlignItems, parse_style_str(value)),
            "align-self" => style.set(StyleProperty::AlignSelf, parse_style_str(value)),
            "align-content" => style.set(StyleProperty::AlignContent, parse_style_str(value)),
            "text-align" => style.set(StyleProperty::TextAlign, parse_text_align(value)),
            "line-height" => style.set(StyleProperty::LineHeight, parse_style_value(value)),
            "text-wrap" => style.set(StyleProperty::TextWrap, parse_text_wrap(value)),

            "inset-block-end" => style.set(StyleProperty::InsetBlockEnd, parse_style_value(value)),
            "inset-block-start" => style.set(StyleProperty::InsetBlockStart, parse_style_value(value)),
            "inset-inline-end" => style.set(StyleProperty::InsetInlineEnd, parse_style_value(value)),
            "inset-inline-start" => style.set(StyleProperty::InsetInlineStart, parse_style_value(value)),

            "justify-items" => style.set(StyleProperty::JustifyItems, parse_style_str(value)),
            "justify-self" => style.set(StyleProperty::JustifySelf, parse_style_str(value)),
            "justify-content" => style.set(StyleProperty::JustifyContent, parse_style_str(value)),

            "overflow-x" => style.set(StyleProperty::OverflowX, parse_style_str(value)),
            "overflow-y" => style.set(StyleProperty::OverflowY, parse_style_str(value)),
            "box-sizing" => style.set(StyleProperty::BoxSizing, parse_style_str(value)),

            _ => {}
        }
    }

    style
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

pub fn document_from_json(base_url: &str, path: &str) -> Document {
    let mut doc = Document::new(base_url);

    let json_data = std::fs::read_to_string(path).expect("Failed to read JSON file");
    let dom_root: DomRoot = serde_json::from_str(&json_data).expect("Failed to parse JSON");

    let root_node_id = doc.new_element(None, "DocumentRoot", None, false, None);
    for node in dom_root.children {
        if let Some(child_node_id) = create_dom_from_json(&mut doc, &node, Some(root_node_id)) {
            doc.add_child(root_node_id, child_node_id);
        }
    }

    doc.set_root(root_node_id);
    doc
}
