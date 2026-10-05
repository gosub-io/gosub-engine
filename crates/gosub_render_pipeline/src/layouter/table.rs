use gosub_lattice::{BoxEdges, CellLayout, CssLength, CssProp, TableRole, TableTree, VerticalAlign};

use crate::common::document::node::{NodeId as DomNodeId, NodeType};
use crate::common::document::pipeline_doc::PipelineDocument;
use crate::common::geo::{Coordinate, Rect};
use crate::layouter::box_model::{BoxModel, Edges};
use crate::layouter::float::float_side;
use crate::layouter::taffy::{TaffyLayouter, MAX_LAYOUT_DEPTH};
use crate::layouter::{CollapsedCellBorders, ElementContext, LayoutElementId, LayoutElementNode, LayoutTree};
use gosub_interface::style::{Display, LengthPercentage, LengthPercentageAuto, Position, Prop};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Adapter that bridges `gosub_lattice`'s `TableTree` with the render pipeline's
/// `LayoutTree`/`PipelineDocument`. Layout results are staged in `pending` and
/// converted to absolute `BoxModel`s by `apply_positions()` after
/// `compute_table_layout` returns.
pub struct PipelineTableTree<'a> {
    doc: &'a dyn PipelineDocument,
    layouter: &'a mut TaffyLayouter,
    layout_tree: &'a mut LayoutTree,
    dom_to_layout: &'a HashMap<DomNodeId, LayoutElementId>,
    /// Relative CellLayouts written by `compute_table_layout`.
    pending: HashMap<DomNodeId, CellLayout>,
    /// Per collapsed cell, the node whose CSS border style paints each edge
    /// (`[top, right, bottom, left]`; `None` = the cell's own border). Only
    /// populated for cells of `border-collapse` tables.
    edge_owners: HashMap<DomNodeId, [Option<DomNodeId>; 4]>,
    /// Cells whose subtree was re-laid-out via `relayout_cell` this pass. Only
    /// these get the `content_offset_y` vertical-align shift: their children
    /// are freshly anchored at the cell top, so the shift applies exactly once.
    relaid: HashSet<DomNodeId>,
    /// Memo for `subtree_contains_table`: `layout_cell` asks per cell per pass, and each
    /// walk re-materializes anonymous wrappers, so uncached it is superlinear in table size.
    contains_table_cache: RefCell<HashMap<DomNodeId, bool>>,
}

impl<'a> PipelineTableTree<'a> {
    pub fn new(
        doc: &'a dyn PipelineDocument,
        layouter: &'a mut TaffyLayouter,
        layout_tree: &'a mut LayoutTree,
        dom_to_layout: &'a HashMap<DomNodeId, LayoutElementId>,
    ) -> Self {
        Self {
            doc,
            layouter,
            layout_tree,
            dom_to_layout,
            pending: HashMap::new(),
            edge_owners: HashMap::new(),
            relaid: HashSet::new(),
            contains_table_cache: RefCell::new(HashMap::new()),
        }
    }

    /// True when the DOM subtree under `id` contains a `display: table` node.
    /// Such cells keep the first-pass height approximation: re-running taffy on
    /// them would clobber the box models lattice computed for the inner table.
    fn subtree_contains_table(&self, id: DomNodeId) -> bool {
        self.subtree_contains_table_bounded(id, 0).0
    }

    /// `(contains a table, gave up on depth)`.
    ///
    /// This walks the DOM, not the layout tree, so the layout depth cap does not bound it on its
    /// own - the same hole `collect_tables_preorder` closes, and a cell holding 20,000 nested
    /// elements reached it here. It stops at [`MAX_LAYOUT_DEPTH`] for the same reason: every
    /// level of the DOM path to a box is a level of the layout tree too, so a table deeper than
    /// the cap has no layout box, and the caller's nested-table branch would find no height for
    /// it either.
    ///
    /// A truncated answer is *not* cached. The bound is on depth below the node being asked
    /// about, so the same node can be reached at different depths from different cells; caching
    /// a `false` that only means "did not look far enough" would hand it to a shallower caller
    /// that would have looked deep enough.
    fn subtree_contains_table_bounded(&self, id: DomNodeId, depth: usize) -> (bool, bool) {
        if let Some(&hit) = self.contains_table_cache.borrow().get(&id) {
            return (hit, false);
        }
        if depth >= MAX_LAYOUT_DEPTH {
            return (false, true);
        }

        let mut hit = false;
        let mut truncated = false;
        for &child in self.doc.children(id).iter() {
            if is_table_box(self.doc, child) {
                hit = true;
                break;
            }
            let (child_hit, child_truncated) = self.subtree_contains_table_bounded(child, depth + 1);
            truncated |= child_truncated;
            if child_hit {
                hit = true;
                break;
            }
        }

        if !truncated {
            self.contains_table_cache.borrow_mut().insert(id, hit);
        }
        (hit, truncated)
    }

    /// Sum of the border-box heights of the nested tables directly contained in a cell (not
    /// counting tables nested deeper inside those). Zero if the cell holds no table. This lets
    /// a table cell grow to contain a nested table whose height lattice computes in a later pass.
    /// One side of a table's padding as its layout box has it, for the padding properties of a
    /// table node; `None` for anything else.
    fn resolved_table_padding(&self, id: DomNodeId, prop: CssProp) -> Option<f64> {
        if !matches!(
            prop,
            CssProp::PaddingTop | CssProp::PaddingRight | CssProp::PaddingBottom | CssProp::PaddingLeft
        ) || !is_table_box(self.doc, id)
        {
            return None;
        }
        let padding = self
            .dom_to_layout
            .get(&id)
            .and_then(|layout_id| self.layout_tree.arena.get(layout_id))?
            .box_model
            .padding;
        Some(match prop {
            CssProp::PaddingTop => padding.top,
            CssProp::PaddingRight => padding.right,
            CssProp::PaddingBottom => padding.bottom,
            _ => padding.left,
        })
    }

    fn nested_table_height(&self, cell_layout_id: LayoutElementId) -> f32 {
        let Some(el) = self.layout_tree.arena.get(&cell_layout_id) else {
            return 0.0;
        };
        let mut total = 0.0;
        for &child_id in &el.children {
            let Some(child) = self.layout_tree.arena.get(&child_id) else {
                continue;
            };
            let is_table = is_table_box(self.doc, child.dom_node_id);
            if is_table {
                // Self-contained nested table - stop here, don't double-count its inner tables.
                // MARGIN box: the table's margins are part of the content extent it occupies
                // in the cell (negative margins can collapse it out entirely).
                total += child.box_model.margin_box.height.max(0.0) as f32;
            } else {
                // The table may be wrapped (e.g. in an anonymous box); keep descending.
                total += self.nested_table_height(child_id);
            }
        }
        total
    }

    /// Convert pending relative positions to absolute `BoxModel`s in the arena.
    /// Must be called after `compute_table_layout` returns.
    pub fn apply_positions(&mut self, table_dom_id: DomNodeId, border_corrected: &mut HashSet<DomNodeId>) {
        // Under border-collapse the grid (incl. the perimeter border halves) starts at
        // the table's BORDER box origin - the table's own border joined the conflict
        // inside lattice and no longer insets the content. The box model read here is
        // still the taffy first-pass one, whose border/padding would inset wrongly.
        let collapse = borders_collapse(self.doc, table_dom_id);
        let table_abs = self
            .dom_to_layout
            .get(&table_dom_id)
            .and_then(|id| self.layout_tree.arena.get(id))
            .map(|e| {
                let b = if collapse {
                    e.box_model.border_box
                } else {
                    e.box_model.content_box
                };
                Coordinate::new(b.x, b.y)
            })
            .unwrap_or(Coordinate::ZERO);

        let pending = std::mem::take(&mut self.pending);
        apply_recursive(
            self.doc,
            table_dom_id,
            table_abs,
            Coordinate::ZERO,
            &pending,
            &self.edge_owners,
            &self.relaid,
            border_corrected,
            self.dom_to_layout,
            &mut self.layout_tree.arena,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn apply_recursive(
    doc: &dyn PipelineDocument,
    id: DomNodeId,
    parent_abs: Coordinate,
    // Translation to apply to non-pending children. For nodes inside a
    // lattice-repositioned cell this is (new_cell_abs - old_cell_abs), plus
    // the cell's vertical-align shift when its subtree was re-anchored.
    offset: Coordinate,
    pending: &HashMap<DomNodeId, CellLayout>,
    edge_owners: &HashMap<DomNodeId, [Option<DomNodeId>; 4]>,
    relaid: &HashSet<DomNodeId>,
    // Cells whose skipped-relayout subtree already received the raw-vs-collapsed border
    // shift; the correction is a one-time conversion, not a per-pass translation.
    border_corrected: &mut HashSet<DomNodeId>,
    dom_to_layout: &HashMap<DomNodeId, LayoutElementId>,
    arena: &mut HashMap<LayoutElementId, LayoutElementNode>,
) {
    // Iterative, because this walks the DOM rather than the layout tree and so is not bounded by
    // the layout depth cap: a cell holding 20,000 nested elements overflowed the stack here, and
    // on a successful table layout the subtree is walked a second time. Unlike the other DOM
    // walks in this file it cannot simply stop at the cap - a pending cell can sit at a shallow
    // depth underneath a deep subtree, and its position still has to be applied.
    //
    // Children are pushed in reverse so they pop in document order, which keeps the traversal -
    // and every box it writes - identical to the recursion this replaces.
    let mut stack = vec![(id, parent_abs, offset)];
    while let Some((id, parent_abs, offset)) = stack.pop() {
        for child_id in doc.children(id).into_iter().rev() {
            match pending.get(&child_id) {
                None => {
                    // Non-table-structure node: shift it by the accumulated translation
                    // so it stays correctly positioned relative to its parent cell.
                    if let Some(&layout_id) = dom_to_layout.get(&child_id) {
                        if let Some(element) = arena.get_mut(&layout_id) {
                            translate_box_model(&mut element.box_model, offset);
                        }
                    }
                    stack.push((child_id, parent_abs, offset));
                }
                Some(cell_layout) => {
                    let abs = Coordinate::new(
                        parent_abs.x + cell_layout.position.x,
                        parent_abs.y + cell_layout.position.y,
                    );
                    // Read old position before overwriting so we can compute the
                    // translation needed for non-pending children of this cell.
                    let old_abs = dom_to_layout
                        .get(&child_id)
                        .and_then(|&lid| arena.get(&lid))
                        .map(|el| Coordinate::new(el.box_model.border_box.x, el.box_model.border_box.y))
                        .unwrap_or(abs);
                    if let Some(&layout_id) = dom_to_layout.get(&child_id) {
                        if let Some(element) = arena.get_mut(&layout_id) {
                            element.box_model = cell_layout_to_box_model(cell_layout, abs);
                            element.collapsed_borders =
                                edge_owners.get(&child_id).map(|&owners| CollapsedCellBorders {
                                    widths: [
                                        cell_layout.border.top as f32,
                                        cell_layout.border.right as f32,
                                        cell_layout.border.bottom as f32,
                                        cell_layout.border.left as f32,
                                    ],
                                    outsets: cell_layout.border_outsets.map(|v| v as f32),
                                    owners,
                                });
                        }
                    }
                    // vertical-align: only cells whose subtree was re-anchored at the
                    // cell top this pass get the shift, so it applies exactly once.
                    let valign_shift = if relaid.contains(&child_id) {
                        cell_layout.content_offset_y
                    } else {
                        0.0
                    };
                    // Skipped-relayout collapsed cells (nested-table guard): their children were
                    // positioned by the FIRST taffy pass with the raw CSS border widths, but the
                    // collapse geometry replaced those with half the resolved boundary. Shift the
                    // subtree by the difference so content sits at the collapsed content origin.
                    // Rows are in `pending` too but never collapsed cells; `edge_owners` is the
                    // exact "collapsed cell" predicate.
                    let border_delta = if !relaid.contains(&child_id)
                        && edge_owners.contains_key(&child_id)
                        && border_corrected.insert(child_id)
                    {
                        let child_border = &doc.computed_style(child_id).border;
                        let raw_left = f64::from(child_border.left_width);
                        let raw_top = f64::from(child_border.top_width);
                        let dl = cell_layout.border.left - raw_left;
                        let dt = cell_layout.border.top - raw_top;
                        if dl != 0.0 || dt != 0.0 {
                            Coordinate::new(dl, dt)
                        } else {
                            Coordinate::ZERO
                        }
                    } else {
                        Coordinate::ZERO
                    };
                    let child_offset = Coordinate::new(
                        abs.x - old_abs.x + border_delta.x,
                        abs.y - old_abs.y + valign_shift + border_delta.y,
                    );
                    stack.push((child_id, abs, child_offset));
                }
            }
        }
    }
}

#[cfg(test)]
thread_local! {
    /// How many boxes the table passes moved on this thread. Tests use it to check that the
    /// work grows with the size of the page and not with its square.
    pub(crate) static TRANSLATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn translate_box_model(bm: &mut BoxModel, offset: Coordinate) {
    if offset.x == 0.0 && offset.y == 0.0 {
        return;
    }
    #[cfg(test)]
    TRANSLATIONS.with(|n| n.set(n.get() + 1));
    bm.border_box.x += offset.x;
    bm.border_box.y += offset.y;
    bm.padding_box.x += offset.x;
    bm.padding_box.y += offset.y;
    bm.content_box.x += offset.x;
    bm.content_box.y += offset.y;
    bm.margin_box.x += offset.x;
    bm.margin_box.y += offset.y;
}

fn cell_layout_to_box_model(layout: &CellLayout, abs: Coordinate) -> BoxModel {
    let border_box = Rect::new(abs.x, abs.y, layout.size.width, layout.size.height);
    BoxModel::new(
        border_box,
        Edges {
            top: layout.padding.top,
            right: layout.padding.right,
            bottom: layout.padding.bottom,
            left: layout.padding.left,
        },
        Edges {
            top: layout.border.top,
            right: layout.border.right,
            bottom: layout.border.bottom,
            left: layout.border.left,
        },
        Edges {
            top: 0.0,
            right: 0.0,
            bottom: 0.0,
            left: 0.0,
        },
    )
}

impl TableTree for PipelineTableTree<'_> {
    type NodeId = DomNodeId;

    /// The DOM children, less what generates no box among the table's parts.
    ///
    /// Every child of a table, row group or row that is not itself a table part is laid out as
    /// an anonymous cell, so the indentation between rows, a comment or a hidden `<input>` would
    /// each make a row of their own. CSS 2 §17.2.1 treats them as `display: none`. A cell's
    /// children are its content and are left alone.
    fn children(&self, id: DomNodeId) -> Vec<DomNodeId> {
        let children = self.doc.children(id);
        match self.table_role(id) {
            TableRole::Table
            | TableRole::RowGroup
            | TableRole::HeaderGroup
            | TableRole::FooterGroup
            | TableRole::Row => children
                .into_iter()
                .filter(|&child| !self.doc.generates_no_table_box(child))
                .collect(),
            _ => children,
        }
    }

    fn table_role(&self, id: DomNodeId) -> TableRole {
        let style = self.doc.computed_style(id);
        if !style.has(Prop::Display) {
            return TableRole::Other;
        }
        match style.box_group.display {
            Display::Table | Display::InlineTable => TableRole::Table,
            Display::TableCaption => TableRole::Caption,
            Display::TableColumnGroup => TableRole::ColumnGroup,
            Display::TableColumn => TableRole::Column,
            Display::TableRowGroup => TableRole::RowGroup,
            Display::TableHeaderGroup => TableRole::HeaderGroup,
            Display::TableFooterGroup => TableRole::FooterGroup,
            Display::TableRow => TableRole::Row,
            Display::TableCell => TableRole::Cell,
            _ => TableRole::Other,
        }
    }

    fn css_length(&self, id: DomNodeId, prop: CssProp) -> CssLength {
        // The table's own padding is what the grid gets wrapped in afterwards, and that comes
        // from the table's box as taffy resolved it. Lattice places the grid and the caption
        // inside and outside of it, so it is handed the same resolved pixels rather than a
        // percentage to resolve on its own and maybe differently.
        if let Some(side) = self.resolved_table_padding(id, prop) {
            return CssLength::Px(side);
        }
        let style = self.doc.computed_style(id);
        let (size, border, padding) = (&style.size, &style.border, &style.padding);
        // A `calc()` mixing a length and a percentage is laid out as though it were not declared,
        // as it is by taffy (see `CssTaffyConverter::lpa`): the lengths read here are paddings,
        // unset at 0, and sizes, unset at `auto`.
        let length = |value: LengthPercentage| match value {
            LengthPercentage::Px(px) => CssLength::Px(f64::from(px)),
            LengthPercentage::Percent(pct) => CssLength::Percent(f64::from(pct)),
            LengthPercentage::Calc { .. } => CssLength::Px(0.0),
        };
        let length_auto = |value: LengthPercentageAuto| match value {
            LengthPercentageAuto::Px(px) => CssLength::Px(f64::from(px)),
            LengthPercentageAuto::Percent(pct) => CssLength::Percent(f64::from(pct)),
            LengthPercentageAuto::Auto | LengthPercentageAuto::Calc { .. } => CssLength::Auto,
        };
        match prop {
            CssProp::Width => length_auto(size.width),
            CssProp::Height => length_auto(size.height),
            CssProp::MinWidth => length_auto(size.min_width),
            CssProp::MinHeight => length_auto(size.min_height),
            CssProp::MaxWidth => length_auto(size.max_width),
            CssProp::MaxHeight => length_auto(size.max_height),
            CssProp::BorderTopWidth => CssLength::Px(f64::from(border.top_width)),
            CssProp::BorderRightWidth => CssLength::Px(f64::from(border.right_width)),
            CssProp::BorderBottomWidth => CssLength::Px(f64::from(border.bottom_width)),
            CssProp::BorderLeftWidth => CssLength::Px(f64::from(border.left_width)),
            CssProp::PaddingTop => length(padding.top),
            CssProp::PaddingRight => length(padding.right),
            CssProp::PaddingBottom => length(padding.bottom),
            CssProp::PaddingLeft => length(padding.left),
            // border-spacing is inherited, so the computed value carries the cascade down to
            // the user-agent default (`table { border-spacing: 2px }`).
            CssProp::BorderSpacingX => CssLength::Px(f64::from(style.inherited.border_spacing_x)),
            CssProp::BorderSpacingY => CssLength::Px(f64::from(style.inherited.border_spacing_y)),
            // Px(1.0) is the lattice sentinel for `table-layout: fixed`.
            CssProp::TableLayout => match style.box_group.table_layout {
                gosub_interface::style::TableLayout::Fixed => CssLength::Px(1.0),
                gosub_interface::style::TableLayout::Auto => CssLength::Auto,
            },
            // Px(1.0) = `border-collapse: collapse` (inherited, so it walks up).
            CssProp::BorderCollapse => match style.inherited.border_collapse {
                gosub_interface::style::BorderCollapse::Collapse => CssLength::Px(1.0),
                gosub_interface::style::BorderCollapse::Separate => CssLength::Auto,
            },
            // Px(1.0) = `caption-side: bottom`.
            CssProp::CaptionSide => match style.inherited.caption_side {
                gosub_interface::style::CaptionSide::Bottom => CssLength::Px(1.0),
                gosub_interface::style::CaptionSide::Top => CssLength::Auto,
            },
            // Px(1.0) = `box-sizing: border-box`.
            CssProp::BoxSizing => match style.box_group.box_sizing {
                gosub_interface::style::BoxSizing::BorderBox => CssLength::Px(1.0),
                gosub_interface::style::BoxSizing::ContentBox => CssLength::Auto,
            },
            // Resolved by the dedicated trait method, not css_length.
            CssProp::VerticalAlign => CssLength::Auto,
        }
    }

    fn attr_usize(&self, id: DomNodeId, attr: &str) -> Option<usize> {
        let node = self.doc.get_node_by_id(id)?;
        match &node.node_type {
            NodeType::Element(data) => data.attributes.get(attr)?.parse::<usize>().ok(),
            _ => None,
        }
    }

    fn set_layout(&mut self, id: DomNodeId, layout: CellLayout) {
        self.pending.insert(id, layout);
    }

    fn set_collapsed_cell_borders(&mut self, id: DomNodeId, layout: BoxEdges, edge_owners: [Option<DomNodeId>; 4]) {
        self.edge_owners.insert(id, edge_owners);
        if let Some(&layout_id) = self.dom_to_layout.get(&id) {
            self.layouter.set_cell_borders(layout_id, layout);
        }
    }

    fn layout_cell(&mut self, id: DomNodeId, available_width: f64) -> f64 {
        let Some(&layout_id) = self.dom_to_layout.get(&id) else {
            return 0.0;
        };

        // Cells hosting a nested table re-use the first-pass height instead of
        // re-laying-out: the nested table's real height is only known after
        // lattice lays it out, and the second (bottom-up) pass in
        // `post_process_tables` propagates it up here.
        if self.subtree_contains_table(id) {
            if let Some(element) = self.layout_tree.arena.get(&layout_id) {
                let taffy_h = element.box_model.content_box.height;
                return taffy_h.max(f64::from(self.nested_table_height(layout_id)));
            }
            return 0.0;
        }

        // Re-run taffy on the cell subtree at the lattice column width so the
        // content (wrapping, alignment, stacked blocks) is laid out against the
        // real cell geometry instead of the first pass's equal-share width.
        // `available_width` is the inner (content) width; taffy sizes the cell's
        // border box, so add the cell's own border and padding back.
        let extras = self
            .layout_tree
            .arena
            .get(&layout_id)
            .map(|el| {
                el.box_model.border.left
                    + el.box_model.border.right
                    + el.box_model.padding.left
                    + el.box_model.padding.right
            })
            .unwrap_or(0.0);
        if let Some(content_h) =
            self.layouter
                .relayout_cell(self.layout_tree, layout_id, (available_width + extras) as f32)
        {
            self.relaid.insert(id);
            return f64::from(content_h);
        }

        // Fallback: the content height from the taffy first pass.
        self.layout_tree
            .arena
            .get(&layout_id)
            .map(|el| el.box_model.content_box.height)
            .unwrap_or(0.0)
    }

    /// Resolves `vertical-align` for a cell by walking up to the table: the
    /// HTML rendering spec puts `vertical-align: inherit` on cells and
    /// `middle` on rows/sections, so the browser default falls out of the walk.
    /// `baseline` (and the inline-only keywords) approximate as Top.
    fn vertical_align(&self, id: DomNodeId) -> VerticalAlign {
        use gosub_interface::style::VerticalAlign as CssVerticalAlign;
        let mut cur = Some(id);
        while let Some(node) = cur {
            let style = self.doc.computed_style(node);
            if style.has(Prop::VerticalAlign) {
                match style.box_group.vertical_align {
                    CssVerticalAlign::Top => return VerticalAlign::Top,
                    CssVerticalAlign::Middle => return VerticalAlign::Middle,
                    CssVerticalAlign::Bottom => return VerticalAlign::Bottom,
                    // CSS 2 §17.5.3: cell values other than top/middle/bottom behave
                    // as baseline.
                    CssVerticalAlign::Baseline
                    | CssVerticalAlign::TextTop
                    | CssVerticalAlign::TextBottom
                    | CssVerticalAlign::Sub
                    | CssVerticalAlign::Super => return VerticalAlign::Baseline,
                    // The `inherit` the user-agent sheet puts on cells, or a length: keep walking.
                    CssVerticalAlign::Other => {}
                }
            }
            if self.table_role(node) == TableRole::Table {
                break;
            }
            cur = self.doc.parent(node);
        }
        VerticalAlign::Top
    }

    fn cell_baseline(&mut self, id: DomNodeId) -> Option<f64> {
        let &layout_id = self.dom_to_layout.get(&id)?;
        let cell_top = self.layout_tree.arena.get(&layout_id)?.box_model.border_box.y;

        // First text element in the cell's subtree, in tree order = the first in-flow
        // line box (nested tables excluded - their baselines don't propagate here).
        fn first_text(tree: &LayoutTree, id: LayoutElementId, doc: &dyn PipelineDocument) -> Option<LayoutElementId> {
            let el = tree.arena.get(&id)?;
            if matches!(el.context, ElementContext::Text(_)) {
                return Some(id);
            }
            if is_table_box(doc, el.dom_node_id) {
                return None;
            }
            for &c in &el.children {
                if let Some(hit) = first_text(tree, c, doc) {
                    return Some(hit);
                }
            }
            None
        }
        let text_id = first_text(self.layout_tree, layout_id, self.doc)?;
        let text_el = self.layout_tree.arena.get(&text_id)?;
        let ElementContext::Text(ref ctx) = text_el.context else {
            return None;
        };
        let ascent = self.layouter.first_line_ascent(&ctx.text, &ctx.font_info)?;
        Some((text_el.box_model.content_box.y - cell_top) + f64::from(ascent))
    }

    fn cell_intrinsic_widths(&mut self, id: DomNodeId) -> (f64, f64) {
        let Some(&layout_id) = self.dom_to_layout.get(&id) else {
            if std::env::var("LATTICE_DEBUG").is_ok() {
                eprintln!("lattice-dbg: cell {:?} NOT in dom_to_layout", id);
            }
            return (0.0, 0.0);
        };
        // Max-content is taffy's answer at unlimited width. Min-content is the widest
        // unbreakable run under the cell, measured from the laid-out boxes: taffy's own
        // min-content pass cannot break between the items of an inline-block (a non-wrapping
        // flex row here) and reported Wikipedia's comma-separated infobox lists as one
        // unbreakable 1100px run. Both are border-box widths, so the walk - which only sees
        // content - gets the cell's own padding and border added back.
        let max = f64::from(self.layouter.measure_max_content_width(layout_id).unwrap_or(0.0));
        let extras = self
            .layout_tree
            .arena
            .get(&layout_id)
            .map(|el| {
                el.box_model.border.left
                    + el.box_model.border.right
                    + el.box_model.padding.left
                    + el.box_model.padding.right
            })
            .unwrap_or(0.0);
        let min = f64::from(subtree_min_content_width(
            self.doc,
            self.layout_tree,
            self.layouter,
            layout_id,
            true,
        )) + extras;
        let w = (min, max.max(min));
        if std::env::var("LATTICE_DEBUG").is_ok() {
            eprintln!("lattice-dbg: cell {:?} intrinsics={:?}", id, w);
        }
        w
    }

    // Taffy genuinely measures: all-zero intrinsics mean truly empty cells, which
    // shrink to fit rather than triggering the mock-tree fill-available fallback.
    fn measures_intrinsics(&self) -> bool {
        true
    }
}

/// Widest unbreakable run of content in a layout subtree - its min-content width.
///
/// Text contributes its longest word, measured unconstrained: that is the narrowest a text box
/// can be without the shaper breaking inside a word. Under `white-space: nowrap` the whole run
/// is unbreakable. A replaced element contributes its whole border-box width, since an image has
/// no break opportunities at all, and so does a box the author gave an explicit px width. The
/// laid-out boxes cannot answer this on their own - a text box carries the width it was
/// allotted, which may be anything from one word to the whole run - so the words are re-measured
/// through the same font system the layouter used. `root` is the cell itself, whose own box
/// never counts (it is what is being sized).
fn subtree_min_content_width(
    doc: &dyn PipelineDocument,
    layout_tree: &LayoutTree,
    layouter: &mut TaffyLayouter,
    id: LayoutElementId,
    root: bool,
) -> f32 {
    let Some(el) = layout_tree.arena.get(&id) else {
        return 0.0;
    };
    match &el.context {
        ElementContext::Text(text_ctx) => {
            if text_ctx.no_wrap {
                return layouter.word_width(&text_ctx.text, &text_ctx.font_info);
            }
            text_ctx
                .text
                .split_ascii_whitespace()
                .map(|run| layouter.word_width(run, &text_ctx.font_info))
                .fold(0.0_f32, f32::max)
        }
        // A form control, like an image, is as wide as its box and has no break opportunities.
        ElementContext::Image(_)
        | ElementContext::Svg(_)
        | ElementContext::FormControl(_)
        | ElementContext::SelectPopup(_) => el.box_model.border_box.width as f32,
        ElementContext::TableBorderOverlay(_) => 0.0,
        ElementContext::None => {
            let from_children = el
                .children
                .iter()
                .map(|&cid| subtree_min_content_width(doc, layout_tree, layouter, cid, false))
                .fold(0.0_f32, f32::max);
            if root {
                return from_children;
            }
            // An explicit width is as unbreakable as an image: the box will be that wide
            // whatever its words are.
            let style = doc.computed_style(el.dom_node_id);
            let explicit = style.has(Prop::Width) && style.size.width.to_px().is_some_and(|w| w > 0.0);
            if explicit {
                from_children.max(el.box_model.border_box.width as f32)
            } else {
                from_children
            }
        }
    }
}

/// Post-process all `display: table` nodes in the layout tree after the
/// Taffy first pass. Correct positions are written back via `gosub_lattice`.
/// Needs the layouter itself (not just the mapping) so cells can be re-laid-out
/// at their final lattice widths via `relayout_cell`.
pub fn post_process_tables(layouter: &mut TaffyLayouter, layout_tree: &mut LayoutTree) {
    // Clone the mapping so `layouter` can be borrowed mutably per table below.
    let dom_to_layout = layouter.dom_to_layout_mapping().clone();
    // Clone the doc Arc up front so we don't hold a borrow on layout_tree
    // when we later pass it mutably to PipelineTableTree.
    let doc: Arc<dyn PipelineDocument> = Arc::clone(&layout_tree.render_tree.doc);

    // Collect table nodes in pre-order DOM traversal so outer tables are always
    // processed before any nested tables they contain. This is required so that
    // when we process an inner table, the parent cell's box model has already
    // been updated by the outer table's apply_positions call.
    let mut table_nodes: Vec<(DomNodeId, LayoutElementId)> = Vec::new();
    if let Some(root_dom_id) = doc.root() {
        collect_tables_preorder(&*doc, root_dom_id, &dom_to_layout, &mut table_nodes, 0);
    }

    log::info!("lattice: post_process_tables found {} table node(s)", table_nodes.len());

    // Two passes. Pass 1 is pre-order (outer->inner): it establishes column widths, which flow
    // top-down (a nested table reads its width from its already-sized parent cell). Pass 2 is
    // post-order (inner->outer): each table is re-laid-out *after* the tables nested inside its
    // cells, so an outer cell's height now reflects its nested table's true height - height
    // flows bottom-up. A single reverse pass propagates through any table-nesting depth.
    // A nested table's surrounding geometry is owned by its outer table, so
    // only top-level tables push the document flow around when they resize.
    let table_dom_ids: HashSet<DomNodeId> = table_nodes.iter().map(|&(d, _)| d).collect();
    // One-time raw-vs-collapsed border corrections for skipped-relayout cells,
    // persistent across both passes (see apply_recursive).
    let mut border_corrected: HashSet<DomNodeId> = HashSet::new();
    let mut flow_shifts = FlowShifts::default();
    let is_nested = |dom_id: DomNodeId| -> bool {
        let mut cur = doc.parent(dom_id);
        while let Some(p) = cur {
            if table_dom_ids.contains(&p) {
                return true;
            }
            cur = doc.parent(p);
        }
        false
    };
    let nested: HashSet<DomNodeId> = table_nodes.iter().map(|&(d, _)| d).filter(|&d| is_nested(d)).collect();

    for pass in 0..2 {
        let order: Vec<(DomNodeId, LayoutElementId)> = if pass == 0 {
            table_nodes.clone()
        } else {
            table_nodes.iter().rev().copied().collect()
        };
        for (table_dom_id, table_layout_id) in order {
            lay_out_one_table(
                &*doc,
                layouter,
                layout_tree,
                &dom_to_layout,
                table_dom_id,
                table_layout_id,
                nested.contains(&table_dom_id),
                &mut border_corrected,
                &mut flow_shifts,
            );
        }
    }
    // Before the overlays below, which copy the table boxes.
    flow_shifts.apply(layout_tree);

    // A caption sits outside the table box, in the wrapper box around it (CSS 2 §17.4), so the
    // table's border and background go round the grid alone. Until here the table's box has
    // been that wrapper - the space the flow around it makes room for, and the origin both table
    // passes place the grid from - so it is split only now. The caption's band moves into the
    // table's margin, which leaves the margin box, and with it the table's place in its flow,
    // as it was. Lattice already put the caption out past the table's border and padding.
    for &(table_dom_id, table_layout_id) in &table_nodes {
        let Some(caption) = doc
            .children(table_dom_id)
            .into_iter()
            .find(|&child| is_caption_box(&*doc, child))
        else {
            continue;
        };
        let Some(band) = dom_to_layout
            .get(&caption)
            .and_then(|id| layout_tree.arena.get(id))
            .map(|el| el.box_model.border_box.height)
        else {
            continue;
        };
        let bottom = doc.computed_style(caption).inherited.caption_side == gosub_interface::style::CaptionSide::Bottom;
        let (above, below) = if bottom { (0.0, band) } else { (band, 0.0) };
        let Some(table_el) = layout_tree.arena.get_mut(&table_layout_id) else {
            continue;
        };
        let bm = table_el.box_model;
        let bb = bm.border_box;
        let mut margin = bm.margin;
        margin.top += above;
        margin.bottom += below;
        table_el.box_model = BoxModel::new(
            Rect::new(bb.x, bb.y + above, bb.width, (bb.height - band).max(0.0)),
            bm.padding,
            bm.border,
            margin,
        );
    }

    // Collapsed borders paint IN FRONT of all table content (css-tables /
    // w3c/csswg-drafts#11570): append a synthetic overlay element as each collapsed
    // table's last child. The paint-order DFS then emits the cells' border strips after
    // the whole subtree, so descendants (negative margins, abs boxes) cannot cover them.
    for (table_dom_id, table_layout_id) in table_nodes {
        let collapse = borders_collapse(&*doc, table_dom_id);
        if !collapse {
            continue;
        }
        let mut cells: Vec<LayoutElementId> = Vec::new();
        collect_collapsed_cells(layout_tree, table_layout_id, &mut cells);
        if cells.is_empty() {
            continue;
        }
        let Some(table_el) = layout_tree.arena.get(&table_layout_id) else {
            continue;
        };
        let (bm, render_node_id) = (table_el.box_model, table_el.render_node_id);
        let overlay_id = layout_tree.next_node_id();
        layout_tree.arena.insert(
            overlay_id,
            LayoutElementNode {
                id: overlay_id,
                dom_node_id: table_dom_id,
                render_node_id,
                parent: Some(table_layout_id),
                box_model: bm,
                children: vec![],
                context: crate::layouter::ElementContext::TableBorderOverlay(cells),
                background_media: None,
                collapsed_borders: None,
            },
        );
        if let Some(table_el) = layout_tree.arena.get_mut(&table_layout_id) {
            table_el.children.push(overlay_id);
        }
    }
}

/// DFS-collect (in paint order) the layout elements under `id` that carry collapsed
/// borders. Nested collapsed tables collect their own overlay, so recursion stops at
/// inner `display: table` boundaries.
fn collect_collapsed_cells(layout_tree: &LayoutTree, id: LayoutElementId, out: &mut Vec<LayoutElementId>) {
    let Some(el) = layout_tree.arena.get(&id) else { return };
    for &child_id in &el.children {
        let Some(child) = layout_tree.arena.get(&child_id) else {
            continue;
        };
        if child.collapsed_borders.is_some() {
            out.push(child_id);
        }
        let child_is_table = is_table_box(&*layout_tree.render_tree.doc, child.dom_node_id);
        if !child_is_table {
            collect_collapsed_cells(layout_tree, child_id, out);
        }
    }
}

/// Run lattice for a single table node and write the computed cell positions and the table's
/// own size back into the layout tree.
#[allow(clippy::too_many_arguments)]
fn lay_out_one_table(
    doc: &dyn PipelineDocument,
    layouter: &mut TaffyLayouter,
    layout_tree: &mut LayoutTree,
    dom_to_layout: &HashMap<DomNodeId, LayoutElementId>,
    table_dom_id: DomNodeId,
    table_layout_id: LayoutElementId,
    is_nested: bool,
    border_corrected: &mut HashSet<DomNodeId>,
    flow_shifts: &mut FlowShifts,
) {
    // Use the parent element's content width as available_width. For nested
    // tables the parent is a table cell whose box model was already updated
    // by the outer table's apply_positions call, giving us the correct width.
    // Fall back to the table's own Taffy-computed width for root-level tables.
    //
    // An absolutely-positioned table is the exception: its width is constrained by its
    // insets (`left`/`right`), which taffy has already resolved in the first pass - the
    // parent's content width would ignore them (CSS 2 §10.3.7).
    let table_style = doc.computed_style(table_dom_id);
    let table_is_abs = table_style.has(Prop::Position)
        && matches!(table_style.box_group.position, Position::Absolute | Position::Fixed);
    let own_width = || {
        layout_tree
            .arena
            .get(&table_layout_id)
            .map(|e| e.box_model.content_box.width)
            .unwrap_or(0.0)
    };
    let available_width = if table_is_abs {
        own_width()
    } else {
        doc.parent(table_dom_id)
            .and_then(|p| dom_to_layout.get(&p))
            .and_then(|&pid| layout_tree.arena.get(&pid))
            .map(|el| el.box_model.content_box.width)
            .unwrap_or_else(own_width)
    };

    let old_box = layout_tree.arena.get(&table_layout_id).map(|e| e.box_model.border_box);

    let mut tree = PipelineTableTree::new(doc, layouter, layout_tree, dom_to_layout);

    match gosub_lattice::compute_table_layout(&mut tree, table_dom_id, available_width, None) {
        Ok((table_width, table_height)) => {
            tree.apply_positions(table_dom_id, border_corrected);
            // Write back both dimensions so deeply-nested tables can read the
            // correct width from this table's box model via their parent lookup.
            // Lattice returns the GRID extents (content box); the table's own border
            // and padding wrap around them to form the border box - without this a
            // `border-bottom: 100px` table collapsed to its (possibly zero) grid.
            // Under border-collapse the table has no padding and its border joined the
            // perimeter conflict inside lattice (the resolved halves are part of the
            // returned extents), so nothing wraps around the grid.
            let collapse = borders_collapse(doc, table_dom_id);
            if let Some(el) = layout_tree.arena.get_mut(&table_layout_id) {
                let bb = el.box_model.border_box;
                let (border, padding) = if collapse {
                    (Edges::ZERO, Edges::ZERO)
                } else {
                    (el.box_model.border, el.box_model.padding)
                };
                let bw = table_width + border.left + border.right + padding.left + padding.right;
                let bh = table_height + border.top + border.bottom + padding.top + padding.bottom;
                el.box_model = BoxModel::new(Rect::new(bb.x, bb.y, bw, bh), padding, border, el.box_model.margin);
            }
            // The first taffy pass only approximated the table's height; when
            // lattice's real height differs, the rest of the document flow
            // still sits at the old positions. Shift everything below the
            // table down (or up) by the delta and grow the ancestor chain, so
            // following siblings and the page height stay correct. Nested
            // tables skip this - the outer table's own lattice pass owns the
            // geometry around them.
            //
            // A float or an absolutely positioned table is out of flow: what follows it in
            // the source is laid out beside or behind it, not after it, so its height must
            // not push anything down. Wikipedia's infobox and its thumbnails are floated
            // tables, and absorbing their growth shoved the whole article body 400px down.
            let table_is_out_of_flow = table_is_abs || float_side(doc, table_dom_id).is_some();
            if !is_nested && !table_is_out_of_flow {
                if let Some(old) = old_box {
                    let new_h = layout_tree
                        .arena
                        .get(&table_layout_id)
                        .map(|e| e.box_model.border_box.height)
                        .unwrap_or(table_height);
                    let delta = new_h - old.height;
                    if std::env::var("LATTICE_DEBUG").is_ok() {
                        eprintln!(
                            "lattice-dbg: shift table {:?} old_h={} new_h={} delta={}",
                            table_dom_id, old.height, new_h, delta
                        );
                    }
                    if delta.abs() > 0.5 {
                        flow_shifts.record(table_layout_id, old.y + old.height, delta);
                    }
                }
            }
        }
        Err(e) => {
            log::warn!("lattice: table layout failed for node {:?}: {:?}", table_dom_id, e);
        }
    }
}

/// The flow shifts caused by tables whose real height differs from the first taffy pass.
///
/// A table that changes height moves what follows it in its flow, and its ancestors grow to
/// hold it. What follows is decided level by level, from the table up: in each ancestor, the
/// children that start at or below the old bottom of the child that grew move by that growth,
/// and the ancestor itself grows by as much as its lowest child bottom moved. So a table in one
/// column of a flex row moves what is under it in that column, not the column beside it, and
/// the row grows only if that column was the tallest.
///
/// Applying each shift as it happened scanned and moved the whole arena once per table, so a
/// page of many tables was quadratic. 10,000 sibling tables took 19 s. The shifts are recorded
/// here instead and applied in one pass at the end, working in unshifted coordinates
/// throughout: nothing the table passes read depends on where another table sits, and a table
/// and its cells are placed relative to the table's own box.
///
/// A table that resizes in both passes, as one holding a nested table does, is recorded once:
/// against its original bottom, with both changes added up.
#[derive(Default)]
struct FlowShifts {
    /// Per resized table: its original bottom and its total change in height.
    tables: HashMap<LayoutElementId, (f64, f64)>,
}

impl FlowShifts {
    /// Record that `table` grew by `delta` (negative: shrank) from a bottom at `old_bottom`.
    fn record(&mut self, table: LayoutElementId, old_bottom: f64, delta: f64) {
        self.tables
            .entry(table)
            .and_modify(|(_, total)| *total += delta)
            .or_insert((old_bottom, delta));
    }

    fn apply(self, layout_tree: &mut LayoutTree) {
        if self.tables.is_empty() {
            return;
        }
        // An element at `y` is below a bottom at `threshold` when `y >= threshold - 0.5`.
        let below = |threshold: f64, y: f64| y >= threshold - 0.5;
        let bottom = |el: &LayoutElementNode| el.box_model.border_box.y + el.box_model.border_box.height;

        // Growth and old bottom of every element that grew: the tables, then their ancestors.
        // A table's box already holds its new height, so its old bottom is the recorded one.
        let mut grown: HashMap<LayoutElementId, (f64, f64)> = self
            .tables
            .iter()
            .map(|(&id, &(old_bottom, delta))| (id, (old_bottom, delta)))
            .collect();

        // The ancestors to resolve, deepest first, so every child's growth is known by the time
        // its parent is resolved.
        let mut depth: HashMap<LayoutElementId, usize> = HashMap::new();
        for &table in self.tables.keys() {
            let mut chain = Vec::new();
            let mut cur = layout_tree.arena.get(&table).and_then(|el| el.parent);
            while let Some(id) = cur {
                if depth.contains_key(&id) {
                    break;
                }
                chain.push(id);
                cur = layout_tree.arena.get(&id).and_then(|el| el.parent);
            }
            let base = cur.and_then(|id| depth.get(&id)).map_or(0, |&d| d + 1);
            for (i, &id) in chain.iter().rev().enumerate() {
                depth.insert(id, base + i);
            }
        }
        let mut ancestors: Vec<LayoutElementId> = depth.keys().copied().collect();
        ancestors.sort_by_key(|id| std::cmp::Reverse(depth[id]));

        // How far each child moves within its parent.
        let mut moved: HashMap<LayoutElementId, f64> = HashMap::new();
        for parent in ancestors {
            let children = layout_tree
                .arena
                .get(&parent)
                .map(|el| el.children.clone())
                .unwrap_or_default();

            let mut shifts: Vec<(f64, f64, LayoutElementId)> = children
                .iter()
                .filter_map(|&c| grown.get(&c).map(|&(old_bottom, growth)| (old_bottom, growth, c)))
                .collect();
            shifts.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut prefix = Vec::with_capacity(shifts.len() + 1);
            prefix.push(0.0);
            for &(_, growth, _) in &shifts {
                prefix.push(prefix[prefix.len() - 1] + growth);
            }

            let mut old_lowest = f64::NEG_INFINITY;
            let mut new_lowest = f64::NEG_INFINITY;
            for &child in &children {
                let Some(el) = layout_tree.arena.get(&child) else {
                    continue;
                };
                let y = el.box_model.border_box.y;
                let (old_bottom, growth) = grown.get(&child).copied().unwrap_or((bottom(el), 0.0));
                let mut offset = prefix[shifts.partition_point(|&(threshold, _, _)| below(threshold, y))];
                // A child never moves for its own growth, which a zero-height one would.
                if growth != 0.0 && below(old_bottom, y) {
                    offset -= growth;
                }
                if offset != 0.0 {
                    moved.insert(child, offset);
                }
                old_lowest = old_lowest.max(old_bottom);
                new_lowest = new_lowest.max(old_bottom + growth + offset);
            }

            let growth = if old_lowest.is_finite() {
                new_lowest - old_lowest
            } else {
                0.0
            };
            if let Some(el) = layout_tree.arena.get(&parent) {
                grown.insert(parent, (bottom(el), growth));
            }
        }

        // One walk from the root: every element moves with everything above it.
        let mut visited: HashSet<LayoutElementId> = HashSet::new();
        let mut stack = vec![(
            layout_tree.root_id,
            moved.get(&layout_tree.root_id).copied().unwrap_or(0.0),
        )];
        while let Some((id, offset)) = stack.pop() {
            if !visited.insert(id) {
                continue;
            }
            let Some(el) = layout_tree.arena.get_mut(&id) else {
                continue;
            };
            translate_box_model(&mut el.box_model, Coordinate::new(0.0, offset));
            if !self.tables.contains_key(&id) {
                if let Some(&(_, growth)) = grown.get(&id) {
                    el.box_model.border_box.height += growth;
                    el.box_model.padding_box.height += growth;
                    el.box_model.content_box.height += growth;
                    el.box_model.margin_box.height += growth;
                }
            }
            for &child in &el.children {
                stack.push((child, offset + moved.get(&child).copied().unwrap_or(0.0)));
            }
        }

        // Elements outside the tree (the open `<select>` popup) keep the plain rule: moved by
        // every table whose old bottom they sit below.
        let mut by_bottom: Vec<(f64, f64)> = self.tables.values().copied().collect();
        by_bottom.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut prefix = Vec::with_capacity(by_bottom.len() + 1);
        prefix.push(0.0);
        for &(_, delta) in &by_bottom {
            prefix.push(prefix[prefix.len() - 1] + delta);
        }
        for (id, el) in layout_tree.arena.iter_mut() {
            if visited.contains(id) {
                continue;
            }
            let y = el.box_model.border_box.y;
            let offset = prefix[by_bottom.partition_point(|&(threshold, _)| below(threshold, y))];
            translate_box_model(&mut el.box_model, Coordinate::new(0.0, offset));
        }
    }
}

/// Pre-order DFS that collects all `display: table` nodes into `out`, parents first.
///
/// This walks the DOM, not the layout tree, so the layout depth cap does not bound it on its
/// own: a document nesting 20,000 elements overflowed the stack here. It stops at
/// [`MAX_LAYOUT_DEPTH`] instead. Nothing is lost by that - every level of the DOM path to a box
/// is a level of the layout tree too, so a node deeper than the cap has no layout box and would
/// never have been collected.
fn collect_tables_preorder(
    doc: &dyn PipelineDocument,
    id: DomNodeId,
    dom_to_layout: &HashMap<DomNodeId, LayoutElementId>,
    out: &mut Vec<(DomNodeId, LayoutElementId)>,
    depth: usize,
) {
    if is_table_box(doc, id) {
        if let Some(&layout_id) = dom_to_layout.get(&id) {
            out.push((id, layout_id));
        }
    }
    if depth > MAX_LAYOUT_DEPTH {
        return;
    }
    for child in doc.children(id) {
        collect_tables_preorder(doc, child, dom_to_layout, out, depth + 1);
    }
}

/// Whether the element's own cascade made it a table box.
fn is_table_box(doc: &dyn PipelineDocument, id: DomNodeId) -> bool {
    let style = doc.computed_style(id);
    style.has(Prop::Display) && matches!(style.box_group.display, Display::Table | Display::InlineTable)
}

fn is_caption_box(doc: &dyn PipelineDocument, id: DomNodeId) -> bool {
    let style = doc.computed_style(id);
    style.has(Prop::Display) && style.box_group.display == Display::TableCaption
}

/// Whether a table collapses its borders. Inherited, so the computed value is the answer
/// wherever it is asked.
fn borders_collapse(doc: &dyn PipelineDocument, id: DomNodeId) -> bool {
    doc.computed_style(id).inherited.border_collapse == gosub_interface::style::BorderCollapse::Collapse
}
