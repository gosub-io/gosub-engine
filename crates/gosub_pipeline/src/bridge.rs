//! Bridge between the `gosub_interface` DOM and the `gosub_pipeline` document model.
//!
//! # Style extraction
//!
//! Styles are extracted in two passes:
//! 1. **Full CSS cascade** via `CssSystem::properties_from_node` — applies the UA stylesheet,
//!    all author stylesheets and computes the cascade for the node.
//! 2. **Inline styles** from the `style="…"` attribute are overlaid on top as a safety net
//!    (the cascade already handles them, but this ensures they always win).

use std::sync::Arc;

use gosub_interface::config::HasDocument;
use gosub_interface::css3::{CssProperty, CssPropertyMap, CssSystem};
use gosub_interface::document::Document as GosubDoc;
use gosub_interface::node::{CommentDataType, ElementDataType, Node as GosubNode, NodeType, TextDataType};

use crate::common::document::document::Document as PipelineDocument;
use crate::common::document::node::{AttrMap, NodeId as PipelineNodeId};
use crate::common::document::style::{
    Color, Display, FontWeight, StyleProperty, StylePropertyList, StyleValue, TextAlign, TextWrap, Unit,
};

// ─── CSS cascade → StylePropertyList ─────────────────────────────────────────

/// Convert a gosub `CssPropertyMap` (full cascade output) into a pipeline `StylePropertyList`.
fn css_property_map_to_style_list<S>(prop_map: &S::PropertyMap) -> StylePropertyList
where
    S: CssSystem,
{
    let mut list = StylePropertyList::new();

    for (name, prop) in prop_map.iter() {
        match name {
            // ── Color properties ────────────────────────────────────────────
            "color" => {
                if let Some((r, g, b, a)) = prop.as_color() {
                    list.set_property(StyleProperty::Color, StyleValue::Color(css_color(r, g, b, a)));
                }
            }
            "background-color" => {
                if let Some((r, g, b, a)) = prop.as_color() {
                    list.set_property(StyleProperty::BackgroundColor, StyleValue::Color(css_color(r, g, b, a)));
                }
            }
            "border-top-color" => {
                if let Some((r, g, b, a)) = prop.as_color() {
                    list.set_property(StyleProperty::BorderTopColor, StyleValue::Color(css_color(r, g, b, a)));
                }
            }
            "border-right-color" => {
                if let Some((r, g, b, a)) = prop.as_color() {
                    list.set_property(StyleProperty::BorderRightColor, StyleValue::Color(css_color(r, g, b, a)));
                }
            }
            "border-bottom-color" => {
                if let Some((r, g, b, a)) = prop.as_color() {
                    list.set_property(StyleProperty::BorderBottomColor, StyleValue::Color(css_color(r, g, b, a)));
                }
            }
            "border-left-color" => {
                if let Some((r, g, b, a)) = prop.as_color() {
                    list.set_property(StyleProperty::BorderLeftColor, StyleValue::Color(css_color(r, g, b, a)));
                }
            }

            // ── Display ─────────────────────────────────────────────────────
            "display" => {
                if let Some(s) = prop.as_string() {
                    let d = match s {
                        // "inline" is the CSS *initial* value — skip it so elements with no
                        // explicit display rule fall through to Taffy's default (Block), which
                        // is what the browser UA stylesheet intends for block-level elements.
                        // Inline is only applied when a stylesheet rule explicitly says so.
                        "inline"             => None,
                        "block"              => Some(Display::Block),
                        "inline-block"       => Some(Display::InlineBlock),
                        "none"               => Some(Display::None),
                        "flex"               => Some(Display::Flex),
                        "table"              => Some(Display::Table),
                        "table-caption"      => Some(Display::TableCaption),
                        "table-cell"         => Some(Display::TableCell),
                        "table-footer-group" => Some(Display::TableFooterGroup),
                        "table-header-group" => Some(Display::TableHeaderGroup),
                        "table-row"          => Some(Display::TableRow),
                        "table-row-group"    => Some(Display::TableRowGroup),
                        _                    => None,
                    };
                    if let Some(d) = d {
                        list.set_property(StyleProperty::Display, StyleValue::Display(d));
                    }
                }
            }

            // ── Font ─────────────────────────────────────────────────────────
            "font-size" => {
                if let Some(sv) = css_unit_value::<S>(prop) {
                    list.set_property(StyleProperty::FontSize, sv);
                }
            }
            "font-family" => {
                if let Some(s) = prop.as_string() {
                    list.set_property(StyleProperty::FontFamily,
                        StyleValue::Keyword(s.trim_matches('"').trim_matches('\'').to_string()));
                }
            }
            "font-weight" => {
                let fw = if let Some(s) = prop.as_string() {
                    match s {
                        "normal"  => Some(FontWeight::Normal),
                        "bold"    => Some(FontWeight::Bold),
                        "bolder"  => Some(FontWeight::Bolder),
                        "lighter" => Some(FontWeight::Lighter),
                        _         => None,
                    }
                } else {
                    prop.as_number().map(FontWeight::Number)
                };
                if let Some(fw) = fw {
                    list.set_property(StyleProperty::FontWeight, StyleValue::FontWeight(fw));
                }
            }

            // ── Box model — dimensions ────────────────────────────────────────
            "width"      => { set_unit::<S>(&mut list, StyleProperty::Width,     prop); }
            "height"     => { set_unit::<S>(&mut list, StyleProperty::Height,    prop); }
            "min-width"  => { set_unit::<S>(&mut list, StyleProperty::MinWidth,  prop); }
            "min-height" => { set_unit::<S>(&mut list, StyleProperty::MinHeight, prop); }
            "max-width"  => { set_unit::<S>(&mut list, StyleProperty::MaxWidth,  prop); }
            "max-height" => { set_unit::<S>(&mut list, StyleProperty::MaxHeight, prop); }

            // ── Box model — spacing ───────────────────────────────────────────
            "margin-top"    => { set_unit::<S>(&mut list, StyleProperty::MarginTop,    prop); }
            "margin-right"  => { set_unit::<S>(&mut list, StyleProperty::MarginRight,  prop); }
            "margin-bottom" => { set_unit::<S>(&mut list, StyleProperty::MarginBottom, prop); }
            "margin-left"   => { set_unit::<S>(&mut list, StyleProperty::MarginLeft,   prop); }
            "padding-top"    => { set_unit::<S>(&mut list, StyleProperty::PaddingTop,    prop); }
            "padding-right"  => { set_unit::<S>(&mut list, StyleProperty::PaddingRight,  prop); }
            "padding-bottom" => { set_unit::<S>(&mut list, StyleProperty::PaddingBottom, prop); }
            "padding-left"   => { set_unit::<S>(&mut list, StyleProperty::PaddingLeft,   prop); }

            // ── Border ───────────────────────────────────────────────────────
            "border-top-width"    => { set_unit::<S>(&mut list, StyleProperty::BorderTopWidth,    prop); }
            "border-right-width"  => { set_unit::<S>(&mut list, StyleProperty::BorderRightWidth,  prop); }
            "border-bottom-width" => { set_unit::<S>(&mut list, StyleProperty::BorderBottomWidth, prop); }
            "border-left-width"   => { set_unit::<S>(&mut list, StyleProperty::BorderLeftWidth,   prop); }
            "border-top-left-radius"     => { set_unit::<S>(&mut list, StyleProperty::BorderTopLeftRadius,     prop); }
            "border-top-right-radius"    => { set_unit::<S>(&mut list, StyleProperty::BorderTopRightRadius,    prop); }
            "border-bottom-left-radius"  => { set_unit::<S>(&mut list, StyleProperty::BorderBottomLeftRadius,  prop); }
            "border-bottom-right-radius" => { set_unit::<S>(&mut list, StyleProperty::BorderBottomRightRadius, prop); }

            // ── Flexbox ───────────────────────────────────────────────────────
            "flex-basis"  => { set_unit::<S>(&mut list, StyleProperty::FlexBasis,  prop); }
            "flex-grow"   => { if let Some(n) = prop.as_number() { list.set_property(StyleProperty::FlexGrow,   StyleValue::Number(n)); } }
            "flex-shrink" => { if let Some(n) = prop.as_number() { list.set_property(StyleProperty::FlexShrink, StyleValue::Number(n)); } }
            "flex-direction" => { set_keyword::<S>(&mut list, StyleProperty::FlexDirection, prop); }
            "flex-wrap"      => { set_keyword::<S>(&mut list, StyleProperty::FlexWrap,      prop); }
            "align-items"    => { set_keyword::<S>(&mut list, StyleProperty::AlignItems,    prop); }
            "align-self"     => { set_keyword::<S>(&mut list, StyleProperty::AlignSelf,     prop); }
            "align-content"  => { set_keyword::<S>(&mut list, StyleProperty::AlignContent,  prop); }
            "justify-items"   => { set_keyword::<S>(&mut list, StyleProperty::JustifyItems,   prop); }
            "justify-self"    => { set_keyword::<S>(&mut list, StyleProperty::JustifySelf,    prop); }
            "justify-content" => { set_keyword::<S>(&mut list, StyleProperty::JustifyContent, prop); }
            "gap" => { set_unit::<S>(&mut list, StyleProperty::Gap, prop); }

            // ── Grid ──────────────────────────────────────────────────────────
            "grid-auto-flow"       => { set_keyword::<S>(&mut list, StyleProperty::GridAutoFlow,       prop); }
            "grid-row"             => { set_keyword::<S>(&mut list, StyleProperty::GridRow,             prop); }
            "grid-column"          => { set_keyword::<S>(&mut list, StyleProperty::GridColumn,          prop); }
            "grid-template-rows"   => { set_keyword::<S>(&mut list, StyleProperty::GridTemplateRows,    prop); }
            "grid-template-columns"=> { set_keyword::<S>(&mut list, StyleProperty::GridTemplateColumns, prop); }
            "grid-auto-rows"       => { set_keyword::<S>(&mut list, StyleProperty::GridAutoRows,        prop); }
            "grid-auto-columns"    => { set_keyword::<S>(&mut list, StyleProperty::GridAutoColumns,     prop); }

            // ── Text ──────────────────────────────────────────────────────────
            "text-align" => {
                if let Some(s) = prop.as_string() {
                    let ta = match s {
                        "left"          => Some(TextAlign::Left),
                        "right"         => Some(TextAlign::Right),
                        "center"        => Some(TextAlign::Center),
                        "justify"       => Some(TextAlign::Justify),
                        "start"         => Some(TextAlign::Start),
                        "end"           => Some(TextAlign::End),
                        "match-parent"  => Some(TextAlign::MatchParent),
                        "initial"       => Some(TextAlign::Initial),
                        "inherit"       => Some(TextAlign::Inherit),
                        _               => None,
                    };
                    if let Some(ta) = ta {
                        list.set_property(StyleProperty::TextAlign, StyleValue::TextAlign(ta));
                    }
                }
            }
            "text-wrap" | "white-space" => {
                if let Some(s) = prop.as_string() {
                    let tw = match s {
                        "wrap"         => Some(TextWrap::Wrap),
                        "nowrap" | "no-wrap" => Some(TextWrap::NoWrap),
                        "balance"      => Some(TextWrap::Balance),
                        "pretty"       => Some(TextWrap::Pretty),
                        "stable"       => Some(TextWrap::Stable),
                        "initial"      => Some(TextWrap::Initial),
                        "inherit"      => Some(TextWrap::Inherit),
                        _              => None,
                    };
                    if let Some(tw) = tw {
                        list.set_property(StyleProperty::TextWrap, StyleValue::TextWrap(tw));
                    }
                }
            }
            "line-height" => { set_unit::<S>(&mut list, StyleProperty::LineHeight, prop); }

            // ── Positioning ───────────────────────────────────────────────────
            "position"       => { set_keyword::<S>(&mut list, StyleProperty::Position,      prop); }
            "inset-block-end"    => { set_unit::<S>(&mut list, StyleProperty::InsetBlockEnd,    prop); }
            "inset-block-start"  => { set_unit::<S>(&mut list, StyleProperty::InsetBlockStart,  prop); }
            "inset-inline-end"   => { set_unit::<S>(&mut list, StyleProperty::InsetInlineEnd,   prop); }
            "inset-inline-start" => { set_unit::<S>(&mut list, StyleProperty::InsetInlineStart, prop); }

            // ── Overflow / misc ───────────────────────────────────────────────
            "overflow-x"      => { set_keyword::<S>(&mut list, StyleProperty::OverflowX,      prop); }
            "overflow-y"      => { set_keyword::<S>(&mut list, StyleProperty::OverflowY,      prop); }
            "box-sizing"      => { set_keyword::<S>(&mut list, StyleProperty::BoxSizing,      prop); }
            "scrollbar-width" => { set_keyword::<S>(&mut list, StyleProperty::ScrollbarWidth, prop); }
            "aspect-ratio" => {
                if let Some(n) = prop.as_number() {
                    list.set_property(StyleProperty::AspectRatio, StyleValue::Number(n));
                }
            }

            _ => {}
        }
    }

    list
}

/// Build a `Color` from f32 r/g/b/a components in the range `[0.0, 1.0]`.
#[inline]
fn css_color(r: f32, g: f32, b: f32, a: f32) -> Color {
    let r8 = (r * 255.0) as u8;
    let g8 = (g * 255.0) as u8;
    let b8 = (b * 255.0) as u8;
    if a < 1.0 { Color::Rgba(r8, g8, b8, a) } else { Color::Rgb(r8, g8, b8) }
}

/// Extract a `StyleValue` from a CSS property that carries a length/percentage.
fn css_unit_value<S: CssSystem>(prop: &S::Property) -> Option<StyleValue> {
    if let Some((v, unit_str)) = prop.as_unit() {
        let unit = match unit_str {
            "em"  => Unit::Em,
            "rem" => Unit::Rem,
            "%"   => return Some(StyleValue::Percentage(v)),
            _     => Unit::Px,
        };
        return Some(StyleValue::Unit(v, unit));
    }
    if let Some(pct) = prop.as_percentage() {
        return Some(StyleValue::Percentage(pct));
    }
    // Fallback: convert whatever unit the CSS engine has to px
    let px = prop.unit_to_px();
    if px != 0.0 {
        return Some(StyleValue::Unit(px, Unit::Px));
    }
    None
}

#[inline]
fn set_unit<S: CssSystem>(list: &mut StylePropertyList, prop_key: StyleProperty, prop: &S::Property) {
    if let Some(sv) = css_unit_value::<S>(prop) {
        list.set_property(prop_key, sv);
    }
}

#[inline]
fn set_keyword<S: CssSystem>(list: &mut StylePropertyList, prop_key: StyleProperty, prop: &S::Property) {
    if let Some(s) = prop.as_string() {
        list.set_property(prop_key, StyleValue::Keyword(s.to_string()));
    }
}

// ─── CSS inline-style parser ─────────────────────────────────────────────────

pub fn parse_inline_style(style_attr: &str) -> StylePropertyList {
    let mut list = StylePropertyList::new();
    if style_attr.trim().is_empty() {
        return list;
    }

    for decl in style_attr.split(';') {
        let decl = decl.trim();
        if decl.is_empty() {
            continue;
        }
        let Some((prop, val)) = decl.split_once(':') else {
            continue;
        };
        let prop = prop.trim().to_ascii_lowercase();
        let val = val.trim();

        match prop.as_str() {
            "display" => {
                let display = match val {
                    "block"        => Some(Display::Block),
                    "inline"       => Some(Display::Inline),
                    "inline-block" => Some(Display::InlineBlock),
                    "none"         => Some(Display::None),
                    "flex"         => Some(Display::Flex),
                    _              => None,
                };
                if let Some(d) = display {
                    list.set_property(StyleProperty::Display, StyleValue::Display(d));
                }
            }
            "color" => {
                if let Some(c) = parse_color(val) {
                    list.set_property(StyleProperty::Color, StyleValue::Color(c));
                }
            }
            "background-color" => {
                if let Some(c) = parse_color(val) {
                    list.set_property(StyleProperty::BackgroundColor, StyleValue::Color(c));
                }
            }
            "font-size" => {
                if let Some(sv) = parse_unit_value(val) {
                    list.set_property(StyleProperty::FontSize, sv);
                }
            }
            "font-family" => {
                list.set_property(
                    StyleProperty::FontFamily,
                    StyleValue::Keyword(
                        val.trim_matches('"').trim_matches('\'').to_string(),
                    ),
                );
            }
            "font-weight" => {
                let fw = match val {
                    "normal"  => Some(FontWeight::Normal),
                    "bold"    => Some(FontWeight::Bold),
                    "bolder"  => Some(FontWeight::Bolder),
                    "lighter" => Some(FontWeight::Lighter),
                    n => n.parse::<f32>().ok().map(FontWeight::Number),
                };
                if let Some(fw) = fw {
                    list.set_property(StyleProperty::FontWeight, StyleValue::FontWeight(fw));
                }
            }
            "width"  => { set_unit_str(&mut list, StyleProperty::Width,  val); }
            "height" => { set_unit_str(&mut list, StyleProperty::Height, val); }
            "margin-top"    | "margin"  => { set_unit_str(&mut list, StyleProperty::MarginTop,    val); }
            "margin-right"              => { set_unit_str(&mut list, StyleProperty::MarginRight,   val); }
            "margin-bottom"             => { set_unit_str(&mut list, StyleProperty::MarginBottom,  val); }
            "margin-left"               => { set_unit_str(&mut list, StyleProperty::MarginLeft,    val); }
            "padding-top"   | "padding" => { set_unit_str(&mut list, StyleProperty::PaddingTop,   val); }
            "padding-right"             => { set_unit_str(&mut list, StyleProperty::PaddingRight,  val); }
            "padding-bottom"            => { set_unit_str(&mut list, StyleProperty::PaddingBottom, val); }
            "padding-left"              => { set_unit_str(&mut list, StyleProperty::PaddingLeft,   val); }
            "border-top-width"    => { set_unit_str(&mut list, StyleProperty::BorderTopWidth,    val); }
            "border-right-width"  => { set_unit_str(&mut list, StyleProperty::BorderRightWidth,  val); }
            "border-bottom-width" => { set_unit_str(&mut list, StyleProperty::BorderBottomWidth, val); }
            "border-left-width"   => { set_unit_str(&mut list, StyleProperty::BorderLeftWidth,   val); }
            _ => {}
        }
    }

    list
}

fn set_unit_str(list: &mut StylePropertyList, prop: StyleProperty, val: &str) {
    if let Some(sv) = parse_unit_value(val) {
        list.set_property(prop, sv);
    }
}

fn parse_unit_value(val: &str) -> Option<StyleValue> {
    if let Some(n) = val.strip_suffix("px") {
        return n.trim().parse::<f32>().ok().map(|v| StyleValue::Unit(v, Unit::Px));
    }
    if let Some(n) = val.strip_suffix("rem") {
        return n.trim().parse::<f32>().ok().map(|v| StyleValue::Unit(v, Unit::Rem));
    }
    if let Some(n) = val.strip_suffix("em") {
        return n.trim().parse::<f32>().ok().map(|v| StyleValue::Unit(v, Unit::Em));
    }
    if let Some(n) = val.strip_suffix('%') {
        return n.trim().parse::<f32>().ok().map(StyleValue::Percentage);
    }
    val.trim().parse::<f32>().ok().map(StyleValue::Number)
}

fn parse_color(val: &str) -> Option<Color> {
    if let Ok(c) = csscolorparser::Color::from_html(val) {
        let r = (c.r * 255.0) as u8;
        let g = (c.g * 255.0) as u8;
        let b = (c.b * 255.0) as u8;
        if c.a < 1.0 {
            return Some(Color::Rgba(r, g, b, c.a as f32));
        }
        return Some(Color::Rgb(r, g, b));
    }
    Some(Color::Named(val.to_string()))
}

// ─── DOM bridge ──────────────────────────────────────────────────────────────

/// Walk the gosub Document tree and produce a [`PipelineDocument`].
///
/// Node IDs are assigned sequentially by the pipeline document; they do **not**
/// map to the gosub node IDs. The tree structure is preserved.
pub fn build_pipeline_document<C>(gosub_doc: &C::Document, base_url: &str) -> Arc<PipelineDocument>
where
    C: HasDocument,
    C::Document: GosubDoc<C>,
{
    let mut pipeline_doc = PipelineDocument::new(base_url);
    let root = gosub_doc.get_root();
    build_node::<C>(gosub_doc, root, None, &mut pipeline_doc);
    Arc::new(pipeline_doc)
}

fn build_node<C>(
    gosub_doc: &C::Document,
    node: &<C::Document as GosubDoc<C>>::Node,
    parent_id: Option<PipelineNodeId>,
    pipeline_doc: &mut PipelineDocument,
) where
    C: HasDocument,
    C::Document: GosubDoc<C>,
    <<C::Document as GosubDoc<C>>::Node as GosubNode<C>>::ElementData: ElementDataType<C>,
    <<C::Document as GosubDoc<C>>::Node as GosubNode<C>>::TextData:    TextDataType,
    <<C::Document as GosubDoc<C>>::Node as GosubNode<C>>::CommentData: CommentDataType,
{
    let poc_id: Option<PipelineNodeId> = match node.type_of() {
        NodeType::ElementNode => {
            let Some(elem) = node.get_element_data() else { return; };

            let tag = elem.name().to_ascii_lowercase();
            let raw_attrs = elem.attributes();

            let mut attr_map = AttrMap::new();
            for (k, v) in raw_attrs {
                attr_map.set(k, v);
            }

            // Step 1: full CSS cascade (UA stylesheet + author stylesheets)
            let mut styles = if let Some(prop_map) =
                <C::CssSystem as CssSystem>::properties_from_node::<C>(
                    node,
                    gosub_doc.stylesheets(),
                    gosub_doc,
                    node.id(),
                )
            {
                css_property_map_to_style_list::<C::CssSystem>(&prop_map)
            } else {
                StylePropertyList::new()
            };

            // Step 2: overlay inline styles — they have the highest specificity
            // and the cascade should already include them, but we apply them again
            // here as a belt-and-suspenders guarantee.
            if let Some(inline) = raw_attrs.get("style") {
                for (prop, val) in parse_inline_style(inline).properties {
                    styles.set_property(prop, val);
                }
            }

            let self_closing = matches!(
                tag.as_str(),
                "area" | "base" | "br" | "col" | "embed" | "hr" | "img"
                    | "input" | "link" | "meta" | "param" | "source"
                    | "track" | "wbr"
            );

            Some(pipeline_doc.new_element(
                parent_id, &tag, Some(attr_map), self_closing, Some(styles),
            ))
        }
        NodeType::TextNode => {
            let Some(text_data) = node.get_text_data() else { return; };
            let text = text_data.value();
            if text.trim().is_empty() {
                return;
            }
            Some(pipeline_doc.new_text(parent_id, text, None))
        }
        NodeType::CommentNode => {
            let Some(comment_data) = node.get_comment_data() else { return; };
            Some(pipeline_doc.new_comment(parent_id, comment_data.value()))
        }
        NodeType::DocumentNode => {
            // Root document wrapper — no visual node; recurse directly
            for &child_id in node.children() {
                if let Some(child) = gosub_doc.node_by_id(child_id) {
                    build_node::<C>(gosub_doc, child, parent_id, pipeline_doc);
                }
            }
            return;
        }
        NodeType::DocTypeNode => return,
    };

    if let Some(id) = poc_id {
        if pipeline_doc.root_id.is_none() {
            pipeline_doc.set_root(id);
        }
        for &child_id in node.children() {
            if let Some(child) = gosub_doc.node_by_id(child_id) {
                build_node::<C>(gosub_doc, child, Some(id), pipeline_doc);
            }
        }
    }
}
