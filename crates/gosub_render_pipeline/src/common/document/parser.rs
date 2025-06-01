use crate::common::document::style::TextAlign;
use regex::Regex;
use serde::Deserialize;
use std::collections::HashMap;
use crate::common::document::document::Document;
use crate::common::document::node::{AttrMap, NodeId, NodeType};
use crate::common::document::style::{Color, Display, FontWeight, StyleProperty, StylePropertyList, StyleValue, TextWrap, Unit};
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

// Text is "as-is" from the JSON, but we don't want text with multiple spaces and newlines.
fn clean_text(input: &str) -> String {
    let no_newlines = input.replace('\n', " ");
    let space_regex = Regex::new(r"\s{2,}").unwrap();
    space_regex.replace_all(&no_newlines, " ").to_string()
}

fn create_dom_from_json(doc: &mut Document, node: &DomNode, parent_id: Option<NodeId>) -> Option<NodeId> {
    let mut attrs = AttrMap::new();
    for (key, value) in &node.attributes {
        attrs.set(key, value);
    }

    if let Some(text) = &node.text {
        // When we encounter text, we don't have any style, but we need to use the styles from the parent.
        let parent_node = doc.get_node_by_id(parent_id.unwrap()).unwrap();
        let parent_styles = match &parent_node.node_type {
            NodeType::Element(parent_element) => Some(parent_element.styles.clone()),
            _ => None,
        };
        return Some(doc.new_text(parent_id, clean_text(text).as_str(), parent_styles));
    }

    if let Some(comment) = &node.comment {
        return Some(doc.new_comment(parent_id, comment));
    }

    let Some(tag) = &node.tag else {
        eprintln!("Warning: Encountered node without a tag! {:?}", node);
        return None;
    };

    let style = get_style_from_node(node);
    let node_id = doc.new_element(parent_id, &tag, Some(attrs), node.self_closing, Some(style.clone()));

    // if node_id.is_greater_than(12) {
    //     return None
    // }

    for child in &node.children {
        match create_dom_from_json(doc, child, Some(node_id)) {
            Some(child_node_id) => doc.add_child(node_id, child_node_id),
            None => {}
        }
    }

    Some(node_id)
}

fn get_style_from_node(node: &DomNode) -> StylePropertyList {
    let mut style = StylePropertyList::new();

    for (key, value) in &node.styles {
        match key.as_str() {
            "display" => style.set_property(StyleProperty::Display, parse_display(value)),
            "position" => style.set_property(StyleProperty::Position, parse_position(value)),

            "width" => style.set_property(StyleProperty::Width, parse_style_value(value)),
            "height" => style.set_property(StyleProperty::Height, parse_style_value(value)),
            "max-width" => style.set_property(StyleProperty::MaxWidth, parse_style_value(value)),
            "min-width" => style.set_property(StyleProperty::MinWidth, parse_style_value(value)),
            "max-height" => style.set_property(StyleProperty::MaxHeight, parse_style_value(value)),
            "min-height" => style.set_property(StyleProperty::MinHeight, parse_style_value(value)),

            "border-top-width" => style.set_property(StyleProperty::BorderTopWidth, parse_style_value(value)),
            "border-left-width" => style.set_property(StyleProperty::BorderLeftWidth, parse_style_value(value)),
            "border-right-width" => style.set_property(StyleProperty::BorderRightWidth, parse_style_value(value)),
            "border-bottom-width" => style.set_property(StyleProperty::BorderBottomWidth, parse_style_value(value)),
            "border-bottom-left-radius" => style.set_property(StyleProperty::BorderBottomLeftRadius, parse_style_value(value)),
            "border-bottom-right-radius" => style.set_property(StyleProperty::BorderBottomRightRadius, parse_style_value(value)),
            "border-top-left-radius" => style.set_property(StyleProperty::BorderTopLeftRadius, parse_style_value(value)),
            "border-top-right-radius" => style.set_property(StyleProperty::BorderTopRightRadius, parse_style_value(value)),
            "border-top-color" => style.set_property(StyleProperty::BorderTopColor, StyleValue::Color(Color::Named(value.to_string()))),
            "border-left-color" => style.set_property(StyleProperty::BorderLeftColor, StyleValue::Color(Color::Named(value.to_string()))),
            "border-right-color" => style.set_property(StyleProperty::BorderRightColor, StyleValue::Color(Color::Named(value.to_string()))),
            "border-bottom-color" => style.set_property(StyleProperty::BorderBottomColor, StyleValue::Color(Color::Named(value.to_string()))),

            "margin-top" => style.set_property(StyleProperty::MarginTop, parse_style_value(value)),
            "margin-left" => style.set_property(StyleProperty::MarginLeft, parse_style_value(value)),
            "margin-right" => style.set_property(StyleProperty::MarginRight, parse_style_value(value)),
            "margin-bottom" => style.set_property(StyleProperty::MarginBottom, parse_style_value(value)),

            "padding-top" => style.set_property(StyleProperty::PaddingTop, parse_style_value(value)),
            "padding-left" => style.set_property(StyleProperty::PaddingLeft, parse_style_value(value)),
            "padding-right" => style.set_property(StyleProperty::PaddingRight, parse_style_value(value)),
            "padding-bottom" => style.set_property(StyleProperty::PaddingBottom, parse_style_value(value)),

            "color" => style.set_property(StyleProperty::Color, StyleValue::Color(Color::Named(value.to_string()))),
            "background-color" => style.set_property(StyleProperty::BackgroundColor, StyleValue::Color(Color::Named(value.to_string()))),

            "font-weight" => style.set_property(StyleProperty::FontWeight, parse_font_weight(value)),
            "font-size" => style.set_property(StyleProperty::FontSize, parse_style_value(value)),
            "font-family" => style.set_property(StyleProperty::FontFamily, StyleValue::Keyword(value.to_string())),

            "flex-basis" => style.set_property(StyleProperty::FlexBasis, parse_style_str(value)),
            "flex-direction" => style.set_property(StyleProperty::FlexDirection, parse_style_str(value)),
            "flex-grow" => style.set_property(StyleProperty::FlexGrow, parse_style_num(value)),
            "flex-shrink" => style.set_property(StyleProperty::FlexShrink, parse_style_num(value)),
            "flex-wrap" => style.set_property(StyleProperty::FlexWrap, parse_style_str(value)),

            "aspect-ratio" => style.set_property(StyleProperty::AspectRatio, parse_style_num(value)),
            "gap" => style.set_property(StyleProperty::Gap, parse_style_value(value)),
            "align-items" => style.set_property(StyleProperty::AlignItems, parse_style_str(value)),
            "align-self" => style.set_property(StyleProperty::AlignSelf, parse_style_str(value)),
            "align-content" => style.set_property(StyleProperty::AlignContent, parse_style_str(value)),
            "text-align" => style.set_property(StyleProperty::TextAlign, parse_text_align(value)),
            "line-height" => style.set_property(StyleProperty::LineHeight, parse_style_value(value)),
            "text-wrap" => style.set_property(StyleProperty::TextWrap, parse_text_wrap(value)),

            "inset-block-end" => style.set_property(StyleProperty::InsetBlockEnd, parse_style_value(value)),
            "inset-block-start" => style.set_property(StyleProperty::InsetBlockStart, parse_style_value(value)),
            "inset-inline-end" => style.set_property(StyleProperty::InsetInlineEnd, parse_style_value(value)),
            "inset-inline-start" => style.set_property(StyleProperty::InsetInlineStart, parse_style_value(value)),

            "justify-items" => style.set_property(StyleProperty::JustifyItems, parse_style_str(value)),
            "justify-self" => style.set_property(StyleProperty::JustifySelf, parse_style_str(value)),
            "justify-content" => style.set_property(StyleProperty::JustifyContent, parse_style_str(value)),

            "overflow-x" => style.set_property(StyleProperty::OverflowX, parse_style_str(value)),
            "overflow-y" => style.set_property(StyleProperty::OverflowY, parse_style_str(value)),
            "box-sizing" => style.set_property(StyleProperty::BoxSizing, parse_style_str(value)),

            _ => {}
        }
    }

    style
}

fn parse_text_wrap(value: &str) -> StyleValue {
    match value {
        "wrap" => StyleValue::TextWrap(TextWrap::Wrap),
        "nowrap" => StyleValue::TextWrap(TextWrap::NoWrap),
        "balance" => StyleValue::TextWrap(TextWrap::Balance),
        "pretty" => StyleValue::TextWrap(TextWrap::Pretty),
        "stable" => StyleValue::TextWrap(TextWrap::Stable),
        "initial" => StyleValue::TextWrap(TextWrap::Initial),
        "inherit" => StyleValue::TextWrap(TextWrap::Inherit),
        "revert" => StyleValue::TextWrap(TextWrap::Revert),
        "revert-layer" => StyleValue::TextWrap(TextWrap::RevertLayer),
        "unset" => StyleValue::TextWrap(TextWrap::Unset),
        _ => StyleValue::TextWrap(TextWrap::Wrap),
    }
}

fn parse_position(position: &str) -> StyleValue {
    StyleValue::Keyword(position.to_string())
}

fn parse_style_str(val: &str) -> StyleValue {
    StyleValue::Keyword(val.to_string())
}

fn parse_text_align(val: &str) -> StyleValue {
    match val {
        "left" => StyleValue::TextAlign(TextAlign::Start),
        "right" => StyleValue::TextAlign(TextAlign::End),
        "start" => StyleValue::TextAlign(TextAlign::Start),
        "end" => StyleValue::TextAlign(TextAlign::End),
        "center" => StyleValue::TextAlign(TextAlign::Center),
        "justify" => StyleValue::TextAlign(TextAlign::Justify),
        _ => StyleValue::TextAlign(TextAlign::Start)
    }
}

fn parse_style_num(val: &str) -> StyleValue {
    if let Ok(num) = val.parse::<f32>() {
        StyleValue::Number(num)
    } else {
        StyleValue::Keyword(val.to_string())
    }
}

fn parse_display(value: &String) -> StyleValue {
    match value.as_str() {
        "block" => StyleValue::Display(Display::Block),
        "inline" => StyleValue::Display(Display::Inline),
        "inline-block" => StyleValue::Display(Display::InlineBlock),
        "none" => StyleValue::Display(Display::None),
        "flex" => StyleValue::Display(Display::Flex),
        "table" => StyleValue::Display(Display::Table),
        "table-caption" => StyleValue::Display(Display::TableCaption),
        "table-cell" => StyleValue::Display(Display::TableCell),
        "table-footer-group" => StyleValue::Display(Display::TableFooterGroup),
        "table-header-group" => StyleValue::Display(Display::TableHeaderGroup),
        "table-row" => StyleValue::Display(Display::TableRow),
        "table-row-group" => StyleValue::Display(Display::TableRowGroup),
        _ => StyleValue::Keyword(value.to_string()),
    }
}

fn parse_style_value(value: &str) -> StyleValue {
    if let Ok(px_value) = value.replace("px", "").parse::<f32>() {
        StyleValue::Unit(px_value, Unit::Px)
    } else {
        StyleValue::Keyword(value.to_string())
    }
}

fn parse_font_weight(value: &str) -> StyleValue {
    match value {
        "bold" | "bolder" => StyleValue::FontWeight(FontWeight::Bolder),
        "lighter" => StyleValue::FontWeight(FontWeight::Lighter),
        "normal" => StyleValue::FontWeight(FontWeight::Normal),
        _ => {
            if let Ok(num) = value.parse::<f32>() {
                StyleValue::FontWeight(FontWeight::Number(num))
            } else {
                StyleValue::Keyword(value.to_string())
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