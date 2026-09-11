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
    GridPlacement, GridTemplateArea, GridTemplateComponent, LengthPercentage, LengthPercentageAuto, Line, Overflow,
    Point, Position, Rect, Size, Style, TextAlign, TrackSizingFunction,
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
        ts.border.top = self.get_border_lp(StyleProperty::BorderTopWidth, ts.border.top);
        ts.border.right = self.get_border_lp(StyleProperty::BorderRightWidth, ts.border.right);
        ts.border.bottom = self.get_border_lp(StyleProperty::BorderBottomWidth, ts.border.bottom);
        ts.border.left = self.get_border_lp(StyleProperty::BorderLeftWidth, ts.border.left);
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
        ts.align_content = self.get_align_content(StyleProperty::AlignContent, Some(AlignContent::FLEX_START));
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
        ts.grid_template_areas = self.get_grid_areas(ts.grid_template_areas);
        ts.grid_row = self.get_grid_line(StyleProperty::GridRow, ts.grid_row);
        ts.grid_column = self.get_grid_line(StyleProperty::GridColumn, ts.grid_column);
        // `grid-area` is the shorthand for both axes. The CSS engine does not expand it into
        // longhands, so it is read here and applied after them - an element that sets both gets
        // the shorthand, which is the common case (`grid-area: content` with no `grid-row`).
        if let Some((row, column)) = self.get_grid_area() {
            ts.grid_row = row;
            ts.grid_column = column;
        }

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
                // A cell is a block container: its block-level children stack, as do the anonymous
                // containers the inline layout emits for line boxes. Taffy's default direction is
                // `Row`, so a cell holding more than one block laid them out side by side - which
                // put Wikipedia's infobox image caption in a narrow strip beside the picture
                // instead of underneath it.
                ts.flex_direction = FlexDirection::Column;
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
                ts.align_items = Some(AlignItems::BASELINE);
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

        // CSS 2.1 §9.7: `float` blockifies the box and takes it out of normal flow. Taffy has no
        // float support, so model only the out-of-flow half here - as an absolutely positioned
        // box, which is the one thing Taffy does that a float also does: siblings lay out as if
        // it were not there, and an `auto` width shrinks to fit. `post_process_floats` then puts
        // it where the float rules say it goes. `float` does not apply to an absolutely
        // positioned box, so leave those alone.
        if ts.position != Position::Absolute && crate::layouter::float::float_side(self.doc, self.node_id).is_some() {
            ts.position = Position::Absolute;
            ts.inset = Rect::auto();

            // Blockification (CSS Display §2.7) only touches *inline-level* boxes, and maps each
            // to its block-level equivalent: `inline` and `inline-block` become `block`, but
            // `inline-flex` becomes `flex` and `inline-grid` becomes `grid`. Anything already
            // block-level - `block`, `flex`, `grid`, the table displays - is left alone. Forcing
            // `Display::Block` here laid a floated flex or grid container's children out as
            // blocks instead of as items.
            //
            // This has to read the CSS display rather than `ts.display`: by now the converter has
            // mapped `inline`, `inline-block` and every table part onto `Display::Flex` as well,
            // so the taffy value no longer distinguishes them from a real flex container.
            let keeps_its_formatting_context = matches!(
                self.get_own(&StyleProperty::Display),
                Some(Value::Display(
                    CssDisplay::Flex
                        | CssDisplay::InlineFlex
                        | CssDisplay::Grid
                        | CssDisplay::InlineGrid
                        | CssDisplay::Table
                        | CssDisplay::TableCaption
                        | CssDisplay::TableCell
                        | CssDisplay::TableFooterGroup
                        | CssDisplay::TableHeaderGroup
                        | CssDisplay::TableRow
                        | CssDisplay::TableRowGroup
                ))
            );
            if !keeps_its_formatting_context {
                ts.display = Display::Block;
            }
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

    /// Border widths must resolve through the *computed* value: the initial width is `medium`
    /// (3px) and `border-style: none` zeroes it, neither of which `get_own` can see.
    fn get_border_lp(&self, prop: StyleProperty, default: LengthPercentage) -> LengthPercentage {
        match self.doc.get_style(self.node_id, &prop) {
            Value::Unit(value, unit) => match unit {
                CssUnit::Px => LengthPercentage::length(value),
                CssUnit::Percent => LengthPercentage::percent(value / 100.0),
                CssUnit::Em | CssUnit::Rem => LengthPercentage::length(value * self.font_size_px()),
            },
            Value::Number(value) => LengthPercentage::length(value),
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
                "start" => Some(AlignItems::START),
                "end" => Some(AlignItems::END),
                "flex-start" => Some(AlignItems::FLEX_START),
                "flex-end" => Some(AlignItems::FLEX_END),
                "center" => Some(AlignItems::CENTER),
                "baseline" => Some(AlignItems::BASELINE),
                "stretch" => Some(AlignItems::STRETCH),
                _ => default,
            },
            _ => default,
        }
    }

    fn get_align_self(&self, prop: StyleProperty, default: Option<AlignSelf>) -> Option<AlignSelf> {
        match self.get_own(&prop) {
            Some(Value::Keyword(id)) => match lookup(id).as_str() {
                "auto" => None,
                "start" => Some(AlignSelf::START),
                "end" => Some(AlignSelf::END),
                "flex-start" => Some(AlignSelf::FLEX_START),
                "flex-end" => Some(AlignSelf::FLEX_END),
                "center" => Some(AlignSelf::CENTER),
                "baseline" => Some(AlignSelf::BASELINE),
                "stretch" => Some(AlignSelf::STRETCH),
                _ => default,
            },
            _ => default,
        }
    }

    fn get_align_content(&self, prop: StyleProperty, default: Option<AlignContent>) -> Option<AlignContent> {
        match self.get_own(&prop) {
            Some(Value::Keyword(id)) => match lookup(id).as_str() {
                "normal" => default,
                "start" => Some(AlignContent::START),
                "end" => Some(AlignContent::END),
                "flex-start" => Some(AlignContent::FLEX_START),
                "flex-end" => Some(AlignContent::FLEX_END),
                "center" => Some(AlignContent::CENTER),
                "stretch" => Some(AlignContent::STRETCH),
                "space-between" => Some(AlignContent::SPACE_BETWEEN),
                "space-evenly" => Some(AlignContent::SPACE_EVENLY),
                "space-around" => Some(AlignContent::SPACE_AROUND),
                _ => default,
            },
            _ => default,
        }
    }

    /// `text-align` inherits, so this must read the computed value - `get_own` sees nothing on a
    /// descendant that inherits it. `left`/`right` collapse onto `start`/`end` as elsewhere (LTR).
    fn get_text_align(&self, default: TextAlign) -> TextAlign {
        match self.doc.get_style(self.node_id, &StyleProperty::TextAlign) {
            Value::TextAlign(val) => match val {
                CssTextAlign::Center => TextAlign::LegacyCenter,
                CssTextAlign::Start | CssTextAlign::Left => TextAlign::LegacyLeft,
                CssTextAlign::End | CssTextAlign::Right => TextAlign::LegacyRight,
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

    /// `grid-template-areas`, as the rectangle each area name covers.
    fn get_grid_areas(&self, default: Vec<GridTemplateArea<String>>) -> Vec<GridTemplateArea<String>> {
        match self.get_own(&StyleProperty::GridTemplateAreas) {
            Some(Value::Keyword(id)) => {
                let s = lookup(id);
                match s.as_str() {
                    "none" | "" => Vec::new(),
                    _ => parse_grid_areas(s.as_str()),
                }
            }
            _ => default,
        }
    }

    /// `grid-area`, as `(grid-row, grid-column)`. `None` when the property is not set, so the
    /// longhands the caller already resolved are kept.
    fn get_grid_area(&self) -> Option<(Line<GridPlacement>, Line<GridPlacement>)> {
        let Some(Value::Keyword(id)) = self.get_own(&StyleProperty::GridArea) else {
            return None;
        };
        let s = lookup(id);
        parse_grid_area(s.as_str())
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

        // `repeat(<count>, <track-list>)` - expand a fixed integer count into that many
        // copies of its track list. `auto-fill`/`auto-fit` counts are not supported yet and
        // cause the whole value to be ignored (falls back to the default) rather than
        // mis-rendering.
        if let Some(inner) = token.strip_prefix("repeat(").and_then(|t| t.strip_suffix(')')) {
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
    if let Some(slash) = s.find('/') {
        let start_str = s[..slash].trim();
        let end_str = s[slash + 1..].trim();
        return Some(Line {
            start: parse_single_placement(start_str),
            end: parse_single_placement(end_str),
        });
    }
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
        let rest = rest.trim();
        if let Ok(n) = rest.parse::<u16>() {
            return span(n);
        }
        // `span <name>` - span until the next line with that name.
        if is_custom_ident(rest) {
            return GridPlacement::NamedSpan(rest.to_string(), 1);
        }
    }
    if let Ok(n) = s.parse::<i16>() {
        return GridPlacement::from_line_index(n);
    }
    // A bare identifier is a named line. `grid-area: content` names an *area*, whose implicit
    // `content-start` / `content-end` lines taffy derives from `grid-template-areas`, so the
    // same placement covers both spellings.
    if is_custom_ident(s) {
        return GridPlacement::NamedLine(s.to_string(), 1);
    }
    GridPlacement::Auto
}

/// A CSS `<custom-ident>`: letters, digits, `-` and `_`, not starting with a digit. Used to tell
/// a named grid line from a keyword or a malformed token.
fn is_custom_ident(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with(|c: char| c.is_ascii_digit())
        && s.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_')
}

/// Parse `grid-area` into `(grid-row, grid-column)`.
///
/// The shorthand is `<row-start> / <column-start> / <row-end> / <column-end>`, and any part
/// left out is copied from its opposite when that is a custom ident (so `grid-area: content`
/// places the item across the whole `content` area).
fn parse_grid_area(s: &str) -> Option<(Line<GridPlacement>, Line<GridPlacement>)> {
    let s = s.trim();
    if s.is_empty() || s == "auto" || s == "none" {
        return None;
    }
    let parts: Vec<&str> = s.split('/').map(str::trim).collect();
    let placement = |i: usize| parts.get(i).map_or(GridPlacement::Auto, |p| parse_single_placement(p));

    // An omitted end line repeats the start when the start is a name, and is `auto` otherwise -
    // css-grid-2 §8.4. The same rule gives the column axis its value when only one part is given.
    let mirror = |from: &GridPlacement, i: usize| match parts.get(i) {
        Some(part) => parse_single_placement(part),
        None => match from {
            GridPlacement::NamedLine(name, idx) => GridPlacement::NamedLine(name.clone(), *idx),
            _ => GridPlacement::Auto,
        },
    };

    let row_start = placement(0);
    let column_start = mirror(&row_start, 1);
    let row_end = mirror(&row_start, 2);
    let column_end = mirror(&column_start, 3);

    Some((
        Line {
            start: row_start,
            end: row_end,
        },
        Line {
            start: column_start,
            end: column_end,
        },
    ))
}

/// Parse `grid-template-areas` - one row per line, cells separated by whitespace - into the
/// rectangle each name covers, in taffy's 1-based grid line coordinates.
///
/// `.` (or any run of dots) is a null cell and names no area. A name that does not form a
/// rectangle is not rejected the way css-grid-2 requires; it gets its bounding box, which keeps
/// a typo from dropping the whole shell.
fn parse_grid_areas(s: &str) -> Vec<GridTemplateArea<String>> {
    // Preserve document order so a page's areas keep a stable order in the output.
    let mut order: Vec<&str> = Vec::new();
    let mut bounds: std::collections::HashMap<&str, (u16, u16, u16, u16)> = std::collections::HashMap::new();

    for (row, line) in s.lines().enumerate() {
        for (column, cell) in line.split_whitespace().enumerate() {
            if cell.chars().all(|c| c == '.') {
                continue;
            }
            let (row, column) = (row as u16, column as u16);
            match bounds.get_mut(cell) {
                Some((row_start, row_end, column_start, column_end)) => {
                    *row_start = (*row_start).min(row);
                    *row_end = (*row_end).max(row);
                    *column_start = (*column_start).min(column);
                    *column_end = (*column_end).max(column);
                }
                None => {
                    order.push(cell);
                    bounds.insert(cell, (row, row, column, column));
                }
            }
        }
    }

    order
        .into_iter()
        .filter_map(|name| {
            let (row_start, row_end, column_start, column_end) = *bounds.get(name)?;
            // Cell indices are 0-based; grid lines are 1-based and an area ends on the line
            // *after* its last cell.
            Some(GridTemplateArea {
                name: name.to_string(),
                row_start: row_start + 1,
                row_end: row_end + 2,
                column_start: column_start + 1,
                column_end: column_end + 2,
            })
        })
        .collect()
}

#[cfg(test)]
mod grid_area_tests {
    use super::{parse_grid_area, parse_grid_areas};
    use taffy::{GridPlacement, GridTemplateArea};

    fn area(name: &str, rows: (u16, u16), columns: (u16, u16)) -> GridTemplateArea<String> {
        GridTemplateArea {
            name: name.to_string(),
            row_start: rows.0,
            row_end: rows.1,
            column_start: columns.0,
            column_end: columns.1,
        }
    }

    #[test]
    fn areas_become_line_rectangles() {
        // Wikipedia's Vector-2022 page shell. Grid lines are 1-based and an area ends on the
        // line after its last cell, so a single cell in the first row spans lines 1 to 2.
        let parsed = parse_grid_areas("siteNotice siteNotice\ncolumnStart pageContent\nfooter footer");
        assert_eq!(
            parsed,
            vec![
                area("siteNotice", (1, 2), (1, 3)),
                area("columnStart", (2, 3), (1, 2)),
                area("pageContent", (2, 3), (2, 3)),
                area("footer", (3, 4), (1, 3)),
            ]
        );
    }

    #[test]
    fn an_area_spanning_rows_keeps_one_rectangle() {
        let parsed = parse_grid_areas("side head\nside body");
        assert_eq!(
            parsed,
            vec![
                area("side", (1, 3), (1, 2)),
                area("head", (1, 2), (2, 3)),
                area("body", (2, 3), (2, 3)),
            ]
        );
    }

    #[test]
    fn a_dot_cell_names_no_area() {
        let parsed = parse_grid_areas("titlebar .\ntitlebar columnEnd");
        assert_eq!(
            parsed,
            vec![area("titlebar", (1, 3), (1, 2)), area("columnEnd", (2, 3), (2, 3))]
        );
    }

    #[test]
    fn a_single_name_places_the_item_across_the_whole_area() {
        let (row, column) = parse_grid_area("columnStart").expect("a name is a placement");
        let named = |name: &str| GridPlacement::NamedLine(name.to_string(), 1);
        assert_eq!((row.start, row.end), (named("columnStart"), named("columnStart")));
        assert_eq!((column.start, column.end), (named("columnStart"), named("columnStart")));
    }

    #[test]
    fn the_slash_form_fills_each_axis() {
        let (row, column) = parse_grid_area("2 / 1 / 4 / 3").expect("line numbers are a placement");
        assert_eq!(
            (row.start, row.end),
            (GridPlacement::Line(2.into()), GridPlacement::Line(4.into()))
        );
        assert_eq!(
            (column.start, column.end),
            (GridPlacement::Line(1.into()), GridPlacement::Line(3.into()))
        );
    }

    #[test]
    fn auto_is_not_a_placement() {
        assert!(parse_grid_area("auto").is_none());
        assert!(parse_grid_area("").is_none());
    }
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
