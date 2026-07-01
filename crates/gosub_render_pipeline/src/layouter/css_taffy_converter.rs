use crate::common::document::node::NodeId;
use crate::common::document::pipeline_doc::PipelineDocument;
use crate::common::document::style::{
    lookup, Display as CssDisplay, StyleProperty, TextAlign as CssTextAlign, Unit as CssUnit, Value,
};
use taffy::prelude::{
    minmax, span, FromFr, FromLength, MaxTrackSizingFunction, MinTrackSizingFunction, TaffyAuto, TaffyGridLine,
    TaffyMaxContent, TaffyMinContent, TaffyZero,
};
use taffy::{
    AlignContent, AlignItems, AlignSelf, BoxSizing, Dimension, Display, FlexDirection, FlexWrap, GridAutoFlow,
    GridPlacement, GridTemplateComponent, LengthPercentage, LengthPercentageAuto, Line, Overflow, Point, Position,
    Rect, Size, Style, TextAlign, TrackSizingFunction,
};

/// Converts CSS properties from a `PipelineDocument` node into a Taffy `Style`.
pub struct CssTaffyConverter<'a> {
    node_id: NodeId,
    doc: &'a dyn PipelineDocument,
}

impl<'a> CssTaffyConverter<'a> {
    pub fn new(node_id: NodeId, doc: &'a dyn PipelineDocument) -> Self {
        Self { node_id, doc }
    }

    fn get_own(&self, prop: &StyleProperty) -> Option<Value> {
        self.doc.get_own_style(self.node_id, prop)
    }

    /// Returns this element's *computed* font-size in px (resolving inheritance and
    /// em/rem), or 16px if unresolvable. Used to resolve font-relative lengths such as
    /// `em`/`ch` on other properties (e.g. `max-width: 17ch`).
    fn font_size_px(&self) -> f32 {
        match self.doc.get_style(self.node_id, &StyleProperty::FontSize) {
            Value::Unit(v, CssUnit::Px) => v,
            _ => 16.0,
        }
    }

    fn get_f32(&self, prop: StyleProperty, default: f32) -> f32 {
        match self.get_own(&prop) {
            Some(Value::Number(num)) => num,
            _ => default,
        }
    }

    fn get_f32_opt(&self, prop: StyleProperty, default: Option<f32>) -> Option<f32> {
        match self.get_own(&prop) {
            Some(Value::Number(num)) => Some(num),
            _ => default,
        }
    }

    pub fn convert(&self, is_inline: bool) -> Style {
        let _ = is_inline; // parameter kept for API compatibility; inline wrapping handled by caller
        let mut ts = Style::default();

        ts.display = self.get_display(ts.display);
        // Taffy's built-in default is BorderBox, but the CSS spec default is content-box.
        ts.box_sizing = self.get_box_sizing(BoxSizing::ContentBox);
        ts.overflow = Point {
            x: self.get_overflow(StyleProperty::OverflowX, ts.overflow.x),
            y: self.get_overflow(StyleProperty::OverflowY, ts.overflow.y),
        };
        ts.scrollbar_width = self.get_f32(StyleProperty::ScrollbarWidth, ts.scrollbar_width);
        ts.position = self.get_position(ts.position);

        ts.inset = self.get_inset(ts.inset);
        ts.margin.top = self.get_lpa(StyleProperty::MarginTop, ts.margin.top);
        ts.margin.right = self.get_lpa(StyleProperty::MarginRight, ts.margin.right);
        ts.margin.bottom = self.get_lpa(StyleProperty::MarginBottom, ts.margin.bottom);
        ts.margin.left = self.get_lpa(StyleProperty::MarginLeft, ts.margin.left);
        ts.padding.top = self.get_lp(StyleProperty::PaddingTop, ts.padding.top);
        ts.padding.right = self.get_lp(StyleProperty::PaddingRight, ts.padding.right);
        ts.padding.bottom = self.get_lp(StyleProperty::PaddingBottom, ts.padding.bottom);
        ts.padding.left = self.get_lp(StyleProperty::PaddingLeft, ts.padding.left);
        ts.border.top = self.get_lp(StyleProperty::BorderTopWidth, ts.border.top);
        ts.border.right = self.get_lp(StyleProperty::BorderRightWidth, ts.border.right);
        ts.border.bottom = self.get_lp(StyleProperty::BorderBottomWidth, ts.border.bottom);
        ts.border.left = self.get_lp(StyleProperty::BorderLeftWidth, ts.border.left);
        ts.size.width = self.get_dimension(StyleProperty::Width, ts.size.width);
        ts.size.height = self.get_dimension(StyleProperty::Height, ts.size.height);
        ts.min_size.width = self.get_dimension(StyleProperty::MinWidth, ts.min_size.width);
        ts.min_size.height = self.get_dimension(StyleProperty::MinHeight, ts.min_size.height);
        ts.max_size.width = self.get_dimension(StyleProperty::MaxWidth, ts.max_size.width);
        ts.max_size.height = self.get_dimension(StyleProperty::MaxHeight, ts.max_size.height);
        ts.aspect_ratio = self.get_f32_opt(StyleProperty::AspectRatio, ts.aspect_ratio);
        ts.gap = self.get_size_lp(StyleProperty::Gap, ts.gap);
        ts.align_items = self.get_align_items(StyleProperty::AlignItems, ts.align_items);
        ts.align_self = self.get_align_self(StyleProperty::AlignSelf, ts.align_self);
        // Default align-content to FlexStart rather than Taffy's None (= Stretch).
        ts.align_content = self.get_align_content(StyleProperty::AlignContent, Some(AlignContent::FlexStart));
        ts.justify_items = self.get_align_items(StyleProperty::JustifyItems, ts.justify_items);
        ts.justify_self = self.get_align_self(StyleProperty::JustifySelf, ts.justify_self);
        ts.justify_content = self.get_align_content(StyleProperty::JustifyContent, ts.justify_content);
        ts.text_align = self.get_text_align(ts.text_align);
        ts.flex_direction = self.get_flex_direction(ts.flex_direction);
        ts.flex_wrap = self.get_flex_wrap(ts.flex_wrap);
        ts.flex_grow = self.get_f32(StyleProperty::FlexGrow, ts.flex_grow);
        ts.flex_shrink = self.get_f32(StyleProperty::FlexShrink, ts.flex_shrink);
        ts.flex_basis = self.get_flex_basis(ts.flex_basis);
        ts.grid_template_rows = self.get_grid_template(StyleProperty::GridTemplateRows, ts.grid_template_rows);
        ts.grid_template_columns = self.get_grid_template(StyleProperty::GridTemplateColumns, ts.grid_template_columns);
        ts.grid_auto_rows = self.get_grid_auto(StyleProperty::GridAutoRows, ts.grid_auto_rows);
        ts.grid_auto_columns = self.get_grid_auto(StyleProperty::GridAutoColumns, ts.grid_auto_columns);
        ts.grid_auto_flow = self.get_grid_auto_flow(ts.grid_auto_flow);
        ts.grid_row = self.get_grid_line(StyleProperty::GridRow, ts.grid_row);
        ts.grid_column = self.get_grid_line(StyleProperty::GridColumn, ts.grid_column);

        // Adjust display for table and inline elements.
        match self.get_own(&StyleProperty::Display) {
            Some(Value::Display(CssDisplay::Table)) => {
                ts.display = Display::Flex;
                ts.flex_direction = FlexDirection::Column;
            }
            Some(Value::Display(CssDisplay::TableRow)) => {
                ts.display = Display::Flex;
                ts.flex_direction = FlexDirection::Row;
            }
            Some(Value::Display(CssDisplay::TableCell)) => {
                ts.display = Display::Flex;
                ts.flex_grow = 1.0;
            }
            Some(Value::Display(CssDisplay::TableFooterGroup)) => {
                ts.display = Display::Flex;
                ts.flex_direction = FlexDirection::Column;
            }
            Some(Value::Display(CssDisplay::TableHeaderGroup)) => {
                ts.display = Display::Flex;
                ts.flex_direction = FlexDirection::Column;
            }
            Some(Value::Display(CssDisplay::TableRowGroup)) => {
                ts.display = Display::Flex;
                ts.flex_direction = FlexDirection::Column;
            }
            Some(Value::Display(CssDisplay::InlineBlock)) => {
                ts.display = Display::Flex;
                ts.flex_direction = FlexDirection::Row;
                ts.flex_wrap = FlexWrap::NoWrap;
            }
            // CSS initial value for display is inline; treat unset the same as explicit inline.
            None | Some(Value::Display(CssDisplay::Inline)) => {
                ts.display = Display::Flex;
                ts.flex_direction = FlexDirection::Row;
                ts.flex_wrap = FlexWrap::Wrap;
                ts.align_items = Some(AlignItems::Baseline);
            }
            // inline-flex / inline-grid: internally flex/grid, but participates inline.
            Some(Value::Display(CssDisplay::InlineFlex)) => {
                ts.display = Display::Flex;
                ts.flex_direction = FlexDirection::Row;
            }
            Some(Value::Display(CssDisplay::InlineGrid)) => {
                ts.display = Display::Grid;
            }
            _ => {}
        }

        ts
    }

    fn get_flex_wrap(&self, default: FlexWrap) -> FlexWrap {
        match self.get_own(&StyleProperty::FlexWrap) {
            Some(Value::Keyword(id)) => match lookup(id).as_str() {
                "nowrap" => FlexWrap::NoWrap,
                "wrap" => FlexWrap::Wrap,
                "wrap-reverse" => FlexWrap::WrapReverse,
                _ => default,
            },
            _ => default,
        }
    }

    fn get_flex_basis(&self, default: Dimension) -> Dimension {
        match self.get_own(&StyleProperty::FlexBasis) {
            Some(Value::Unit(val, unit)) => match unit {
                CssUnit::Percent => Dimension::percent(val / 100.0),
                _ => Dimension::from_length(val),
            },
            Some(Value::Number(val)) => Dimension::from_length(val),
            Some(Value::Keyword(id)) if lookup(id) == "auto" => Dimension::auto(),
            _ => default,
        }
    }

    fn get_flex_direction(&self, default: FlexDirection) -> FlexDirection {
        match self.get_own(&StyleProperty::FlexDirection) {
            Some(Value::Keyword(id)) => match lookup(id).as_str() {
                "row" => FlexDirection::Row,
                "row-reverse" => FlexDirection::RowReverse,
                "column" => FlexDirection::Column,
                "column-reverse" => FlexDirection::ColumnReverse,
                _ => default,
            },
            _ => default,
        }
    }

    fn get_display(&self, default: Display) -> Display {
        match self.get_own(&StyleProperty::Display) {
            Some(Value::Display(val)) => match val {
                CssDisplay::Block => Display::Block,
                CssDisplay::InlineBlock => Display::Block, // We override this later
                CssDisplay::Inline => Display::Block,      // We override this later
                CssDisplay::Flex => Display::Flex,
                CssDisplay::InlineFlex => Display::Flex, // We override to inline below
                CssDisplay::Grid => Display::Grid,
                CssDisplay::InlineGrid => Display::Grid, // We override to inline below
                CssDisplay::None => Display::None,
                _ => Display::Block,
            },
            _ => default,
        }
    }

    fn get_position(&self, default: Position) -> Position {
        match self.get_own(&StyleProperty::Position) {
            Some(Value::Keyword(id)) => match lookup(id).as_str() {
                "relative" => Position::Relative,
                "absolute" => Position::Absolute,
                "static" => Position::Relative,
                "fixed" => Position::Absolute,
                "sticky" => Position::Relative,
                _ => default,
            },
            _ => default,
        }
    }

    fn get_lpa(&self, prop: StyleProperty, default: LengthPercentageAuto) -> LengthPercentageAuto {
        match self.get_own(&prop) {
            Some(Value::Unit(value, unit)) => match unit {
                CssUnit::Px => LengthPercentageAuto::length(value),
                CssUnit::Percent => LengthPercentageAuto::percent(value / 100.0),
                CssUnit::Em | CssUnit::Rem => LengthPercentageAuto::length(value * self.font_size_px()),
            },
            Some(Value::Number(value)) => LengthPercentageAuto::length(value),
            Some(Value::Keyword(id)) if lookup(id) == "auto" => LengthPercentageAuto::auto(),
            _ => default,
        }
    }

    fn get_lp(&self, prop: StyleProperty, default: LengthPercentage) -> LengthPercentage {
        match self.get_own(&prop) {
            Some(Value::Unit(value, unit)) => match unit {
                CssUnit::Px => LengthPercentage::length(value),
                CssUnit::Percent => LengthPercentage::percent(value / 100.0),
                CssUnit::Em | CssUnit::Rem => LengthPercentage::length(value * self.font_size_px()),
            },
            Some(Value::Number(value)) => LengthPercentage::length(value),
            _ => default,
        }
    }

    fn get_dimension(&self, prop: StyleProperty, default: Dimension) -> Dimension {
        match self.get_own(&prop) {
            Some(Value::Unit(value, unit)) => match unit {
                CssUnit::Px => Dimension::from_length(value),
                CssUnit::Percent => Dimension::percent(value / 100.0),
                CssUnit::Em | CssUnit::Rem => Dimension::from_length(value * self.font_size_px()),
            },
            Some(Value::Number(value)) => Dimension::from_length(value),
            _ => default,
        }
    }

    fn get_size_lp(&self, prop: StyleProperty, default: Size<LengthPercentage>) -> Size<LengthPercentage> {
        match self.get_own(&prop) {
            Some(Value::Unit(value, unit)) => match unit {
                CssUnit::Px => Size::length(value),
                CssUnit::Percent => Size::percent(value / 100.0),
                CssUnit::Em | CssUnit::Rem => Size::length(value * self.font_size_px()),
            },
            Some(Value::Number(value)) => Size::length(value),
            _ => default,
        }
    }

    fn get_align_items(&self, prop: StyleProperty, default: Option<AlignItems>) -> Option<AlignItems> {
        match self.get_own(&prop) {
            Some(Value::Keyword(id)) => match lookup(id).as_str() {
                "start" => Some(AlignItems::Start),
                "end" => Some(AlignItems::End),
                "flex-start" => Some(AlignItems::FlexStart),
                "flex-end" => Some(AlignItems::FlexEnd),
                "center" => Some(AlignItems::Center),
                "baseline" => Some(AlignItems::Baseline),
                "stretch" => Some(AlignItems::Stretch),
                _ => default,
            },
            _ => default,
        }
    }

    fn get_align_self(&self, prop: StyleProperty, default: Option<AlignSelf>) -> Option<AlignSelf> {
        match self.get_own(&prop) {
            Some(Value::Keyword(id)) => match lookup(id).as_str() {
                "auto" => None,
                "start" => Some(AlignSelf::Start),
                "end" => Some(AlignSelf::End),
                "flex-start" => Some(AlignSelf::FlexStart),
                "flex-end" => Some(AlignSelf::FlexEnd),
                "center" => Some(AlignSelf::Center),
                "baseline" => Some(AlignSelf::Baseline),
                "stretch" => Some(AlignSelf::Stretch),
                _ => default,
            },
            _ => default,
        }
    }

    fn get_align_content(&self, prop: StyleProperty, default: Option<AlignContent>) -> Option<AlignContent> {
        match self.get_own(&prop) {
            Some(Value::Keyword(id)) => match lookup(id).as_str() {
                "normal" => default,
                "start" => Some(AlignContent::Start),
                "end" => Some(AlignContent::End),
                "flex-start" => Some(AlignContent::FlexStart),
                "flex-end" => Some(AlignContent::FlexEnd),
                "center" => Some(AlignContent::Center),
                "stretch" => Some(AlignContent::Stretch),
                "space-between" => Some(AlignContent::SpaceBetween),
                "space-evenly" => Some(AlignContent::SpaceEvenly),
                "space-around" => Some(AlignContent::SpaceAround),
                _ => default,
            },
            _ => default,
        }
    }

    fn get_text_align(&self, default: TextAlign) -> TextAlign {
        match self.get_own(&StyleProperty::TextAlign) {
            Some(Value::TextAlign(val)) => match val {
                CssTextAlign::Center => TextAlign::LegacyCenter,
                CssTextAlign::Start => TextAlign::LegacyLeft,
                CssTextAlign::End => TextAlign::LegacyRight,
                _ => default,
            },
            _ => default,
        }
    }

    fn get_inset(&self, default: Rect<LengthPercentageAuto>) -> Rect<LengthPercentageAuto> {
        Rect {
            top: self.get_lpa(StyleProperty::InsetBlockStart, default.top),
            right: self.get_lpa(StyleProperty::InsetInlineEnd, default.right),
            bottom: self.get_lpa(StyleProperty::InsetBlockEnd, default.bottom),
            left: self.get_lpa(StyleProperty::InsetInlineStart, default.left),
        }
    }

    fn get_overflow(&self, prop: StyleProperty, default: Overflow) -> Overflow {
        match self.get_own(&prop) {
            Some(Value::Keyword(id)) => match lookup(id).as_str() {
                "visible" => Overflow::Visible,
                "hidden" => Overflow::Hidden,
                "scroll" => Overflow::Scroll,
                "clip" => Overflow::Clip,
                _ => default,
            },
            _ => default,
        }
    }

    fn get_box_sizing(&self, default: BoxSizing) -> BoxSizing {
        match self.get_own(&StyleProperty::BoxSizing) {
            Some(Value::Keyword(id)) => match lookup(id).as_str() {
                "content-box" => BoxSizing::ContentBox,
                "border-box" => BoxSizing::BorderBox,
                _ => default,
            },
            _ => default,
        }
    }

    fn get_grid_template(
        &self,
        prop: StyleProperty,
        default: Vec<GridTemplateComponent<String>>,
    ) -> Vec<GridTemplateComponent<String>> {
        match self.get_own(&prop) {
            Some(Value::Keyword(id)) => {
                let s = lookup(id);
                match s.as_str() {
                    "none" | "" => Vec::new(),
                    _ => parse_grid_template(s.as_str()).unwrap_or(default),
                }
            }
            _ => default,
        }
    }

    fn get_grid_auto_flow(&self, default: GridAutoFlow) -> GridAutoFlow {
        match self.get_own(&StyleProperty::GridAutoFlow) {
            Some(Value::Keyword(id)) => match lookup(id).as_str() {
                "row" => GridAutoFlow::Row,
                "column" => GridAutoFlow::Column,
                "row dense" => GridAutoFlow::RowDense,
                "column dense" => GridAutoFlow::ColumnDense,
                _ => default,
            },
            _ => default,
        }
    }

    fn get_grid_line(&self, prop: StyleProperty, default: Line<GridPlacement>) -> Line<GridPlacement> {
        match self.get_own(&prop) {
            Some(Value::Keyword(id)) => {
                let s = lookup(id);
                parse_grid_placement(s.as_str()).unwrap_or(default)
            }
            Some(Value::Number(n)) => Line {
                start: GridPlacement::from_line_index(n as i16),
                end: GridPlacement::Auto,
            },
            _ => default,
        }
    }

    fn get_grid_auto(&self, prop: StyleProperty, default: Vec<TrackSizingFunction>) -> Vec<TrackSizingFunction> {
        match self.get_own(&prop) {
            Some(Value::Keyword(id)) => {
                let s = lookup(id);
                match s.as_str() {
                    "auto" | "none" | "" => Vec::new(),
                    _ => parse_grid_template(s.as_str())
                        .map(|tracks| {
                            tracks
                                .into_iter()
                                .filter_map(|t| match t {
                                    GridTemplateComponent::Single(tsf) => Some(tsf),
                                    _ => None,
                                })
                                .collect()
                        })
                        .unwrap_or(default),
                }
            }
            _ => default,
        }
    }
}

/// Parse a single grid track token ("1fr", "200px", "auto", "50%") into a TrackSizingFunction.
fn parse_grid_track(token: &str) -> Option<TrackSizingFunction> {
    let token = token.trim();
    if token == "auto" {
        return Some(minmax(MinTrackSizingFunction::AUTO, MaxTrackSizingFunction::AUTO));
    }
    if token == "min-content" {
        return Some(minmax(
            MinTrackSizingFunction::MIN_CONTENT,
            MaxTrackSizingFunction::MIN_CONTENT,
        ));
    }
    if token == "max-content" {
        return Some(minmax(
            MinTrackSizingFunction::MAX_CONTENT,
            MaxTrackSizingFunction::MAX_CONTENT,
        ));
    }
    if let Some(rest) = token.strip_suffix("fr") {
        let v: f32 = rest.trim().parse().ok()?;
        return Some(minmax(MinTrackSizingFunction::ZERO, MaxTrackSizingFunction::from_fr(v)));
    }
    if let Some(rest) = token.strip_suffix("px") {
        let v: f32 = rest.trim().parse().ok()?;
        return Some(minmax(
            MinTrackSizingFunction::from_length(v),
            MaxTrackSizingFunction::from_length(v),
        ));
    }
    if let Some(rest) = token.strip_suffix('%') {
        let v: f32 = rest.trim().parse().ok()?;
        let lp = taffy::LengthPercentage::percent(v / 100.0);
        return Some(minmax(
            MinTrackSizingFunction::from(lp),
            MaxTrackSizingFunction::from(lp),
        ));
    }
    if let Some(rest) = token.strip_suffix("em") {
        let v: f32 = rest.trim().parse().ok()?;
        return Some(minmax(
            MinTrackSizingFunction::from_length(v * 16.0),
            MaxTrackSizingFunction::from_length(v * 16.0),
        ));
    }
    None
}

/// Split a track list into top-level tokens, keeping function calls like `repeat(3, 1fr)` or
/// `minmax(100px, 1fr)` whole (their inner whitespace/commas must not split the token).
fn split_grid_tokens(s: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    for ch in s.chars() {
        match ch {
            '(' => {
                depth += 1;
                current.push(ch);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                current.push(ch);
            }
            c if c.is_whitespace() && depth == 0 => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

/// Parse a grid-template-columns/rows value string ("1fr 1fr 1fr", "200px 1fr 100px",
/// "repeat(3, 1fr)", …).
fn parse_grid_template(s: &str) -> Option<Vec<GridTemplateComponent<String>>> {
    let mut tracks = Vec::new();
    for token in split_grid_tokens(s) {
        // Skip named line brackets like [line-name]
        if token.starts_with('[') {
            continue;
        }

        // `repeat(<count>, <track-list>)` — expand a fixed integer count into that many
        // copies of its track list. `auto-fill`/`auto-fit` counts are not supported yet and
        // cause the whole value to be ignored (falls back to the default) rather than
        // mis-rendering.
        if let Some(inner) = token
            .strip_prefix("repeat(")
            .and_then(|t| t.strip_suffix(')'))
        {
            let (count_str, track_str) = inner.split_once(',')?;
            let count: u16 = count_str.trim().parse().ok()?;
            let inner_tracks = parse_grid_template(track_str)?;
            for _ in 0..count {
                tracks.extend(inner_tracks.iter().cloned());
            }
            continue;
        }

        let tsf = parse_grid_track(&token)?;
        tracks.push(GridTemplateComponent::Single(tsf));
    }
    if tracks.is_empty() {
        None
    } else {
        Some(tracks)
    }
}

/// Parse a grid-column/row placement value ("auto", "span 2", "1", "2 / 4", …).
fn parse_grid_placement(s: &str) -> Option<Line<GridPlacement>> {
    let s = s.trim();
    if s == "auto" {
        return Some(Line {
            start: GridPlacement::Auto,
            end: GridPlacement::Auto,
        });
    }
    // Handle "start / end" notation
    if let Some(slash) = s.find('/') {
        let start_str = s[..slash].trim();
        let end_str = s[slash + 1..].trim();
        return Some(Line {
            start: parse_single_placement(start_str),
            end: parse_single_placement(end_str),
        });
    }
    // Single value
    Some(Line {
        start: parse_single_placement(s),
        end: GridPlacement::Auto,
    })
}

fn parse_single_placement(s: &str) -> GridPlacement {
    let s = s.trim();
    if s == "auto" {
        return GridPlacement::Auto;
    }
    if let Some(rest) = s.strip_prefix("span ") {
        if let Ok(n) = rest.trim().parse::<u16>() {
            return span(n);
        }
    }
    if let Ok(n) = s.parse::<i16>() {
        return GridPlacement::from_line_index(n);
    }
    GridPlacement::Auto
}

#[cfg(test)]
mod grid_template_tests {
    use super::{parse_grid_template, split_grid_tokens};

    #[test]
    fn splits_keep_functions_whole() {
        assert_eq!(split_grid_tokens("1fr 1fr 1fr"), vec!["1fr", "1fr", "1fr"]);
        assert_eq!(split_grid_tokens("210px 1fr"), vec!["210px", "1fr"]);
        assert_eq!(split_grid_tokens("repeat(3, 1fr)"), vec!["repeat(3, 1fr)"]);
        assert_eq!(
            split_grid_tokens("repeat(2, 1fr) 200px"),
            vec!["repeat(2, 1fr)", "200px"]
        );
        assert_eq!(
            split_grid_tokens("minmax(100px, 1fr) auto"),
            vec!["minmax(100px, 1fr)", "auto"]
        );
    }

    #[test]
    fn expands_repeat() {
        // repeat(3, 1fr) => three tracks
        assert_eq!(parse_grid_template("repeat(3, 1fr)").unwrap().len(), 3);
        // repeat over a two-track list => count * 2
        assert_eq!(parse_grid_template("repeat(2, 1fr 2fr)").unwrap().len(), 4);
        // repeat mixed with a standalone track
        assert_eq!(parse_grid_template("repeat(2, 1fr) 200px").unwrap().len(), 3);
    }

    #[test]
    fn plain_track_lists() {
        assert_eq!(parse_grid_template("1fr 1fr 1fr").unwrap().len(), 3);
        assert_eq!(parse_grid_template("210px 1fr").unwrap().len(), 2);
        assert_eq!(parse_grid_template("1fr").unwrap().len(), 1);
    }

    #[test]
    fn unsupported_falls_back_to_none() {
        // auto-fill count isn't supported yet -> None (caller uses the default instead of
        // mis-rendering).
        assert!(parse_grid_template("repeat(auto-fill, 1fr)").is_none());
        // Garbage token -> None
        assert!(parse_grid_template("bogus").is_none());
    }
}
