use crate::common::document::node::NodeId;
use crate::common::document::pipeline_doc::PipelineDocument;
use gosub_interface::style::{
    AlignValue, ComputedStyle, Display as CssDisplay, LengthPercentage as CssLengthPercentage,
    LengthPercentageAuto as CssLengthPercentageAuto, Overflow as CssOverflow, Position as CssPosition, Prop,
    TextAlign as CssTextAlign,
};
use std::sync::Arc;
use taffy::prelude::{
    minmax, span, FromFr, FromLength, MaxTrackSizingFunction, MinTrackSizingFunction, TaffyAuto, TaffyGridLine,
    TaffyMaxContent, TaffyMinContent, TaffyZero,
};
use taffy::{
    AlignContent, AlignItems, AlignSelf, BoxSizing, Dimension, Display, FlexDirection, FlexWrap, GridAutoFlow,
    GridPlacement, GridTemplateArea, GridTemplateAreas, GridTemplateComponent, LengthPercentage, LengthPercentageAuto,
    Line, Overflow, Point, Position, Rect, Size, Style, TextAlign, TrackSizingFunction,
};

/// Converts CSS properties from a `PipelineDocument` node into a Taffy `Style`.
pub struct CssTaffyConverter<'a> {
    node_id: NodeId,
    doc: &'a dyn PipelineDocument,
    style: Arc<ComputedStyle>,
}

impl<'a> CssTaffyConverter<'a> {
    pub fn new(node_id: NodeId, doc: &'a dyn PipelineDocument) -> Self {
        let style = doc.computed_style(node_id);
        Self { node_id, doc, style }
    }

    /// The element's own `display`, or `None` when the cascade assigned it none - which the
    /// display fixups below read as `inline`, the CSS initial value.
    fn declared_display(&self) -> Option<CssDisplay> {
        self.style.has(Prop::Display).then_some(self.style.box_group.display)
    }

    pub fn convert(&self, is_inline: bool) -> Style {
        let _ = is_inline; // parameter kept for API compatibility; inline wrapping handled by caller
        let mut ts = Style::default();

        ts.display = self.get_display(ts.display);
        // Taffy's built-in default is BorderBox, but the CSS spec default is content-box.
        ts.box_sizing = self.get_box_sizing(BoxSizing::ContentBox);
        ts.overflow = Point {
            x: self.get_overflow(Prop::OverflowX, self.style.box_group.overflow_x, ts.overflow.x),
            y: self.get_overflow(Prop::OverflowY, self.style.box_group.overflow_y, ts.overflow.y),
        };
        ts.scrollbar_width = self.style.box_group.scrollbar_width.unwrap_or(ts.scrollbar_width);
        ts.position = self.get_position(ts.position);

        let (margin, padding, size, border, flex, grid) = (
            &self.style.margin,
            &self.style.padding,
            &self.style.size,
            &self.style.border,
            &self.style.flex,
            &self.style.grid,
        );
        ts.inset = self.get_inset(ts.inset);
        ts.margin.top = self.lpa(Prop::MarginTop, margin.top, ts.margin.top);
        ts.margin.right = self.lpa(Prop::MarginRight, margin.right, ts.margin.right);
        ts.margin.bottom = self.lpa(Prop::MarginBottom, margin.bottom, ts.margin.bottom);
        ts.margin.left = self.lpa(Prop::MarginLeft, margin.left, ts.margin.left);
        ts.padding.top = self.lp(Prop::PaddingTop, padding.top, ts.padding.top);
        ts.padding.right = self.lp(Prop::PaddingRight, padding.right, ts.padding.right);
        ts.padding.bottom = self.lp(Prop::PaddingBottom, padding.bottom, ts.padding.bottom);
        ts.padding.left = self.lp(Prop::PaddingLeft, padding.left, ts.padding.left);
        ts.border.top = Self::border_lp(border.top_width);
        ts.border.right = Self::border_lp(border.right_width);
        ts.border.bottom = Self::border_lp(border.bottom_width);
        ts.border.left = Self::border_lp(border.left_width);
        ts.size.width = self.dimension(Prop::Width, size.width, ts.size.width);
        ts.size.height = self.dimension(Prop::Height, size.height, ts.size.height);
        ts.min_size.width = self.lpa(Prop::MinWidth, size.min_width, ts.min_size.width);
        ts.min_size.height = self.lpa(Prop::MinHeight, size.min_height, ts.min_size.height);
        ts.max_size.width = self.lpa(Prop::MaxWidth, size.max_width, ts.max_size.width);
        ts.max_size.height = self.lpa(Prop::MaxHeight, size.max_height, ts.max_size.height);
        ts.aspect_ratio = self.style.box_group.aspect_ratio.or(ts.aspect_ratio);
        ts.gap = self.get_gap(ts.gap);
        ts.align_items = self.get_align_items(Prop::AlignItems, flex.align_items, ts.align_items);
        ts.align_self = self.get_align_self(Prop::AlignSelf, flex.align_self, ts.align_self);
        // Default align-content to FlexStart rather than Taffy's None (= Stretch).
        ts.align_content =
            self.get_align_content(Prop::AlignContent, flex.align_content, Some(AlignContent::FLEX_START));
        ts.justify_items = self.get_align_items(Prop::JustifyItems, flex.justify_items, ts.justify_items);
        ts.justify_self = self.get_align_self(Prop::JustifySelf, flex.justify_self, ts.justify_self);
        ts.justify_content = self.get_align_content(Prop::JustifyContent, flex.justify_content, ts.justify_content);
        ts.text_align = self.get_text_align(ts.text_align);
        ts.flex_direction = self.get_flex_direction(ts.flex_direction);
        ts.flex_wrap = self.get_flex_wrap(ts.flex_wrap);
        ts.flex_grow = if self.style.has(Prop::FlexGrow) {
            flex.grow
        } else {
            ts.flex_grow
        };
        ts.flex_shrink = if self.style.has(Prop::FlexShrink) {
            flex.shrink
        } else {
            ts.flex_shrink
        };
        ts.flex_basis = self.get_flex_basis(ts.flex_basis);
        ts.grid_template_rows =
            self.get_grid_template(Prop::GridTemplateRows, &grid.template_rows, ts.grid_template_rows);
        ts.grid_template_columns = self.get_grid_template(
            Prop::GridTemplateColumns,
            &grid.template_columns,
            ts.grid_template_columns,
        );
        ts.grid_auto_rows = self.get_grid_auto(Prop::GridAutoRows, &grid.auto_rows, ts.grid_auto_rows);
        ts.grid_auto_columns = self.get_grid_auto(Prop::GridAutoColumns, &grid.auto_columns, ts.grid_auto_columns);
        ts.grid_auto_flow = self.get_grid_auto_flow(ts.grid_auto_flow);
        ts.grid_template_areas = self.get_grid_areas(ts.grid_template_areas);
        ts.grid_row = self.get_grid_line(Prop::GridRow, &grid.row, ts.grid_row);
        ts.grid_column = self.get_grid_line(Prop::GridColumn, &grid.column, ts.grid_column);
        // `grid-area` is the shorthand for both axes. The CSS engine does not expand it into
        // longhands, so it is read here and applied after them - an element that sets both gets
        // the shorthand, which is the common case (`grid-area: content` with no `grid-row`).
        //
        // KNOWN LIMIT: that makes the shorthand win regardless of source order, so a later
        // `grid-row: 2` after `grid-area: content` is ignored. Computed styles reach this point
        // with no record of the order they were declared in; fixing it properly means expanding
        // `grid-area` into its four longhands in the cascade, where the order still exists.
        if let Some((row, column)) = self.get_grid_area() {
            ts.grid_row = row;
            ts.grid_column = column;
        }

        // Adjust display for table and inline elements.
        match self.declared_display() {
            Some(CssDisplay::Table | CssDisplay::InlineTable) => {
                ts.display = Display::Flex;
                ts.flex_direction = FlexDirection::Column;
            }
            Some(CssDisplay::TableRow) => {
                ts.display = Display::Flex;
                ts.flex_direction = FlexDirection::Row;
            }
            Some(CssDisplay::TableCell) => {
                // Block inner layout so child blocks stack vertically; flex_grow
                // still applies to the cell as an item of its flex-row row.
                ts.display = Display::Block;
                ts.flex_grow = 1.0;
            }
            Some(CssDisplay::TableFooterGroup | CssDisplay::TableHeaderGroup | CssDisplay::TableRowGroup) => {
                ts.display = Display::Flex;
                ts.flex_direction = FlexDirection::Column;
            }
            // <col>/<colgroup> generate no boxes; lattice reads their widths
            // straight from the DOM.
            Some(CssDisplay::TableColumn | CssDisplay::TableColumnGroup) => {
                ts.display = Display::None;
            }
            Some(CssDisplay::InlineBlock) => {
                ts.display = Display::Flex;
                ts.flex_direction = FlexDirection::Row;
                ts.flex_wrap = FlexWrap::NoWrap;
            }
            // CSS initial value for display is inline; treat unset the same as explicit inline.
            None | Some(CssDisplay::Inline) => {
                ts.display = Display::Flex;
                ts.flex_direction = FlexDirection::Row;
                ts.flex_wrap = FlexWrap::Wrap;
                ts.align_items = Some(AlignItems::BASELINE);
            }
            // inline-flex / inline-grid: internally flex/grid, but participates inline.
            Some(CssDisplay::InlineFlex) => {
                ts.display = Display::Flex;
                ts.flex_direction = FlexDirection::Row;
            }
            Some(CssDisplay::InlineGrid) => {
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
                self.declared_display(),
                Some(
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
                )
            );
            if !keeps_its_formatting_context {
                ts.display = Display::Block;
            }
        }

        // CSS 2 §10.3.7: an absolutely-positioned box with `width: auto` shrinks to fit but
        // never beyond its containing block. Taffy sizes such children to raw fit-content
        // (an abs-positioned wide table would overflow the viewport), so cap the BORDER box
        // at 100%. Flipping box-sizing is safe here: it only reinterprets non-auto sizes,
        // and every size except the cap itself is auto in this branch.
        if ts.position == Position::Absolute
            && ts.size.width.is_auto()
            && ts.size.height.is_auto()
            && ts.max_size.width.is_auto()
            && ts.min_size.width.is_auto()
        {
            ts.box_sizing = BoxSizing::BorderBox;
            ts.max_size.width = LengthPercentageAuto::percent(1.0);
        }

        ts
    }

    fn get_flex_wrap(&self, default: FlexWrap) -> FlexWrap {
        if !self.style.has(Prop::FlexWrap) {
            return default;
        }
        match self.style.flex.wrap {
            gosub_interface::style::FlexWrap::NoWrap => FlexWrap::NoWrap,
            gosub_interface::style::FlexWrap::Wrap => FlexWrap::Wrap,
            gosub_interface::style::FlexWrap::WrapReverse => FlexWrap::WrapReverse,
        }
    }

    fn get_flex_basis(&self, default: Dimension) -> Dimension {
        self.dimension(Prop::FlexBasis, self.style.flex.basis, default)
    }

    fn get_flex_direction(&self, default: FlexDirection) -> FlexDirection {
        if !self.style.has(Prop::FlexDirection) {
            return default;
        }
        match self.style.flex.direction {
            gosub_interface::style::FlexDirection::Row => FlexDirection::Row,
            gosub_interface::style::FlexDirection::RowReverse => FlexDirection::RowReverse,
            gosub_interface::style::FlexDirection::Column => FlexDirection::Column,
            gosub_interface::style::FlexDirection::ColumnReverse => FlexDirection::ColumnReverse,
        }
    }

    fn get_display(&self, default: Display) -> Display {
        match self.declared_display() {
            Some(CssDisplay::Block) => Display::Block,
            // Overridden below, once the CSS display is consulted again.
            Some(CssDisplay::InlineBlock | CssDisplay::Inline) => Display::Block,
            Some(CssDisplay::Flex | CssDisplay::InlineFlex) => Display::Flex,
            Some(CssDisplay::Grid | CssDisplay::InlineGrid) => Display::Grid,
            Some(CssDisplay::None) => Display::None,
            Some(_) => Display::Block,
            None => default,
        }
    }

    fn get_position(&self, default: Position) -> Position {
        if !self.style.has(Prop::Position) {
            return default;
        }
        match self.style.box_group.position {
            CssPosition::Absolute | CssPosition::Fixed => Position::Absolute,
            CssPosition::Relative | CssPosition::Static | CssPosition::Sticky => Position::Relative,
        }
    }

    /// A declared `<length-percentage> | auto` as taffy's own, or the caller's default when the
    /// element declared none.
    fn lpa(&self, prop: Prop, value: CssLengthPercentageAuto, default: LengthPercentageAuto) -> LengthPercentageAuto {
        if !self.style.has(prop) {
            return default;
        }
        match value {
            CssLengthPercentageAuto::Px(px) => LengthPercentageAuto::length(px),
            CssLengthPercentageAuto::Percent(pct) => LengthPercentageAuto::percent(pct / 100.0),
            CssLengthPercentageAuto::Auto => LengthPercentageAuto::auto(),
        }
    }

    fn lp(&self, prop: Prop, value: CssLengthPercentage, default: LengthPercentage) -> LengthPercentage {
        if !self.style.has(prop) {
            return default;
        }
        Self::to_taffy_lp(value)
    }

    fn to_taffy_lp(value: CssLengthPercentage) -> LengthPercentage {
        match value {
            CssLengthPercentage::Px(px) => LengthPercentage::length(px),
            CssLengthPercentage::Percent(pct) => LengthPercentage::percent(pct / 100.0),
        }
    }

    /// A border width, which is always known: the initial value is `medium` and
    /// `border-style: none` zeroes it, both of which the computed style has already settled.
    fn border_lp(width: f32) -> LengthPercentage {
        LengthPercentage::length(width)
    }

    fn dimension(&self, prop: Prop, value: CssLengthPercentageAuto, default: Dimension) -> Dimension {
        if !self.style.has(prop) {
            return default;
        }
        match value {
            CssLengthPercentageAuto::Px(px) => Dimension::from_length(px),
            CssLengthPercentageAuto::Percent(pct) => Dimension::percent(pct / 100.0),
            CssLengthPercentageAuto::Auto => Dimension::auto(),
        }
    }

    fn get_gap(&self, default: Size<LengthPercentage>) -> Size<LengthPercentage> {
        if !self.style.has(Prop::Gap) {
            return default;
        }
        match self.style.flex.gap {
            CssLengthPercentage::Px(px) => Size::length(px),
            CssLengthPercentage::Percent(pct) => Size::percent(pct / 100.0),
        }
    }

    fn get_align_items(&self, prop: Prop, value: AlignValue, default: Option<AlignItems>) -> Option<AlignItems> {
        if !self.style.has(prop) {
            return default;
        }
        match value {
            AlignValue::Start => Some(AlignItems::START),
            AlignValue::End => Some(AlignItems::END),
            AlignValue::FlexStart => Some(AlignItems::FLEX_START),
            AlignValue::FlexEnd => Some(AlignItems::FLEX_END),
            AlignValue::Center => Some(AlignItems::CENTER),
            AlignValue::Baseline => Some(AlignItems::BASELINE),
            AlignValue::Stretch => Some(AlignItems::STRETCH),
            _ => default,
        }
    }

    fn get_align_self(&self, prop: Prop, value: AlignValue, default: Option<AlignSelf>) -> Option<AlignSelf> {
        if !self.style.has(prop) {
            return default;
        }
        match value {
            AlignValue::Auto => None,
            AlignValue::Start => Some(AlignSelf::START),
            AlignValue::End => Some(AlignSelf::END),
            AlignValue::FlexStart => Some(AlignSelf::FLEX_START),
            AlignValue::FlexEnd => Some(AlignSelf::FLEX_END),
            AlignValue::Center => Some(AlignSelf::CENTER),
            AlignValue::Baseline => Some(AlignSelf::BASELINE),
            AlignValue::Stretch => Some(AlignSelf::STRETCH),
            _ => default,
        }
    }

    fn get_align_content(&self, prop: Prop, value: AlignValue, default: Option<AlignContent>) -> Option<AlignContent> {
        if !self.style.has(prop) {
            return default;
        }
        match value {
            AlignValue::Normal => default,
            AlignValue::Start => Some(AlignContent::START),
            AlignValue::End => Some(AlignContent::END),
            AlignValue::FlexStart => Some(AlignContent::FLEX_START),
            AlignValue::FlexEnd => Some(AlignContent::FLEX_END),
            AlignValue::Center => Some(AlignContent::CENTER),
            AlignValue::Stretch => Some(AlignContent::STRETCH),
            AlignValue::SpaceBetween => Some(AlignContent::SPACE_BETWEEN),
            AlignValue::SpaceEvenly => Some(AlignContent::SPACE_EVENLY),
            AlignValue::SpaceAround => Some(AlignContent::SPACE_AROUND),
            _ => default,
        }
    }

    /// `text-align` inherits, so this reads the computed value rather than asking whether this
    /// element declared one. `left`/`right` collapse onto `start`/`end` as elsewhere (LTR).
    fn get_text_align(&self, default: TextAlign) -> TextAlign {
        match self.style.inherited.text_align {
            CssTextAlign::Center => TextAlign::LegacyCenter,
            CssTextAlign::Start | CssTextAlign::Left => TextAlign::LegacyLeft,
            CssTextAlign::End | CssTextAlign::Right => TextAlign::LegacyRight,
            _ => default,
        }
    }

    fn get_inset(&self, default: Rect<LengthPercentageAuto>) -> Rect<LengthPercentageAuto> {
        let inset = &self.style.inset;
        Rect {
            top: self.lpa(Prop::InsetBlockStart, inset.block_start, default.top),
            right: self.lpa(Prop::InsetInlineEnd, inset.inline_end, default.right),
            bottom: self.lpa(Prop::InsetBlockEnd, inset.block_end, default.bottom),
            left: self.lpa(Prop::InsetInlineStart, inset.inline_start, default.left),
        }
    }

    fn get_overflow(&self, prop: Prop, value: CssOverflow, default: Overflow) -> Overflow {
        if !self.style.has(prop) {
            return default;
        }
        match value {
            CssOverflow::Visible => Overflow::Visible,
            CssOverflow::Hidden => Overflow::Hidden,
            CssOverflow::Scroll => Overflow::Scroll,
            CssOverflow::Clip => Overflow::Clip,
            // Taffy has no `auto`; the scrollbar machinery is the engine's own.
            CssOverflow::Auto => default,
        }
    }

    fn get_box_sizing(&self, default: BoxSizing) -> BoxSizing {
        if !self.style.has(Prop::BoxSizing) {
            return default;
        }
        match self.style.box_group.box_sizing {
            gosub_interface::style::BoxSizing::ContentBox => BoxSizing::ContentBox,
            gosub_interface::style::BoxSizing::BorderBox => BoxSizing::BorderBox,
        }
    }

    fn get_grid_template(
        &self,
        prop: Prop,
        value: &str,
        default: Vec<GridTemplateComponent<String>>,
    ) -> Vec<GridTemplateComponent<String>> {
        if !self.style.has(prop) {
            return default;
        }
        match value {
            "none" | "" => Vec::new(),
            tracks => parse_grid_template(tracks).unwrap_or(default),
        }
    }

    fn get_grid_auto_flow(&self, default: GridAutoFlow) -> GridAutoFlow {
        if !self.style.has(Prop::GridAutoFlow) {
            return default;
        }
        match self.style.grid.auto_flow {
            gosub_interface::style::GridAutoFlow::Row => GridAutoFlow::Row,
            gosub_interface::style::GridAutoFlow::Column => GridAutoFlow::Column,
            gosub_interface::style::GridAutoFlow::RowDense => GridAutoFlow::RowDense,
            gosub_interface::style::GridAutoFlow::ColumnDense => GridAutoFlow::ColumnDense,
        }
    }

    fn get_grid_line(&self, prop: Prop, value: &str, default: Line<GridPlacement>) -> Line<GridPlacement> {
        if !self.style.has(prop) {
            return default;
        }
        parse_grid_placement(value).unwrap_or(default)
    }

    /// `grid-template-areas`, as the rectangle each area name covers.
    fn get_grid_areas(&self, default: Option<GridTemplateAreas<String>>) -> Option<GridTemplateAreas<String>> {
        if !self.style.has(Prop::GridTemplateAreas) {
            return default;
        }
        match &*self.style.grid.template_areas {
            "none" | "" => None,
            source => {
                let areas = parse_grid_areas(source);
                if areas.is_empty() {
                    return None;
                }
                // taffy 0.14 wants the template's shape alongside the areas. The rows are the
                // non-empty lines and the columns the widest of them, counted the same way the
                // parser walks the string so a ragged template agrees with the bounds it derived.
                let rows = source
                    .lines()
                    .filter(|l| !l.split_whitespace().next().is_none())
                    .count();
                let columns = source.lines().map(|l| l.split_whitespace().count()).max().unwrap_or(0);
                Some(GridTemplateAreas {
                    areas: areas.into_iter().collect(),
                    row_count: rows as u16,
                    column_count: columns as u16,
                })
            }
        }
    }

    /// `grid-area`, as `(grid-row, grid-column)`. `None` only when the property is not set, so the
    /// longhands the caller already resolved are kept.
    fn get_grid_area(&self) -> Option<(Line<GridPlacement>, Line<GridPlacement>)> {
        self.style
            .has(Prop::GridArea)
            .then(|| declared_grid_area(&self.style.grid.area))
    }

    fn get_grid_auto(&self, prop: Prop, value: &str, default: Vec<TrackSizingFunction>) -> Vec<TrackSizingFunction> {
        if !self.style.has(prop) {
            return default;
        }
        match value {
            "auto" | "none" | "" => Vec::new(),
            tracks => parse_grid_template(tracks)
                .map(|tracks| {
                    tracks
                        .into_iter()
                        .filter_map(|track| match track {
                            GridTemplateComponent::Single(sizing) => Some(sizing),
                            _ => None,
                        })
                        .collect()
                })
                .unwrap_or(default),
        }
    }
}

/// A track breadth that is a plain length or percentage (`200px`, `50%`, `1.5em`), which is the
/// part both sides of a track size share. `fr` and the keywords are handled by the callers,
/// because the two sides accept different ones.
fn parse_track_length(token: &str) -> Option<taffy::LengthPercentage> {
    // Zero is the one length CSS lets you write without a unit, and `minmax(0, 1fr)` - the shape
    // Tailwind emits for an equal-width grid - is where it turns up.
    if let Ok(v) = token.parse::<f32>() {
        return (v == 0.0).then(|| taffy::LengthPercentage::length(0.0));
    }
    if let Some(rest) = token.strip_suffix("px") {
        return Some(taffy::LengthPercentage::length(rest.trim().parse().ok()?));
    }
    if let Some(rest) = token.strip_suffix("em") {
        let v: f32 = rest.trim().parse().ok()?;
        return Some(taffy::LengthPercentage::length(v * 16.0));
    }
    if let Some(rest) = token.strip_suffix('%') {
        let v: f32 = rest.trim().parse().ok()?;
        return Some(taffy::LengthPercentage::percent(v / 100.0));
    }
    None
}

/// The minimum of a track size. css-grid-1 calls it `<inflexible-breadth>`: a length, a
/// percentage, `auto`, `min-content` or `max-content` - never an `fr`, which is why the two sides
/// are parsed apart.
fn parse_min_track(token: &str) -> Option<MinTrackSizingFunction> {
    match token {
        "auto" => Some(MinTrackSizingFunction::AUTO),
        "min-content" => Some(MinTrackSizingFunction::MIN_CONTENT),
        "max-content" => Some(MinTrackSizingFunction::MAX_CONTENT),
        _ => parse_track_length(token).map(MinTrackSizingFunction::from),
    }
}

/// The maximum of a track size: everything a minimum accepts, plus `fr`.
fn parse_max_track(token: &str) -> Option<MaxTrackSizingFunction> {
    match token {
        "auto" => Some(MaxTrackSizingFunction::AUTO),
        "min-content" => Some(MaxTrackSizingFunction::MIN_CONTENT),
        "max-content" => Some(MaxTrackSizingFunction::MAX_CONTENT),
        _ => {
            if let Some(rest) = token.strip_suffix("fr") {
                let v: f32 = rest.trim().parse().ok()?;
                return Some(MaxTrackSizingFunction::from_fr(v));
            }
            parse_track_length(token).map(MaxTrackSizingFunction::from)
        }
    }
}

/// Parse a single grid track token ("1fr", "200px", "auto", "50%", "minmax(0, 1fr)") into a
/// TrackSizingFunction.
///
/// `minmax()` used to be missing, and a track list holding one parsed to nothing, so the whole
/// template was dropped and the grid fell back to a single implicit column - which is how
/// ingewikkeld.dev's "trusted by" grid, `repeat(7, minmax(0, 1fr))`, put every logo on a row of
/// its own. `split_grid_tokens` already keeps the call whole; only reading it was missing.
fn parse_grid_track(token: &str) -> Option<TrackSizingFunction> {
    let token = token.trim();

    if token.get(..7).is_some_and(|f| f.eq_ignore_ascii_case("minmax(")) && token.ends_with(')') {
        let args = &token[7..token.len() - 1];
        // Neither side can hold a comma of its own - both are single breadths - so one split is
        // the whole of it.
        let (min, max) = args.split_once(',')?;
        return Some(minmax(parse_min_track(min.trim())?, parse_max_track(max.trim())?));
    }

    // A bare `fr` is a maximum with a zero minimum; every other single value is both sides at
    // once.
    if let Some(rest) = token.strip_suffix("fr") {
        let v: f32 = rest.trim().parse().ok()?;
        return Some(minmax(MinTrackSizingFunction::ZERO, MaxTrackSizingFunction::from_fr(v)));
    }
    Some(minmax(parse_min_track(token)?, parse_max_track(token)?))
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
/// "repeat(3, 1fr)", ...).
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

/// Parse a grid-column/row placement value ("auto", "span 2", "1", "2 / 4", ...).
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
        // `span 2 main` - span until the *second* line with that name. A span counts forward
        // only, so a negative integer is not a span at all and the value is invalid; a negative
        // *line* index is fine and stays so below.
        if let Some((n, name)) = split_index_and_name(rest) {
            if n > 0 {
                return GridPlacement::NamedSpan(name.to_string(), n as u16);
            }
        }
    }
    if let Ok(n) = s.parse::<i16>() {
        return GridPlacement::from_line_index(n);
    }
    // `<integer> && <custom-ident>` - the nth line with that name, written in either order.
    if let Some((n, name)) = split_index_and_name(s) {
        return GridPlacement::NamedLine(name.to_string(), n);
    }
    // A bare identifier is a named line. `grid-area: content` names an *area*, whose implicit
    // `content-start` / `content-end` lines taffy derives from `grid-template-areas`, so the
    // same placement covers both spellings.
    if is_custom_ident(s) {
        return GridPlacement::NamedLine(s.to_string(), 1);
    }
    GridPlacement::Auto
}

/// `<integer> && <custom-ident>`, in either order: `2 main-end` and `main-end 2` both name the
/// second line called `main-end`. Without this the whole value fell through to `auto`, so an item
/// placed on a repeated line was laid out wherever auto-placement happened to put it.
///
/// A zero index is not a line (css-grid-2 forbids it), and `1` is what taffy treats an unqualified
/// name as, so both are left to the plain `<custom-ident>` path above.
fn split_index_and_name(s: &str) -> Option<(i16, &str)> {
    let (first, rest) = s.split_once(char::is_whitespace)?;
    let second = rest.trim();
    if second.is_empty() || second.contains(char::is_whitespace) {
        return None;
    }
    let pair = match (first.parse::<i16>(), second.parse::<i16>()) {
        (Ok(n), Err(_)) => (n, second),
        (Err(_), Ok(n)) => (n, first),
        _ => return None,
    };
    (pair.0 != 0 && is_custom_ident(pair.1)).then_some(pair)
}

/// A CSS `<custom-ident>`: letters, digits, `-` and `_`, not starting with a digit. Used to tell
/// a named grid line from a keyword or a malformed token.
fn is_custom_ident(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with(|c: char| c.is_ascii_digit())
        && s.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_')
}

/// The placement a *declared* `grid-area` means.
///
/// `parse_grid_area` returns `None` for `auto` and `none`, which is right for "this names no
/// area" - but a declaration that says `auto` is the author resetting both axes, not saying
/// nothing. Treating the two the same left an earlier `grid-row` standing through a later
/// `grid-area: auto`.
fn declared_grid_area(s: &str) -> (Line<GridPlacement>, Line<GridPlacement>) {
    let auto = || Line {
        start: GridPlacement::Auto,
        end: GridPlacement::Auto,
    };
    parse_grid_area(s).unwrap_or_else(|| (auto(), auto()))
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

    /// `minmax()` had no branch of its own, so a track list holding one parsed to nothing and the
    /// whole template was dropped - the grid then fell back to a single implicit column. That is
    /// what put every ingewikkeld.dev "trusted by" logo on a row of its own, and every Tailwind
    /// `grid-cols-N`, which expands to exactly this, with it.
    #[test]
    fn minmax_tracks_parse() {
        assert_eq!(parse_grid_template("minmax(0, 1fr) minmax(0, 1fr)").unwrap().len(), 2);
        assert_eq!(parse_grid_template("repeat(7, minmax(0, 1fr))").unwrap().len(), 7);
        assert_eq!(parse_grid_template("minmax(100px, 1fr) auto").unwrap().len(), 2);
        assert_eq!(
            parse_grid_template("minmax(min-content, max-content)").unwrap().len(),
            1
        );
        assert_eq!(parse_grid_template("minmax(10%, 50%)").unwrap().len(), 1);
        // The unitless zero is the one CSS allows, and `minmax(0, 1fr)` is where it shows up.
        assert_eq!(parse_grid_template("minmax(0,1fr)").unwrap().len(), 1);
        // An `fr` is not a valid minimum (css-grid-1 `<inflexible-breadth>`).
        assert!(parse_grid_template("minmax(1fr, 1fr)").is_none());
        // A unitless number that is not zero is not a length.
        assert!(parse_grid_template("minmax(10, 1fr)").is_none());
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

#[cfg(test)]
mod grid_placement_tests {
    use super::{declared_grid_area, parse_single_placement, split_index_and_name};
    use taffy::prelude::TaffyGridLine;
    use taffy::GridPlacement;

    #[test]
    fn an_integer_qualified_name_picks_that_line() {
        assert_eq!(
            parse_single_placement("2 main-end"),
            GridPlacement::NamedLine("main-end".to_string(), 2)
        );
        // css-grid-2 writes the integer and the name in either order.
        assert_eq!(
            parse_single_placement("main-end 2"),
            GridPlacement::NamedLine("main-end".to_string(), 2)
        );
        assert_eq!(
            parse_single_placement("span 2 main"),
            GridPlacement::NamedSpan("main".to_string(), 2)
        );
    }

    #[test]
    fn the_plain_forms_are_unchanged() {
        assert_eq!(parse_single_placement("auto"), GridPlacement::Auto);
        assert_eq!(parse_single_placement("3"), GridPlacement::from_line_index(3));
        assert_eq!(
            parse_single_placement("content"),
            GridPlacement::NamedLine("content".to_string(), 1)
        );
        assert_eq!(parse_single_placement("span 2"), GridPlacement::Span(2));
    }

    /// A declared `grid-area: auto` resets both axes. Reading it as "says nothing" let an earlier
    /// `grid-row` survive a later reset.
    #[test]
    fn a_declared_grid_area_of_auto_resets_both_axes() {
        for reset in ["auto", "none", ""] {
            let (row, column) = declared_grid_area(reset);
            assert_eq!(row.start, GridPlacement::Auto, "{reset:?} row start");
            assert_eq!(row.end, GridPlacement::Auto, "{reset:?} row end");
            assert_eq!(column.start, GridPlacement::Auto, "{reset:?} column start");
            assert_eq!(column.end, GridPlacement::Auto, "{reset:?} column end");
        }

        // A real area still places the item across it.
        let (row, column) = declared_grid_area("content");
        assert_eq!(row.start, GridPlacement::NamedLine("content".to_string(), 1));
        assert_eq!(column.end, GridPlacement::NamedLine("content".to_string(), 1));
    }

    #[test]
    fn a_value_that_is_neither_is_still_auto() {
        assert_eq!(parse_single_placement("2 3"), GridPlacement::Auto);
        assert_eq!(parse_single_placement("a b"), GridPlacement::Auto);
        // A zero line index does not exist, so the value is not a named line either.
        assert_eq!(parse_single_placement("0 main"), GridPlacement::Auto);
        assert_eq!(parse_single_placement("2 main end"), GridPlacement::Auto);
        // A span counts forward, so a negative one is not a span - but a negative *line* index
        // counts from the end of the grid and is perfectly good.
        assert_eq!(parse_single_placement("span -2 main"), GridPlacement::Auto);
        assert_eq!(
            parse_single_placement("-2 main"),
            GridPlacement::NamedLine("main".to_string(), -2)
        );
        assert_eq!(split_index_and_name("2"), None);
    }
}
