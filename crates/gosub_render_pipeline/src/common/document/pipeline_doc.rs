use crate::common::document::node::{AttrMap, ElementData, Node, NodeType};
use crate::common::document::style::{
    intern, BorderStyle, Display, FontWeight, NodeStyle, StyleProperty, TextAlign, TextWrap, Unit, Value,
};
use crate::painter::commands::color::Color;
use crate::painter::commands::gradient::{ColorStop, Gradient, LinearGradient};
use cow_utils::CowUtils;
use gosub_interface::config::HasDocument;
use gosub_interface::css3::{CssProperty, CssPropertyMap, CssSystem, CssValue};
use gosub_interface::document::Document as _;
use gosub_interface::node::NodeType as GosubNodeType;
use gosub_shared::node::NodeId;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;

// ── Bridge: CssProperty → Value ──────────────────────────────────────────────

/// Convert a single `CssProperty` value into the internal `Value` representation.
/// Returns `None` when the property carries no usable value (e.g. `CssValue::None`).
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
            // parse_color returns 0..255 range — matches Value::Color(u8, u8, u8, u8)
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
            if p.as_unit().is_some() {
                Some(Value::Unit(p.unit_to_px(), Unit::Px))
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
                let names: String = list
                    .iter()
                    .filter(|v| !v.is_comma())
                    .filter_map(|v| v.as_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                if !names.is_empty() {
                    return Some(Value::Keyword(intern(&names)));
                }
            }
            None
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
                    "ch" => Value::Unit(v * 0.5, Unit::Em),
                    "ex" => Value::Unit(v * 0.5, Unit::Em),
                    "ic" => Value::Unit(v, Unit::Em),
                    "lh" => Value::Unit(v * 1.4, Unit::Em),
                    // `rem` is root-relative (always 16px here) and everything else is
                    // absolute/viewport — resolve straight to px, no element context needed.
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

/// Extract the first `url(...)` from a property's actual value. Handles both the
/// `background-image` longhand (a bare `url()` function) and the `background` shorthand,
/// whose value is a list like `[url(...), no-repeat]`.
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

// ── Gradient parsing ──────────────────────────────────────────────────────────

/// Search a property value (the `background-image` longhand or a `background` shorthand
/// list) for a `linear-gradient(...)` and parse it into a [`Gradient`].
fn css_property_gradient<S: CssSystem>(p: &S::Property) -> Option<Gradient> {
    if let Some((name, args)) = p.as_function() {
        if name.eq_ignore_ascii_case("linear-gradient") {
            return parse_linear_gradient::<S>(args);
        }
    }
    if let Some(list) = p.as_list() {
        return list.iter().find_map(css_value_gradient::<S>);
    }
    None
}

fn css_value_gradient<S: CssSystem>(v: &S::Value) -> Option<Gradient> {
    if let Some((name, args)) = v.as_function() {
        if name.eq_ignore_ascii_case("linear-gradient") {
            return parse_linear_gradient::<S>(args);
        }
    }
    if let Some(list) = v.as_list() {
        return list.iter().find_map(css_value_gradient::<S>);
    }
    None
}

/// Parse the argument list of a `linear-gradient(...)` into a [`Gradient`]: an optional
/// leading direction (`to <side>[ <side>]` or an `<angle>`) followed by two or more colour
/// stops. Stops without an explicit position are spread evenly between their neighbours.
fn parse_linear_gradient<S: CssSystem>(args: &[S::Value]) -> Option<Gradient> {
    // Split the flat argument list into comma-separated groups.
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

    // Collect colour stops with their (optional) declared positions.
    let mut colors: Vec<Color> = Vec::new();
    let mut offsets: Vec<Option<f32>> = Vec::new();
    for group in groups.iter().skip(first_stop) {
        let Some((r, g, b, a)) = group.iter().find_map(|v| v.as_color()) else {
            continue;
        };
        colors.push(Color::from_rgba(r / 255.0, g / 255.0, b / 255.0, a / 255.0));
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

    Some(Gradient::Linear(LinearGradient { angle_deg, stops }))
}

/// Parse the leading direction group of a `linear-gradient`. Returns the gradient-line
/// angle in CSS degrees, or `None` if the group is not a direction (i.e. it is a colour
/// stop and the gradient uses the default `to bottom`).
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

    /// The CSS `background` / `background-image` gradient for node `id`, if its background
    /// is a (currently only linear) gradient. Returns `None` for solid/image backgrounds.
    fn background_gradient(&self, _id: NodeId) -> Option<Gradient> {
        None
    }

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
}

impl<C> GosubDocumentAdapter<C>
where
    C: HasDocument,
    <C::CssSystem as CssSystem>::PropertyMap: Send + Sync,
{
    pub fn new(doc: Arc<C::Document>) -> Self {
        Self {
            doc,
            style_cache: Mutex::new(HashMap::new()),
            inline_style_cache: Mutex::new(HashMap::new()),
        }
    }

    /// Returns the cached `PropertyMap` for `id`, computing and caching it on first access.
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
        // CSS selectors cannot target text nodes — only elements.
        if self.doc.node_type(id) == GosubNodeType::TextNode {
            return (Default::default(), NodeStyle::new());
        }
        let sheets = self.doc.stylesheets();
        let mut prop_map = C::CssSystem::properties_from_node::<C>(&*self.doc, id, sheets).unwrap_or_default();
        for (_, prop) in prop_map.iter_mut() {
            prop.compute_value();
        }

        // Inline `style` attribute has highest specificity — store separately.
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
        let arc = self.cached_styles(id);

        // Inline styles (from `style` attribute) have highest specificity.
        if let Some(inline) = self.inline_style_cache.lock().get(&id) {
            if let Some(v) = inline.get_own(prop) {
                return Some(v.clone());
            }
        }

        // Computed styles via bridge: CssProperty → Value.
        let css_name = prop.css_name();

        // For `text-decoration-line`, check the `text-decoration` shorthand FIRST when it
        // is `none`.  The CSS shorthand `text-decoration: none` clears all decorations, but
        // because the definitions JSON has empty expanded_properties for the shorthand, it is
        // stored under the key "text-decoration" while the UA stylesheet's
        // `a { text-decoration-line: underline }` is stored under "text-decoration-line".
        // Without this early check the UA longhand would win over the author shorthand.
        if matches!(prop, StyleProperty::TextDecorationLine) {
            if let Some(p) = <_ as CssPropertyMap<C::CssSystem>>::get(arc.as_ref(), "text-decoration") {
                // `none` is represented as CssValue::None, not CssValue::String("none").
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
        // `background` shorthand. Shorthands are stored under their own key (not expanded into
        // longhands), so the shorthand must be inspected explicitly — same pattern as
        // `text-decoration` above. The returned keyword is the unresolved URL; the layouter
        // resolves it against the base URL and loads it into the media store.
        if matches!(prop, StyleProperty::BackgroundImage) {
            for key in ["background-image", "background"] {
                if let Some(p) = <_ as CssPropertyMap<C::CssSystem>>::get(arc.as_ref(), key) {
                    if let Some(url) = css_property_url::<C::CssSystem>(p) {
                        return Some(Value::Keyword(intern(&url)));
                    }
                }
            }
            return None;
        }

        if let Some(p) = <_ as CssPropertyMap<C::CssSystem>>::get(arc.as_ref(), css_name) {
            if let Some(v) = css_property_to_value::<C::CssSystem>(p, prop) {
                return Some(v);
            }
        }

        // HTML presentation attributes (bgcolor, width, …) as lowest-specificity fallback.
        if let Some(attrs) = self.doc.attributes(id) {
            return crate::common::document::inline_style::html_presentation_attr(attrs, prop);
        }

        None
    }

    fn background_gradient(&self, id: NodeId) -> Option<Gradient> {
        let arc = self.cached_styles(id);
        for key in ["background-image", "background"] {
            if let Some(p) = <_ as CssPropertyMap<C::CssSystem>>::get(arc.as_ref(), key) {
                if let Some(g) = css_property_gradient::<C::CssSystem>(p) {
                    return Some(g);
                }
            }
        }
        None
    }

    fn clear_style_cache(&self) {
        self.style_cache.lock().clear();
        self.inline_style_cache.lock().clear();
    }

    fn invalidate_style_for_nodes(&self, ids: &[NodeId]) {
        let mut cache = self.style_cache.lock();
        let mut inline_cache = self.inline_style_cache.lock();
        for id in ids {
            cache.remove(id);
            inline_cache.remove(id);
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
                // Styles are accessed via `doc.get_own_style()` rather than stored in
                // ElementData — CssTaffyConverter uses the PipelineDocument interface directly.
                let element_data = ElementData::new(tag_name, Some(attr_map), false, None);
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
