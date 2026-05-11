use crate::common::document::document::Document;
use crate::common::document::node::{AttrMap, ElementData, Node, NodeType};
use crate::common::document::style::{
    BorderStyle, Color, Display, FontWeight, StyleProperty, StylePropertyList, StyleValue, TextAlign, TextWrap, Unit,
};
use gosub_interface::config::HasDocument;
use gosub_interface::css3::{CssProperty, CssPropertyMap, CssSystem};
use gosub_interface::document::Document as _;
use gosub_interface::node::NodeType as GosubNodeType;
use gosub_shared::node::NodeId;

#[derive(Debug, Clone, PartialEq)]
pub enum PipelineNodeKind {
    Text,
    Comment,
    Element,
}

pub trait PipelineDocument: Send + Sync {
    fn root(&self) -> Option<NodeId>;
    fn children(&self, id: NodeId) -> Vec<NodeId>;
    fn node_kind(&self, id: NodeId) -> PipelineNodeKind;
    fn tag_name(&self, id: NodeId) -> Option<String>;
    fn is_display_none(&self, id: NodeId) -> bool;
    fn parent(&self, id: NodeId) -> Option<NodeId>;
    fn get_style(&self, id: NodeId, prop: StyleProperty) -> Option<StyleValue>;
    fn get_style_f32(&self, id: NodeId, prop: StyleProperty) -> f32;
    fn html_node_id(&self) -> Option<NodeId>;
    fn body_node_id(&self) -> Option<NodeId>;
    fn base_url(&self) -> String;
    fn inner_html(&self, id: NodeId) -> String;
    fn get_node_by_id(&self, _id: NodeId) -> Option<Node> {
        None
    }
}

// impl for poc's own Document (field-based, self-contained)
impl PipelineDocument for Document {
    fn root(&self) -> Option<NodeId> {
        self.root_id
    }

    fn children(&self, id: NodeId) -> Vec<NodeId> {
        self.arena.get(&id).map(|n| n.children.clone()).unwrap_or_default()
    }

    fn node_kind(&self, id: NodeId) -> PipelineNodeKind {
        match self.arena.get(&id) {
            Some(node) => match &node.node_type {
                NodeType::Text(..) => PipelineNodeKind::Text,
                NodeType::Comment(..) => PipelineNodeKind::Comment,
                NodeType::Element(..) => PipelineNodeKind::Element,
            },
            None => {
                log::warn!("node_kind: node {:?} not found, defaulting to Element", id);
                PipelineNodeKind::Element
            }
        }
    }

    fn tag_name(&self, id: NodeId) -> Option<String> {
        self.arena.get(&id).and_then(|node| match &node.node_type {
            NodeType::Element(data) => Some(data.tag_name.clone()),
            _ => None,
        })
    }

    fn is_display_none(&self, id: NodeId) -> bool {
        self.arena
            .get(&id)
            .map(|node| match &node.node_type {
                NodeType::Element(data) => matches!(
                    data.get_style(StyleProperty::Display),
                    Some(StyleValue::Display(d)) if *d == Display::None
                ),
                _ => false,
            })
            .unwrap_or(false)
    }

    fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.arena.get(&id).and_then(|n| n.parent_id)
    }

    fn get_style(&self, id: NodeId, prop: StyleProperty) -> Option<StyleValue> {
        self.arena.get(&id).and_then(|node| match &node.node_type {
            NodeType::Element(data) => data.get_style(prop).cloned(),
            _ => None,
        })
    }

    fn get_style_f32(&self, id: NodeId, prop: StyleProperty) -> f32 {
        match self.get_style(id, prop) {
            Some(StyleValue::Unit(px, _)) => px,
            Some(StyleValue::Number(px)) => px,
            _ => 0.0,
        }
    }

    fn html_node_id(&self) -> Option<NodeId> {
        self.html_node_id
    }

    fn body_node_id(&self) -> Option<NodeId> {
        self.body_node_id
    }

    fn base_url(&self) -> String {
        self.base_url.clone()
    }

    fn get_node_by_id(&self, id: NodeId) -> Option<Node> {
        self.arena.get(&id).cloned()
    }

    fn inner_html(&self, id: NodeId) -> String {
        self.inner_html(id)
    }
}

/// Adapter that wraps any `gosub_interface::document::Document<C>` and exposes it
/// as a `PipelineDocument`.
pub struct GosubDocumentAdapter<C>
where
    C: HasDocument,
{
    pub doc: C::Document,
}

impl<C> GosubDocumentAdapter<C>
where
    C: HasDocument,
{
    pub fn new(doc: C::Document) -> Self {
        Self { doc }
    }

    fn compute_styles(&self, id: NodeId) -> StylePropertyList {
        let sheets = self.doc.stylesheets();
        let Some(prop_map) = C::CssSystem::properties_from_node::<C>(&self.doc, id, sheets) else {
            return StylePropertyList::new();
        };
        build_style_property_list::<C::CssSystem>(&prop_map)
    }

    fn find_child_by_tag(&self, parent: NodeId, tag: &str) -> Option<NodeId> {
        for &child in self.doc.children(parent) {
            if self.doc.tag_name(child).map_or(false, |t| t.eq_ignore_ascii_case(tag)) {
                return Some(child);
            }
        }
        None
    }
}

impl<C> PipelineDocument for GosubDocumentAdapter<C>
where
    C: HasDocument + Send + Sync + 'static,
    C::Document: Send + Sync,
{
    fn root(&self) -> Option<NodeId> {
        Some(self.doc.root())
    }

    fn children(&self, id: NodeId) -> Vec<NodeId> {
        self.doc.children(id).to_vec()
    }

    fn node_kind(&self, id: NodeId) -> PipelineNodeKind {
        match self.doc.node_type(id) {
            GosubNodeType::TextNode => PipelineNodeKind::Text,
            GosubNodeType::CommentNode => PipelineNodeKind::Comment,
            GosubNodeType::ElementNode => PipelineNodeKind::Element,
            _ => PipelineNodeKind::Comment,
        }
    }

    fn tag_name(&self, id: NodeId) -> Option<String> {
        self.doc.tag_name(id).map(|s| s.to_string())
    }

    fn is_display_none(&self, id: NodeId) -> bool {
        matches!(
            self.get_style(id, StyleProperty::Display),
            Some(StyleValue::Display(Display::None))
        )
    }

    fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.doc.parent(id)
    }

    fn get_style(&self, id: NodeId, prop: StyleProperty) -> Option<StyleValue> {
        self.compute_styles(id).properties.remove(&prop)
    }

    fn get_style_f32(&self, id: NodeId, prop: StyleProperty) -> f32 {
        match self.get_style(id, prop) {
            Some(StyleValue::Unit(px, _)) => px,
            Some(StyleValue::Number(px)) => px,
            _ => 0.0,
        }
    }

    fn html_node_id(&self) -> Option<NodeId> {
        let root = self.doc.root();
        self.find_child_by_tag(root, "html")
    }

    fn body_node_id(&self) -> Option<NodeId> {
        let html = self.html_node_id()?;
        self.find_child_by_tag(html, "body")
    }

    fn base_url(&self) -> String {
        self.doc.url().map(|u| u.to_string()).unwrap_or_default()
    }

    fn inner_html(&self, id: NodeId) -> String {
        self.doc.write_from_node(id)
    }

    fn get_node_by_id(&self, id: NodeId) -> Option<Node> {
        let parent_id = self.doc.parent(id);
        let children = self.doc.children(id).to_vec();

        let node_type = match self.doc.node_type(id) {
            GosubNodeType::TextNode => {
                let text = self.doc.text_value(id).unwrap_or("").to_string();
                // Text nodes inherit styles from their parent element
                let style = parent_id.map(|pid| self.compute_styles(pid)).unwrap_or_default();
                NodeType::Text(text, style)
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
                let styles = self.compute_styles(id);
                let element_data = ElementData::new(tag_name, Some(attr_map), false, Some(styles));
                NodeType::Element(element_data)
            }
            _ => return None,
        };

        Some(Node { node_id: id, parent_id, children, node_type })
    }
}

fn str_to_unit(s: &str) -> Unit {
    match s {
        "em" => Unit::Em,
        "rem" => Unit::Rem,
        "%" => Unit::Percent,
        _ => Unit::Px,
    }
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

/// Converts a `CssPropertyMap` into our internal `StylePropertyList`.
fn build_style_property_list<S: CssSystem>(prop_map: &S::PropertyMap) -> StylePropertyList {
    let mut list = StylePropertyList::new();

    // --- Unit-based properties ---
    let unit_props: &[(&str, StyleProperty)] = &[
        ("font-size", StyleProperty::FontSize),
        ("width", StyleProperty::Width),
        ("height", StyleProperty::Height),
        ("margin-top", StyleProperty::MarginTop),
        ("margin-right", StyleProperty::MarginRight),
        ("margin-bottom", StyleProperty::MarginBottom),
        ("margin-left", StyleProperty::MarginLeft),
        ("padding-top", StyleProperty::PaddingTop),
        ("padding-right", StyleProperty::PaddingRight),
        ("padding-bottom", StyleProperty::PaddingBottom),
        ("padding-left", StyleProperty::PaddingLeft),
        ("border-top-width", StyleProperty::BorderTopWidth),
        ("border-right-width", StyleProperty::BorderRightWidth),
        ("border-bottom-width", StyleProperty::BorderBottomWidth),
        ("border-left-width", StyleProperty::BorderLeftWidth),
        ("min-width", StyleProperty::MinWidth),
        ("min-height", StyleProperty::MinHeight),
        ("max-width", StyleProperty::MaxWidth),
        ("max-height", StyleProperty::MaxHeight),
        ("gap", StyleProperty::Gap),
        ("line-height", StyleProperty::LineHeight),
        ("border-top-left-radius", StyleProperty::BorderTopLeftRadius),
        ("border-top-right-radius", StyleProperty::BorderTopRightRadius),
        ("border-bottom-left-radius", StyleProperty::BorderBottomLeftRadius),
        ("border-bottom-right-radius", StyleProperty::BorderBottomRightRadius),
        ("flex-basis", StyleProperty::FlexBasis),
        ("inset-block-start", StyleProperty::InsetBlockStart),
        ("inset-block-end", StyleProperty::InsetBlockEnd),
        ("inset-inline-start", StyleProperty::InsetInlineStart),
        ("inset-inline-end", StyleProperty::InsetInlineEnd),
    ];
    for (css_name, prop) in unit_props {
        if let Some(p) = prop_map.get(css_name) {
            if let Some((val, unit_str)) = p.as_unit() {
                list.set_property(prop.clone(), StyleValue::Unit(val, str_to_unit(unit_str)));
            } else if let Some(val) = p.as_number() {
                list.set_property(prop.clone(), StyleValue::Unit(val, Unit::Px));
            } else if let Some(s) = p.as_string() {
                list.set_property(prop.clone(), StyleValue::Keyword(s.to_string()));
            }
        }
    }

    // --- Color properties ---
    let color_props: &[(&str, StyleProperty)] = &[
        ("color", StyleProperty::Color),
        ("background-color", StyleProperty::BackgroundColor),
        ("border-top-color", StyleProperty::BorderTopColor),
        ("border-right-color", StyleProperty::BorderRightColor),
        ("border-bottom-color", StyleProperty::BorderBottomColor),
        ("border-left-color", StyleProperty::BorderLeftColor),
    ];
    for (css_name, prop) in color_props {
        if let Some(p) = prop_map.get(css_name) {
            if let Some((r, g, b, a)) = p.as_color() {
                list.set_property(
                    prop.clone(),
                    StyleValue::Color(Color::Rgba(r as u8, g as u8, b as u8, a / 255.0)),
                );
            }
        }
    }

    // --- Display ---
    if let Some(p) = prop_map.get("display") {
        if let Some(s) = p.as_string() {
            let display = match s {
                "block" => Display::Block,
                "inline" => Display::Inline,
                "inline-block" => Display::InlineBlock,
                "none" => Display::None,
                "flex" => Display::Flex,
                "table" => Display::Table,
                "table-caption" => Display::TableCaption,
                "table-cell" => Display::TableCell,
                "table-footer-group" => Display::TableFooterGroup,
                "table-header-group" => Display::TableHeaderGroup,
                "table-row" => Display::TableRow,
                "table-row-group" => Display::TableRowGroup,
                _ => Display::Block,
            };
            list.set_property(StyleProperty::Display, StyleValue::Display(display));
        }
    }

    // --- FontWeight ---
    if let Some(p) = prop_map.get("font-weight") {
        let fw = if let Some(n) = p.as_number() {
            FontWeight::Number(n)
        } else if let Some(s) = p.as_string() {
            match s {
                "bold" => FontWeight::Bold,
                "bolder" => FontWeight::Bolder,
                "lighter" => FontWeight::Lighter,
                _ => FontWeight::Normal,
            }
        } else {
            FontWeight::Normal
        };
        list.set_property(StyleProperty::FontWeight, StyleValue::FontWeight(fw));
    }

    // --- TextAlign ---
    if let Some(p) = prop_map.get("text-align") {
        if let Some(s) = p.as_string() {
            let ta = match s {
                "left" => TextAlign::Left,
                "right" => TextAlign::Right,
                "center" => TextAlign::Center,
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
            list.set_property(StyleProperty::TextAlign, StyleValue::TextAlign(ta));
        }
    }

    // --- TextWrap ---
    if let Some(p) = prop_map.get("text-wrap") {
        if let Some(s) = p.as_string() {
            let tw = match s {
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
            list.set_property(StyleProperty::TextWrap, StyleValue::TextWrap(tw));
        }
    }

    // --- Border styles ---
    let border_style_props: &[(&str, StyleProperty)] = &[
        ("border-top-style", StyleProperty::BorderTopStyle),
        ("border-right-style", StyleProperty::BorderRightStyle),
        ("border-bottom-style", StyleProperty::BorderBottomStyle),
        ("border-left-style", StyleProperty::BorderLeftStyle),
    ];
    for (css_name, prop) in border_style_props {
        if let Some(p) = prop_map.get(css_name) {
            if let Some(s) = p.as_string() {
                list.set_property(prop.clone(), StyleValue::BorderStyle(str_to_border_style(s)));
            }
        }
    }

    // --- Numeric properties ---
    let num_props: &[(&str, StyleProperty)] = &[
        ("flex-grow", StyleProperty::FlexGrow),
        ("flex-shrink", StyleProperty::FlexShrink),
        ("aspect-ratio", StyleProperty::AspectRatio),
        ("scrollbar-width", StyleProperty::ScrollbarWidth),
    ];
    for (css_name, prop) in num_props {
        if let Some(p) = prop_map.get(css_name) {
            if let Some(n) = p.as_number() {
                list.set_property(prop.clone(), StyleValue::Number(n));
            }
        }
    }

    // --- Keyword properties ---
    let kw_props: &[(&str, StyleProperty)] = &[
        ("font-family", StyleProperty::FontFamily),
        ("position", StyleProperty::Position),
        ("flex-direction", StyleProperty::FlexDirection),
        ("flex-wrap", StyleProperty::FlexWrap),
        ("align-items", StyleProperty::AlignItems),
        ("align-self", StyleProperty::AlignSelf),
        ("align-content", StyleProperty::AlignContent),
        ("justify-items", StyleProperty::JustifyItems),
        ("justify-self", StyleProperty::JustifySelf),
        ("justify-content", StyleProperty::JustifyContent),
        ("overflow-x", StyleProperty::OverflowX),
        ("overflow-y", StyleProperty::OverflowY),
        ("box-sizing", StyleProperty::BoxSizing),
        ("grid-auto-flow", StyleProperty::GridAutoFlow),
        ("grid-row", StyleProperty::GridRow),
        ("grid-column", StyleProperty::GridColumn),
        ("grid-template-rows", StyleProperty::GridTemplateRows),
        ("grid-template-columns", StyleProperty::GridTemplateColumns),
        ("grid-auto-rows", StyleProperty::GridAutoRows),
        ("grid-auto-columns", StyleProperty::GridAutoColumns),
    ];
    for (css_name, prop) in kw_props {
        if let Some(p) = prop_map.get(css_name) {
            if let Some(s) = p.as_string() {
                list.set_property(prop.clone(), StyleValue::Keyword(s.to_string()));
            }
        }
    }

    list
}
