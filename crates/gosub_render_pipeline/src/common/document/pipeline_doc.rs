use crate::common::document::document::Document;
use crate::common::document::node::{AttrMap, ElementData, Node, NodeType};
use crate::common::document::style::{
    intern, BorderStyle, Display, FontWeight, NodeStyle, StyleProperty, TextAlign, TextWrap, Unit, Value,
};
use cow_utils::CowUtils;
use gosub_interface::config::HasDocument;
use gosub_interface::css3::{CssProperty, CssPropertyMap, CssSystem, CssValue};
use gosub_interface::document::Document as _;
use gosub_interface::node::NodeType as GosubNodeType;
use gosub_shared::node::NodeId;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq)]
pub enum PipelineNodeKind {
    Text,
    Comment,
    Element,
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

    /// Discard the computed-style cache so the next `get_own_style` call re-evaluates
    /// CSS selectors (including `:hover`) from scratch.  No-op for backends that do
    /// not cache styles.
    fn clear_style_cache(&self) {}

    /// Discard cached computed styles for specific nodes only. More efficient than
    /// `clear_style_cache` for hover repaints where only a few elements changed.
    fn invalidate_style_for_nodes(&self, _ids: &[NodeId]) {}

    /// Returns the computed value for `prop` on node `id`:
    ///  1. own value if set,
    ///  2. parent's computed value if the property is inherited,
    ///  3. the CSS-spec initial value otherwise.
    fn get_style(&self, id: NodeId, prop: &StyleProperty) -> Value {
        let raw = if let Some(v) = self.get_own_style(id, prop) {
            v
        } else {
            let meta = prop.meta();
            if meta.inherited {
                if let Some(parent) = self.parent(id) {
                    return self.get_style(parent, prop);
                }
            }
            meta.initial_value()
        };

        // Resolve em/rem for font-size. CSS spec: for font-size, em is relative to the
        // *parent's* computed font-size; rem is relative to the root element's (16px default).
        if matches!(prop, StyleProperty::FontSize) {
            match &raw {
                Value::Unit(v, Unit::Em) => {
                    let parent_px = match self.parent(id) {
                        Some(parent) => match self.get_style(parent, &StyleProperty::FontSize) {
                            Value::Unit(px, Unit::Px) => px,
                            _ => 16.0,
                        },
                        None => 16.0,
                    };
                    return Value::Unit(v * parent_px, Unit::Px);
                }
                Value::Unit(v, Unit::Rem) => {
                    return Value::Unit(v * 16.0, Unit::Px);
                }
                _ => {}
            }
        }

        raw
    }

    fn get_style_f32(&self, id: NodeId, prop: &StyleProperty) -> f32 {
        match self.get_style(id, prop) {
            Value::Unit(v, _) => v,
            Value::Number(v) => v,
            _ => 0.0,
        }
    }
}

// ── impl for the simple JSON-based Document ───────────────────────────────────

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
                NodeType::Element(data) => {
                    matches!(
                        data.get_style(&StyleProperty::Display),
                        Some(Value::Display(Display::None))
                    )
                }
                _ => false,
            })
            .unwrap_or(false)
    }

    fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.arena.get(&id).and_then(|n| n.parent_id)
    }

    fn get_own_style(&self, id: NodeId, prop: &StyleProperty) -> Option<Value> {
        self.arena.get(&id).and_then(|node| match &node.node_type {
            NodeType::Element(data) => data.get_style(prop).cloned(),
            _ => None,
        })
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

// ── GosubDocumentAdapter ──────────────────────────────────────────────────────

/// Adapts any `gosub_interface::document::Document<C>` into a `PipelineDocument`.
pub struct GosubDocumentAdapter<C>
where
    C: HasDocument,
{
    pub doc: Arc<C::Document>,
    /// Per-node own-style cache. Populated lazily; valid for one pipeline run.
    style_cache: Mutex<HashMap<NodeId, NodeStyle>>,
}

impl<C> GosubDocumentAdapter<C>
where
    C: HasDocument,
{
    pub fn new(doc: Arc<C::Document>) -> Self {
        Self {
            doc,
            style_cache: Mutex::new(HashMap::new()),
        }
    }

    fn cached_styles(&self, id: NodeId) -> NodeStyle {
        if let Some(cached) = self.style_cache.lock().get(&id) {
            return cached.clone();
        }
        let style = self.compute_styles(id);
        self.style_cache.lock().insert(id, style.clone());
        style
    }

    fn compute_styles(&self, id: NodeId) -> NodeStyle {
        // CSS selectors cannot target text nodes — only elements. Skip the full
        // stylesheet scan for text nodes; they inherit everything from their parent.
        if self.doc.node_type(id) == GosubNodeType::TextNode {
            return NodeStyle::new();
        }
        let sheets = self.doc.stylesheets();
        let Some(mut prop_map) = C::CssSystem::properties_from_node::<C>(&*self.doc, id, sheets) else {
            return NodeStyle::new();
        };
        for (_, prop) in prop_map.iter_mut() {
            prop.compute_value();
        }
        let mut style = build_node_style::<C::CssSystem>(&prop_map);

        // Inline `style` attribute has highest specificity — overlay it last.
        if let Some(attrs) = self.doc.attributes(id) {
            if let Some(style_attr) = attrs.get("style") {
                let inline = crate::common::document::parser::parse_inline_style_attr(style_attr);
                for (prop, val) in inline.iter() {
                    style.set(prop, val.clone());
                }
            }
        }

        style
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
{
    fn root(&self) -> Option<NodeId> {
        self.html_node_id().or_else(|| Some(self.doc.root()))
    }

    fn children(&self, id: NodeId) -> Vec<NodeId> {
        self.doc.children(id).to_vec()
    }

    fn node_kind(&self, id: NodeId) -> PipelineNodeKind {
        match self.doc.node_type(id) {
            GosubNodeType::TextNode => PipelineNodeKind::Text,
            GosubNodeType::CommentNode | GosubNodeType::DocTypeNode => PipelineNodeKind::Comment,
            GosubNodeType::ElementNode => PipelineNodeKind::Element,
            GosubNodeType::DocumentNode => PipelineNodeKind::Element,
        }
    }

    fn tag_name(&self, id: NodeId) -> Option<String> {
        self.doc.tag_name(id).map(|s| s.to_string())
    }

    fn is_display_none(&self, id: NodeId) -> bool {
        matches!(
            self.get_own_style(id, &StyleProperty::Display),
            Some(Value::Display(Display::None))
        )
    }

    fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.doc.parent(id)
    }

    fn get_own_style(&self, id: NodeId, prop: &StyleProperty) -> Option<Value> {
        self.cached_styles(id).get_own(prop).cloned()
    }

    fn clear_style_cache(&self) {
        self.style_cache.lock().clear();
    }

    fn invalidate_style_for_nodes(&self, ids: &[NodeId]) {
        let mut cache = self.style_cache.lock();
        for id in ids {
            cache.remove(id);
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
        self.doc.write_from_node(id)
    }

    fn get_node_by_id(&self, id: NodeId) -> Option<Node> {
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
                let styles = self.cached_styles(id);
                let element_data = ElementData::new(tag_name, Some(attr_map), false, Some(styles));
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

// ── build_node_style — converts CssPropertyMap into NodeStyle ─────────────────

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

fn build_node_style<S: CssSystem>(prop_map: &S::PropertyMap) -> NodeStyle {
    let mut style = NodeStyle::new();

    // --- border-radius shorthand expansion (before individual corners so they can override) ---
    // CSS: 1 value → all corners; 2 → TL+BR / TR+BL; 3 → TL / TR+BL / BR; 4 → TL TR BR BL
    if let Some(br) = prop_map.get("border-radius") {
        let corners: [StyleProperty; 4] = [
            StyleProperty::BorderTopLeftRadius,
            StyleProperty::BorderTopRightRadius,
            StyleProperty::BorderBottomRightRadius,
            StyleProperty::BorderBottomLeftRadius,
        ];

        if br.as_unit().is_some() {
            // Single value: all four corners the same.
            // Use unit_to_px() so that rem/em values are converted — otherwise
            // `border-radius: 0.25rem` would be stored as 0.25 and treated as
            // 0.25 px (invisible) when get_style_f32 ignores the unit.
            let px = br.unit_to_px();
            if px > 0.0 {
                let val = Value::Unit(px, Unit::Px);
                for c in &corners {
                    style.set(c.clone(), val.clone());
                }
            }
        } else if let Some(list) = br.as_list() {
            // Extract numeric values from the list, converting all units to px.
            let vals: Vec<Value> = list
                .iter()
                .filter_map(|cv| {
                    if cv.as_unit().is_some() {
                        let px = cv.unit_to_px();
                        Some(Value::Unit(px, Unit::Px))
                    } else if let Some(v) = cv.as_number() {
                        Some(Value::Unit(v, Unit::Px))
                    } else {
                        cv.as_percentage().map(|v| Value::Unit(v, Unit::Percent))
                    }
                })
                .collect();

            // Apply the CSS border-radius shorthand expansion pattern
            let expanded: [Value; 4] = match vals.len() {
                2 => [vals[0].clone(), vals[1].clone(), vals[0].clone(), vals[1].clone()],
                3 => [vals[0].clone(), vals[1].clone(), vals[2].clone(), vals[1].clone()],
                4.. => [vals[0].clone(), vals[1].clone(), vals[2].clone(), vals[3].clone()],
                _ => {
                    let v = vals.first().cloned().unwrap_or(Value::Unit(0.0, Unit::Px));
                    [v.clone(), v.clone(), v.clone(), v]
                }
            };
            for (c, v) in corners.iter().zip(expanded.iter()) {
                style.set(c.clone(), v.clone());
            }
        }
    }

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
        // Logical margin aliases — used by some frameworks instead of physical sides
        ("margin-block-start", StyleProperty::MarginTop),
        ("margin-block-end", StyleProperty::MarginBottom),
        ("margin-inline-start", StyleProperty::MarginLeft),
        ("margin-inline-end", StyleProperty::MarginRight),
        ("padding-block-start", StyleProperty::PaddingTop),
        ("padding-block-end", StyleProperty::PaddingBottom),
        ("padding-inline-start", StyleProperty::PaddingLeft),
        ("padding-inline-end", StyleProperty::PaddingRight),
        ("min-width", StyleProperty::MinWidth),
        ("min-height", StyleProperty::MinHeight),
        ("max-width", StyleProperty::MaxWidth),
        ("max-height", StyleProperty::MaxHeight),
        ("gap", StyleProperty::Gap),
        ("column-gap", StyleProperty::Gap), // column-gap alone maps to horizontal gap
        ("row-gap", StyleProperty::Gap),    // row-gap alone maps to vertical gap
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
    // Pre-compute the element's font-size so ch/ex units can use it below.
    // unit_to_px() resolves em/rem with 16px base, giving a close enough value.
    let font_size_for_ch = prop_map
        .get("font-size")
        .filter(|fp| fp.as_unit().is_some())
        .map(|fp| fp.unit_to_px())
        .unwrap_or(16.0_f32);

    for (css_name, prop) in unit_props {
        if let Some(p) = prop_map.get(css_name) {
            if p.as_unit().is_some() {
                // Convert em/rem to px now so downstream code (get_style_f32) can
                // treat all Unit values as pixels without knowing the unit.
                // Also handle font-relative units that unit_to_px() ignores:
                //   ch  ≈ 0.45 × font-size  (width of "0" in the current font)
                //   ex  ≈ 0.50 × font-size  (x-height)
                //   ic  ≈ 1.00 × font-size  (CJK ideograph width)
                //   lh  ≈ 1.40 × font-size  (line-height)
                let px = match p.as_unit() {
                    Some((v, "ch")) => v * font_size_for_ch * 0.45,
                    Some((v, "ex")) => v * font_size_for_ch * 0.50,
                    Some((v, "ic")) => v * font_size_for_ch,
                    Some((v, "lh")) => v * font_size_for_ch * 1.40,
                    _ => p.unit_to_px(),
                };
                style.set(prop.clone(), Value::Unit(px, Unit::Px));
            } else if let Some(pct) = p.as_percentage() {
                style.set(prop.clone(), Value::Unit(pct, Unit::Percent));
            } else if let Some(val) = p.as_number() {
                style.set(prop.clone(), Value::Unit(val, Unit::Px));
            } else if let Some(s) = p.as_string() {
                style.set(prop.clone(), Value::Keyword(intern(s)));
            }
        }
    }

    // --- line-height: unitless number is a font-size multiplier, not pixels ---
    if let Some(p) = prop_map.get("line-height") {
        if p.as_unit().is_some() {
            style.set(StyleProperty::LineHeight, Value::Unit(p.unit_to_px(), Unit::Px));
        } else if let Some(val) = p.as_number() {
            style.set(StyleProperty::LineHeight, Value::Number(val));
        } else if let Some(s) = p.as_string() {
            style.set(StyleProperty::LineHeight, Value::Keyword(intern(s)));
        }
    }

    // --- Color properties ---
    let color_props: &[(&str, StyleProperty)] = &[
        ("color", StyleProperty::Color),
        ("background-color", StyleProperty::BackgroundColor),
        ("background", StyleProperty::BackgroundColor),
        ("border-top-color", StyleProperty::BorderTopColor),
        ("border-right-color", StyleProperty::BorderRightColor),
        ("border-bottom-color", StyleProperty::BorderBottomColor),
        ("border-left-color", StyleProperty::BorderLeftColor),
    ];
    for (css_name, prop) in color_props {
        if let Some(p) = prop_map.get(css_name) {
            // CSS system color keywords (e.g. "Mark", "Field") aren't standard named colors —
            // RgbColor::from falls back to black for unknown strings. Map them to sensible
            // sRGB approximations before attempting normal color parsing.
            if let Some(s) = p.as_string() {
                if let Some((r, g, b, a)) = css_system_color(s) {
                    style.set(prop.clone(), Value::Color(r, g, b, a));
                    continue;
                }
            }
            if let Some((r, g, b, a)) = p.parse_color() {
                style.set(prop.clone(), Value::Color(r as u8, g as u8, b as u8, a as u8));
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
                _ => Display::Block,
            };
            style.set(StyleProperty::Display, Value::Display(display));
        }
    }

    // --- FontStyle ---
    if let Some(p) = prop_map.get("font-style") {
        if let Some(s) = p.as_string() {
            style.set(StyleProperty::FontStyle, Value::Keyword(intern(s)));
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
        style.set(StyleProperty::FontWeight, Value::FontWeight(fw));
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
            style.set(StyleProperty::TextAlign, Value::TextAlign(ta));
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
            style.set(StyleProperty::TextWrap, Value::TextWrap(tw));
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
                style.set(prop.clone(), Value::BorderStyle(str_to_border_style(s)));
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
                style.set(prop.clone(), Value::Number(n));
            }
        }
    }

    // --- font-family: stored as List([String("Arial"), Comma, String("sans-serif")]) ---
    // We reconstruct the CSS font-family stack string so that downstream code (parley's
    // FontFamily::Source) can parse it with FontFamilyName::parse_css_list().
    if let Some(p) = prop_map.get("font-family") {
        if let Some(s) = p.as_string() {
            // Single family: font-family: Arial
            style.set(StyleProperty::FontFamily, Value::Keyword(intern(s)));
        } else if let Some(list) = p.as_list() {
            // Multi-family list: font-family: "Arial", sans-serif
            // Build "Arial, sans-serif" — parley will parse this as a CSS fallback stack.
            let names: String = list
                .iter()
                .filter(|v| !v.is_comma())
                .filter_map(|v| v.as_string())
                .collect::<Vec<_>>()
                .join(", ");
            if !names.is_empty() {
                style.set(StyleProperty::FontFamily, Value::Keyword(intern(&names)));
            }
        }
    }

    // --- Keyword properties ---
    let kw_props: &[(&str, StyleProperty)] = &[
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
        ("white-space", StyleProperty::WhiteSpace),
        ("text-decoration-line", StyleProperty::TextDecorationLine),
    ];
    for (css_name, prop) in kw_props {
        if let Some(p) = prop_map.get(css_name) {
            if let Some(s) = p.as_string() {
                style.set(prop.clone(), Value::Keyword(intern(s)));
            }
        }
    }

    // --- text-decoration shorthand → text-decoration-line ---
    // `text-decoration` can contain a space-separated list of values (e.g. "underline line-through").
    // We extract the relevant decoration keywords and store them as a single consolidated keyword.
    if let Some(p) = prop_map.get("text-decoration") {
        let raw = format!("{p}");
        let has_underline = raw.contains("underline");
        let has_line_through = raw.contains("line-through");
        let kw = if has_underline && has_line_through {
            "underline line-through"
        } else if has_underline {
            "underline"
        } else if has_line_through {
            "line-through"
        } else {
            "none"
        };
        if kw != "none" {
            style.set(StyleProperty::TextDecorationLine, Value::Keyword(intern(kw)));
        }
    }

    style
}

/// Maps CSS system color keywords to (r, g, b, a) sRGB values so they render as something
/// sensible rather than defaulting to black. RgbColor::from returns black for any unrecognised
/// string, so we intercept known system color names before the normal parse path.
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
