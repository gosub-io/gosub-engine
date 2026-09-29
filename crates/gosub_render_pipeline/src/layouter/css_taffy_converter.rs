use crate::common::document::node::NodeId;
use crate::common::document::pipeline_doc::PipelineDocument;
use gosub_interface::style::{
    AlignValue, ComputedStyle, Display as CssDisplay, GridAreas, GridLine as CssGridLine,
    LengthPercentage as CssLengthPercentage, LengthPercentageAuto as CssLengthPercentageAuto, Overflow as CssOverflow,
    Position as CssPosition, Prop, RepeatCount, TextAlign as CssTextAlign, TrackBreadth, TrackList, TrackListItem,
    TrackSize,
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
        // The FlexStart default for align-content is applied at the end, once `ts.display` is final.
        ts.align_content = self.get_align_content(Prop::AlignContent, flex.align_content, ts.align_content);
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
        // The shorthands (`grid-row`, `grid-column`, `grid-area`) arrive expanded into these four,
        // in source order, with the omitted ends already filled in (css-grid-2 §8.4).
        ts.grid_row = Line {
            start: self.get_grid_line(Prop::GridRowStart, &grid.row_start, ts.grid_row.start),
            end: self.get_grid_line(Prop::GridRowEnd, &grid.row_end, ts.grid_row.end),
        };
        ts.grid_column = Line {
            start: self.get_grid_line(Prop::GridColumnStart, &grid.column_start, ts.grid_column.start),
            end: self.get_grid_line(Prop::GridColumnEnd, &grid.column_end, ts.grid_column.end),
        };

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

        // Default align-content to FlexStart rather than Taffy's None (= Stretch), but only where
        // it means anything. taffy 0.14 applies a non-`normal` `align-content` to block formatting
        // contexts too, and a block container carrying FlexStart stops a child's bottom margin
        // collapsing through it: WPT `margin-collapse-min-height-003` went from matching its
        // reference to drawing a 100x80 green box where a 100x30 one belongs, and every table
        // fixture grew 16px. On 0.12 block layout ignored the field, so the blanket default was
        // harmless; now it has to be confined to the layout modes that read it. This runs after
        // the display fixups above, so inline boxes (mapped to wrapping flex) get the default and
        // blockified floats (mapped to block) do not. An explicit value is left alone.
        if ts.align_content.is_none() && matches!(ts.display, Display::Flex | Display::Grid) {
            ts.align_content = Some(AlignContent::FLEX_START);
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
            // A `calc()` mixing a length and a percentage has no taffy form while the engine
            // uses taffy's own tree, whose calc resolver answers 0; it waits for layout to own its
            // tree (see `SendTaffyTree`). Until then it lays out as though it were not declared.
            CssLengthPercentageAuto::Calc { .. } => default,
        }
    }

    fn lp(&self, prop: Prop, value: CssLengthPercentage, default: LengthPercentage) -> LengthPercentage {
        if !self.style.has(prop) {
            return default;
        }
        Self::to_taffy_lp(value).unwrap_or(default)
    }

    /// Taffy's form of a length, or `None` for a `calc()` mixing a length and a percentage, which
    /// has none; see [`Self::lpa`].
    fn to_taffy_lp(value: CssLengthPercentage) -> Option<LengthPercentage> {
        match value {
            CssLengthPercentage::Px(px) => Some(LengthPercentage::length(px)),
            CssLengthPercentage::Percent(pct) => Some(LengthPercentage::percent(pct / 100.0)),
            CssLengthPercentage::Calc { .. } => None,
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
            // See `lpa`: no taffy form yet.
            CssLengthPercentageAuto::Calc { .. } => default,
        }
    }

    /// `row-gap` is the space between rows, so taffy's height; `column-gap` its width.
    fn get_gap(&self, default: Size<LengthPercentage>) -> Size<LengthPercentage> {
        let gap = |prop: Prop, value: CssLengthPercentage, default: LengthPercentage| {
            if !self.style.has(prop) {
                return default;
            }
            Self::to_taffy_lp(value).unwrap_or(default)
        };
        Size {
            width: gap(Prop::ColumnGap, self.style.flex.column_gap, default.width),
            height: gap(Prop::RowGap, self.style.flex.row_gap, default.height),
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

    /// Taffy's `text_align` places a block container's block-level children, which CSS
    /// `text-align` never does (CSS 2.1 §16.2: it aligns inline content). Only the `-webkit-`
    /// keywords the HTML rendering section gives `<center>` and the `align` attribute move
    /// blocks, and those are what taffy's legacy alignment is for. The line boxes read the
    /// computed value themselves. `text-align` inherits, so this reads the computed value rather
    /// than asking whether this element declared one.
    fn get_text_align(&self, default: TextAlign) -> TextAlign {
        match self.style.inherited.text_align {
            CssTextAlign::WebkitCenter => TextAlign::LegacyCenter,
            CssTextAlign::WebkitLeft => TextAlign::LegacyLeft,
            CssTextAlign::WebkitRight => TextAlign::LegacyRight,
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
        value: &TrackList,
        default: Vec<GridTemplateComponent<String>>,
    ) -> Vec<GridTemplateComponent<String>> {
        if !self.style.has(prop) {
            return default;
        }
        // An empty list is `none`.
        if value.is_empty() {
            return Vec::new();
        }
        match expand_tracks(value) {
            Some(tracks) if !tracks.is_empty() => tracks.into_iter().map(GridTemplateComponent::Single).collect(),
            _ => default,
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

    fn get_grid_line(&self, prop: Prop, value: &CssGridLine, default: GridPlacement) -> GridPlacement {
        if !self.style.has(prop) {
            return default;
        }
        match value {
            CssGridLine::Auto => GridPlacement::Auto,
            CssGridLine::Line(index) => GridPlacement::from_line_index(*index),
            CssGridLine::Span(count) => span(*count),
            CssGridLine::Named(name, index) => GridPlacement::NamedLine(name.to_string(), *index),
            CssGridLine::NamedSpan(name, count) => GridPlacement::NamedSpan(name.to_string(), *count),
        }
    }

    /// `grid-template-areas`, as the rectangle each area name covers.
    fn get_grid_areas(&self, default: Option<GridTemplateAreas<String>>) -> Option<GridTemplateAreas<String>> {
        if !self.style.has(Prop::GridTemplateAreas) {
            return default;
        }
        let rows = &self.style.grid.template_areas;
        // taffy 0.14 wants the template's shape alongside the areas: the rows that hold a cell,
        // and the widest of them. An empty template is `none`.
        let row_count = rows.iter().filter(|row| !row.is_empty()).count();
        let column_count = rows.iter().map(|row| row.len()).max().unwrap_or(0);
        if row_count == 0 || column_count == 0 {
            return None;
        }
        // The shape is kept even when nothing in it is named. `". ." ". ."` declares a 2x2
        // explicit grid and no areas at all, and taffy sizes the explicit grid from these counts,
        // so dropping it would move every auto-placed item.
        Some(GridTemplateAreas {
            areas: grid_areas(rows).into_iter().collect(),
            row_count: row_count as u16,
            column_count: column_count as u16,
        })
    }

    fn get_grid_auto(
        &self,
        prop: Prop,
        value: &TrackList,
        default: Vec<TrackSizingFunction>,
    ) -> Vec<TrackSizingFunction> {
        if !self.style.has(prop) {
            return default;
        }
        // An empty list is the initial `auto`.
        if value.is_empty() {
            return Vec::new();
        }
        expand_tracks(value)
            .filter(|tracks| !tracks.is_empty())
            .unwrap_or(default)
    }
}

/// A track length. A `calc()` has no value until taffy can resolve one (see the calc note on
/// `LengthPercentage::Calc`), so a track list holding one falls back as a whole.
fn track_length(length: CssLengthPercentage) -> Option<LengthPercentage> {
    match length {
        CssLengthPercentage::Px(px) => Some(LengthPercentage::length(px)),
        CssLengthPercentage::Percent(pct) => Some(LengthPercentage::percent(pct / 100.0)),
        CssLengthPercentage::Calc { .. } => None,
    }
}

/// The minimum of a track size. css-grid-1 calls it `<inflexible-breadth>`: never an `fr`.
fn min_track(breadth: TrackBreadth) -> Option<MinTrackSizingFunction> {
    match breadth {
        TrackBreadth::Length(length) => track_length(length).map(MinTrackSizingFunction::from),
        TrackBreadth::Fr(_) => None,
        TrackBreadth::Auto => Some(MinTrackSizingFunction::AUTO),
        TrackBreadth::MinContent => Some(MinTrackSizingFunction::MIN_CONTENT),
        TrackBreadth::MaxContent => Some(MinTrackSizingFunction::MAX_CONTENT),
    }
}

/// The maximum of a track size: everything a minimum accepts, plus `fr`.
fn max_track(breadth: TrackBreadth) -> Option<MaxTrackSizingFunction> {
    match breadth {
        TrackBreadth::Fr(fr) => Some(MaxTrackSizingFunction::from_fr(fr)),
        TrackBreadth::Length(length) => track_length(length).map(MaxTrackSizingFunction::from),
        TrackBreadth::Auto => Some(MaxTrackSizingFunction::AUTO),
        TrackBreadth::MinContent => Some(MaxTrackSizingFunction::MIN_CONTENT),
        TrackBreadth::MaxContent => Some(MaxTrackSizingFunction::MAX_CONTENT),
    }
}

/// One track size. A bare `fr` is a maximum with a zero minimum; every other single value is
/// both sides at once. `fit-content()` is not mapped yet and fails the list.
fn track_size(size: &TrackSize) -> Option<TrackSizingFunction> {
    match *size {
        TrackSize::Single(TrackBreadth::Fr(fr)) => Some(minmax(
            MinTrackSizingFunction::ZERO,
            MaxTrackSizingFunction::from_fr(fr),
        )),
        TrackSize::Single(breadth) => Some(minmax(min_track(breadth)?, max_track(breadth)?)),
        TrackSize::MinMax(min, max) => Some(minmax(min_track(min)?, max_track(max)?)),
        TrackSize::FitContent(_) => None,
    }
}

/// A track list as the flat tracks taffy takes. Line names are dropped and a fixed `repeat()` is
/// expanded; `auto-fill` / `auto-fit` are not supported yet, and `None` - anything this cannot
/// map - makes the caller keep its default rather than mis-render.
fn expand_tracks(items: &[TrackListItem]) -> Option<Vec<TrackSizingFunction>> {
    let mut tracks = Vec::new();
    for item in items {
        match item {
            TrackListItem::LineNames(_) => {}
            TrackListItem::Track(size) => tracks.push(track_size(size)?),
            TrackListItem::Repeat(RepeatCount::Count(count), inner) => {
                let inner = expand_tracks(inner)?;
                if inner.is_empty() {
                    return None;
                }
                for _ in 0..*count {
                    tracks.extend(inner.iter().cloned());
                }
            }
            TrackListItem::Repeat(RepeatCount::AutoFill | RepeatCount::AutoFit, _) => return None,
        }
    }
    Some(tracks)
}

/// `grid-template-areas` as the rectangle each name covers, in taffy's 1-based grid line
/// coordinates.
///
/// A null cell names no area. A name that does not form a rectangle is not rejected the way
/// css-grid-2 requires; it gets its bounding box, which keeps a typo from dropping the whole
/// shell.
fn grid_areas(rows: &GridAreas) -> Vec<GridTemplateArea<String>> {
    // Preserve document order so a page's areas keep a stable order in the output.
    let mut order: Vec<&str> = Vec::new();
    let mut bounds: std::collections::HashMap<&str, (u16, u16, u16, u16)> = std::collections::HashMap::new();

    for (row, cells) in rows.iter().enumerate() {
        for (column, cell) in cells.iter().enumerate() {
            let Some(cell) = cell.as_deref() else {
                continue;
            };
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
    use super::grid_areas;
    use std::sync::Arc;
    use taffy::GridTemplateArea;

    /// Rows as the computed stage builds them: whitespace-separated cells, dots for a null cell.
    fn parse_grid_areas(source: &str) -> Vec<GridTemplateArea<String>> {
        let rows: Vec<Arc<[Option<Arc<str>>]>> = source
            .lines()
            .map(|row| {
                row.split_whitespace()
                    .map(|cell| (!cell.chars().all(|c| c == '.')).then(|| Arc::from(cell)))
                    .collect()
            })
            .collect();
        grid_areas(&Arc::from(rows))
    }

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
}

#[cfg(test)]
mod grid_template_tests {
    use super::expand_tracks;
    use gosub_interface::style::{
        LengthPercentage, RepeatCount, TrackBreadth, TrackBreadth::Fr, TrackListItem, TrackSize,
    };
    use std::sync::Arc;

    fn track(breadth: TrackBreadth) -> TrackListItem {
        TrackListItem::Track(TrackSize::Single(breadth))
    }

    fn minmax(min: TrackBreadth, max: TrackBreadth) -> TrackListItem {
        TrackListItem::Track(TrackSize::MinMax(min, max))
    }

    fn repeat(count: RepeatCount, items: Vec<TrackListItem>) -> TrackListItem {
        TrackListItem::Repeat(count, Arc::from(items))
    }

    fn count(items: Vec<TrackListItem>) -> Option<usize> {
        expand_tracks(&items).map(|tracks| tracks.len())
    }

    const ZERO: TrackBreadth = TrackBreadth::Length(LengthPercentage::Px(0.0));

    #[test]
    fn expands_repeat_and_drops_line_names() {
        assert_eq!(
            count(vec![repeat(RepeatCount::Count(3), vec![track(Fr(1.0))])]),
            Some(3)
        );
        assert_eq!(
            count(vec![repeat(
                RepeatCount::Count(2),
                vec![track(Fr(1.0)), track(Fr(2.0))]
            )]),
            Some(4)
        );
        assert_eq!(
            count(vec![
                TrackListItem::LineNames(Arc::from([Arc::from("a")])),
                repeat(RepeatCount::Count(2), vec![track(Fr(1.0))]),
                track(TrackBreadth::Length(LengthPercentage::Px(200.0))),
            ]),
            Some(3)
        );
    }

    /// Every Tailwind `grid-cols-N` is `repeat(N, minmax(0, 1fr))`; a template that failed on it
    /// put each item on a row of its own.
    #[test]
    fn minmax_tracks_map() {
        assert_eq!(
            count(vec![repeat(RepeatCount::Count(7), vec![minmax(ZERO, Fr(1.0))])]),
            Some(7)
        );
        assert_eq!(
            count(vec![minmax(TrackBreadth::MinContent, TrackBreadth::MaxContent)]),
            Some(1)
        );
        // An `fr` is not a valid minimum (css-grid-1 `<inflexible-breadth>`).
        assert_eq!(count(vec![minmax(Fr(1.0), Fr(1.0))]), None);
    }

    /// What is not mapped yet fails the whole list, so the caller keeps its default instead of
    /// laying out part of a template.
    #[test]
    fn unsupported_tracks_fail_the_list() {
        assert_eq!(count(vec![repeat(RepeatCount::AutoFill, vec![track(Fr(1.0))])]), None);
        assert_eq!(
            count(vec![TrackListItem::Track(TrackSize::FitContent(LengthPercentage::Px(
                100.0
            )))]),
            None
        );
        assert_eq!(
            count(vec![track(TrackBreadth::Length(LengthPercentage::Calc {
                px: 10.0,
                percent: 50.0
            }))]),
            None
        );
    }
}
