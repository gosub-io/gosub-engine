use gosub_lattice::{BoxEdges, CellLayout, CssLength, CssProp, TableRole, TableTree, VerticalAlign};

use crate::common::document::node::{NodeId as DomNodeId, NodeType};
use crate::common::document::pipeline_doc::PipelineDocument;
use crate::common::document::style::{lookup, Display, StyleProperty, Unit, Value};
use crate::common::geo::{Coordinate, Rect};
use crate::layouter::box_model::{BoxModel, Edges};
use crate::layouter::taffy::TaffyLayouter;
use crate::layouter::{CollapsedCellBorders, ElementContext, LayoutElementId, LayoutElementNode, LayoutTree};
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
        }
    }

    /// True when the DOM subtree under `id` contains a `display: table` node.
    /// Such cells keep the first-pass height approximation: re-running taffy on
    /// them would clobber the box models lattice computed for the inner table.
    fn subtree_contains_table(&self, id: DomNodeId) -> bool {
        self.doc.children(id).iter().any(|&child| {
            matches!(
                self.doc.get_own_style(child, &StyleProperty::Display),
                Some(Value::Display(Display::Table | Display::InlineTable))
            ) || self.subtree_contains_table(child)
        })
    }

    /// Sum of the border-box heights of the nested tables directly contained in a cell (not
    /// counting tables nested deeper inside those). Zero if the cell holds no table. This lets
    /// a table cell grow to contain a nested table whose height lattice computes in a later pass.
    fn nested_table_height(&self, cell_layout_id: LayoutElementId) -> f32 {
        let Some(el) = self.layout_tree.arena.get(&cell_layout_id) else {
            return 0.0;
        };
        let mut total = 0.0;
        for &child_id in &el.children {
            let Some(child) = self.layout_tree.arena.get(&child_id) else {
                continue;
            };
            let is_table = matches!(
                self.doc.get_own_style(child.dom_node_id, &StyleProperty::Display),
                Some(Value::Display(Display::Table | Display::InlineTable))
            );
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
        let collapse = matches!(
            self.doc.get_style(table_dom_id, &StyleProperty::BorderCollapse),
            Value::Keyword(k) if lookup(k) == "collapse"
        );
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
    for child_id in doc.children(id) {
        match pending.get(&child_id) {
            None => {
                // Non-table-structure node: shift it by the accumulated translation
                // so it stays correctly positioned relative to its parent cell.
                if let Some(&layout_id) = dom_to_layout.get(&child_id) {
                    if let Some(element) = arena.get_mut(&layout_id) {
                        translate_box_model(&mut element.box_model, offset);
                    }
                }
                apply_recursive(doc, child_id, parent_abs, offset, pending, edge_owners, relaid, border_corrected, dom_to_layout, arena);
            }
            Some(cell_layout) => {
                let abs = Coordinate::new(
                    parent_abs.x + cell_layout.position.x as f64,
                    parent_abs.y + cell_layout.position.y as f64,
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
                        element.collapsed_borders = edge_owners.get(&child_id).map(|&owners| CollapsedCellBorders {
                            widths: [
                                cell_layout.border.top,
                                cell_layout.border.right,
                                cell_layout.border.bottom,
                                cell_layout.border.left,
                            ],
                            outsets: cell_layout.border_outsets,
                            owners,
                        });
                    }
                }
                // vertical-align: only cells whose subtree was re-anchored at the
                // cell top this pass get the shift, so it applies exactly once.
                let valign_shift = if relaid.contains(&child_id) {
                    cell_layout.content_offset_y as f64
                } else {
                    0.0
                };
                // Skipped-relayout collapsed cells (nested-table guard): their children were
                // positioned by the FIRST taffy pass with the raw CSS border widths, but the
                // collapse geometry replaced those with half the resolved boundary. Shift the
                // subtree by the difference so content sits at the collapsed content origin.
                let border_delta = if !relaid.contains(&child_id) && border_corrected.insert(child_id) {
                    let raw_left = doc.get_style_f32(child_id, &StyleProperty::BorderLeftWidth) as f64;
                    let raw_top = doc.get_style_f32(child_id, &StyleProperty::BorderTopWidth) as f64;
                    let dl = cell_layout.border.left as f64 - raw_left;
                    let dt = cell_layout.border.top as f64 - raw_top;
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
                apply_recursive(doc, child_id, abs, child_offset, pending, edge_owners, relaid, border_corrected, dom_to_layout, arena);
            }
        }
    }
}

fn translate_box_model(bm: &mut BoxModel, offset: Coordinate) {
    if offset.x == 0.0 && offset.y == 0.0 {
        return;
    }
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
    let border_box = Rect::new(abs.x, abs.y, layout.size.width as f64, layout.size.height as f64);
    BoxModel::new(
        border_box,
        Edges {
            top: layout.padding.top as f64,
            right: layout.padding.right as f64,
            bottom: layout.padding.bottom as f64,
            left: layout.padding.left as f64,
        },
        Edges {
            top: layout.border.top as f64,
            right: layout.border.right as f64,
            bottom: layout.border.bottom as f64,
            left: layout.border.left as f64,
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

    fn children(&self, id: DomNodeId) -> Vec<DomNodeId> {
        self.doc.children(id)
    }

    fn table_role(&self, id: DomNodeId) -> TableRole {
        match self.doc.get_own_style(id, &StyleProperty::Display) {
            Some(Value::Display(d)) => match d {
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
            },
            _ => TableRole::Other,
        }
    }

    fn css_length(&self, id: DomNodeId, prop: CssProp) -> CssLength {
        let style_prop = match prop {
            CssProp::Width => StyleProperty::Width,
            CssProp::Height => StyleProperty::Height,
            CssProp::MinWidth => StyleProperty::MinWidth,
            CssProp::MinHeight => StyleProperty::MinHeight,
            CssProp::MaxWidth => StyleProperty::MaxWidth,
            CssProp::MaxHeight => StyleProperty::MaxHeight,
            CssProp::BorderTopWidth => StyleProperty::BorderTopWidth,
            CssProp::BorderRightWidth => StyleProperty::BorderRightWidth,
            CssProp::BorderBottomWidth => StyleProperty::BorderBottomWidth,
            CssProp::BorderLeftWidth => StyleProperty::BorderLeftWidth,
            CssProp::PaddingTop => StyleProperty::PaddingTop,
            CssProp::PaddingRight => StyleProperty::PaddingRight,
            CssProp::PaddingBottom => StyleProperty::PaddingBottom,
            CssProp::PaddingLeft => StyleProperty::PaddingLeft,
            // border-spacing is inherited, so get_style (below) resolves it through
            // the cascade down to the UA default (`table { border-spacing: 2px }`).
            CssProp::BorderSpacingX => StyleProperty::BorderSpacingX,
            CssProp::BorderSpacingY => StyleProperty::BorderSpacingY,
            // Px(1.0) is the lattice sentinel for `table-layout: fixed`.
            CssProp::TableLayout => {
                return match self.doc.get_style(id, &StyleProperty::TableLayout) {
                    Value::Keyword(k) if lookup(k) == "fixed" => CssLength::Px(1.0),
                    _ => CssLength::Auto,
                };
            }
            // Px(1.0) = `border-collapse: collapse` (inherited, so get_style walks up).
            CssProp::BorderCollapse => {
                return match self.doc.get_style(id, &StyleProperty::BorderCollapse) {
                    Value::Keyword(k) if lookup(k) == "collapse" => CssLength::Px(1.0),
                    _ => CssLength::Auto,
                };
            }
            // Px(1.0) = `caption-side: bottom`.
            CssProp::CaptionSide => {
                return match self.doc.get_style(id, &StyleProperty::CaptionSide) {
                    Value::Keyword(k) if lookup(k) == "bottom" => CssLength::Px(1.0),
                    _ => CssLength::Auto,
                };
            }
            // Resolved by the dedicated trait method, not css_length.
            CssProp::VerticalAlign => return CssLength::Auto,
        };

        match self.doc.get_style(id, &style_prop) {
            Value::Unit(v, Unit::Px) => CssLength::Px(v),
            Value::Unit(v, Unit::Percent) => CssLength::Percent(v),
            Value::Unit(0.0, _) => CssLength::Zero,
            _ => CssLength::Auto,
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

    fn layout_cell(&mut self, id: DomNodeId, available_width: f32) -> f32 {
        let Some(&layout_id) = self.dom_to_layout.get(&id) else {
            return 0.0;
        };

        // Cells hosting a nested table re-use the first-pass height instead of
        // re-laying-out: the nested table's real height is only known after
        // lattice lays it out, and the second (bottom-up) pass in
        // `post_process_tables` propagates it up here.
        if self.subtree_contains_table(id) {
            if let Some(element) = self.layout_tree.arena.get(&layout_id) {
                let taffy_h = element.box_model.content_box.height as f32;
                return taffy_h.max(self.nested_table_height(layout_id));
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
                (el.box_model.border.left + el.box_model.border.right + el.box_model.padding.left
                    + el.box_model.padding.right) as f32
            })
            .unwrap_or(0.0);
        if let Some(content_h) = self
            .layouter
            .relayout_cell(self.layout_tree, layout_id, available_width + extras)
        {
            self.relaid.insert(id);
            return content_h;
        }

        // Fallback: the content height from the taffy first pass.
        self.layout_tree
            .arena
            .get(&layout_id)
            .map(|el| el.box_model.content_box.height as f32)
            .unwrap_or(0.0)
    }

    /// Resolves `vertical-align` for a cell by walking up to the table: the
    /// HTML rendering spec puts `vertical-align: inherit` on cells and
    /// `middle` on rows/sections, so the browser default falls out of the walk.
    /// `baseline` (and the inline-only keywords) approximate as Top.
    fn vertical_align(&self, id: DomNodeId) -> VerticalAlign {
        let mut cur = Some(id);
        while let Some(node) = cur {
            if let Some(Value::Keyword(k)) = self.doc.get_own_style(node, &StyleProperty::VerticalAlign) {
                match lookup(k).as_str() {
                    "top" => return VerticalAlign::Top,
                    "middle" => return VerticalAlign::Middle,
                    "bottom" => return VerticalAlign::Bottom,
                    // CSS 2 §17.5.3: cell values other than top/middle/bottom behave
                    // as baseline.
                    "baseline" | "text-top" | "text-bottom" | "sub" | "super" => {
                        return VerticalAlign::Baseline
                    }
                    // "inherit" (or anything unrecognised): keep walking.
                    _ => {}
                }
            }
            if self.table_role(node) == TableRole::Table {
                break;
            }
            cur = self.doc.parent(node);
        }
        VerticalAlign::Top
    }

    fn cell_baseline(&mut self, id: DomNodeId) -> Option<f32> {
        let &layout_id = self.dom_to_layout.get(&id)?;
        let cell_top = self.layout_tree.arena.get(&layout_id)?.box_model.border_box.y;

        // First text element in the cell's subtree, in tree order = the first in-flow
        // line box (nested tables excluded - their baselines don't propagate here).
        fn first_text(tree: &LayoutTree, id: LayoutElementId, doc: &dyn PipelineDocument) -> Option<LayoutElementId> {
            let el = tree.arena.get(&id)?;
            if matches!(el.context, ElementContext::Text(_)) {
                return Some(id);
            }
            if matches!(
                doc.get_own_style(el.dom_node_id, &StyleProperty::Display),
                Some(Value::Display(Display::Table | Display::InlineTable))
            ) {
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
        Some((text_el.box_model.content_box.y - cell_top) as f32 + ascent)
    }

    fn cell_intrinsic_widths(&mut self, id: DomNodeId) -> (f32, f32) {
        let Some(&layout_id) = self.dom_to_layout.get(&id) else {
            if std::env::var("LATTICE_DEBUG").is_ok() {
                eprintln!("lattice-dbg: cell {:?} NOT in dom_to_layout", id);
            }
            return (0.0, 0.0);
        };
        let w = self.layouter.measure_intrinsic_widths(layout_id).unwrap_or((0.0, 0.0));
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
        collect_tables_preorder(&*doc, root_dom_id, &dom_to_layout, &mut table_nodes);
    }

    log::info!("lattice: post_process_tables found {} table node(s)", table_nodes.len());

    // Two passes. Pass 1 is pre-order (outer→inner): it establishes column widths, which flow
    // top-down (a nested table reads its width from its already-sized parent cell). Pass 2 is
    // post-order (inner→outer): each table is re-laid-out *after* the tables nested inside its
    // cells, so an outer cell's height now reflects its nested table's true height - height
    // flows bottom-up. A single reverse pass propagates through any table-nesting depth.
    // A nested table's surrounding geometry is owned by its outer table, so
    // only top-level tables push the document flow around when they resize.
    let table_dom_ids: HashSet<DomNodeId> = table_nodes.iter().map(|&(d, _)| d).collect();
    // One-time raw-vs-collapsed border corrections for skipped-relayout cells,
    // persistent across both passes (see apply_recursive).
    let mut border_corrected: HashSet<DomNodeId> = HashSet::new();
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
            );
        }
    }

    // Collapsed borders paint IN FRONT of all table content (css-tables /
    // w3c/csswg-drafts#11570): append a synthetic overlay element as each collapsed
    // table's last child. The paint-order DFS then emits the cells' border strips after
    // the whole subtree, so descendants (negative margins, abs boxes) cannot cover them.
    for (table_dom_id, table_layout_id) in table_nodes {
        let collapse = matches!(
            doc.get_style(table_dom_id, &StyleProperty::BorderCollapse),
            Value::Keyword(k) if lookup(k) == "collapse"
        );
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
        let Some(child) = layout_tree.arena.get(&child_id) else { continue };
        if child.collapsed_borders.is_some() {
            out.push(child_id);
        }
        let child_is_table = matches!(
            layout_tree
                .render_tree
                .doc
                .get_own_style(child.dom_node_id, &StyleProperty::Display),
            Some(Value::Display(Display::Table | Display::InlineTable))
        );
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
) {
    // Use the parent element's content width as available_width. For nested
    // tables the parent is a table cell whose box model was already updated
    // by the outer table's apply_positions call, giving us the correct width.
    // Fall back to the table's own Taffy-computed width for root-level tables.
    //
    // An absolutely-positioned table is the exception: its width is constrained by its
    // insets (`left`/`right`), which taffy has already resolved in the first pass - the
    // parent's content width would ignore them (CSS 2 §10.3.7).
    let table_is_abs = matches!(
        doc.get_own_style(table_dom_id, &StyleProperty::Position),
        Some(Value::Keyword(id)) if matches!(lookup(id).as_str(), "absolute" | "fixed")
    );
    let own_width = || {
        layout_tree
            .arena
            .get(&table_layout_id)
            .map(|e| e.box_model.content_box.width as f32)
            .unwrap_or(0.0)
    };
    let available_width = if table_is_abs {
        own_width()
    } else {
        doc.parent(table_dom_id)
            .and_then(|p| dom_to_layout.get(&p))
            .and_then(|&pid| layout_tree.arena.get(&pid))
            .map(|el| el.box_model.content_box.width as f32)
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
            let collapse = matches!(
                doc.get_style(table_dom_id, &StyleProperty::BorderCollapse),
                Value::Keyword(k) if lookup(k) == "collapse"
            );
            if let Some(el) = layout_tree.arena.get_mut(&table_layout_id) {
                let bb = el.box_model.border_box;
                let (border, padding) = if collapse {
                    (Edges::ZERO, Edges::ZERO)
                } else {
                    (el.box_model.border, el.box_model.padding)
                };
                let bw = table_width as f64 + border.left + border.right + padding.left + padding.right;
                let bh = table_height as f64 + border.top + border.bottom + padding.top + padding.bottom;
                el.box_model = BoxModel::new(Rect::new(bb.x, bb.y, bw, bh), padding, border, el.box_model.margin);
            }
            // The first taffy pass only approximated the table's height; when
            // lattice's real height differs, the rest of the document flow
            // still sits at the old positions. Shift everything below the
            // table down (or up) by the delta and grow the ancestor chain, so
            // following siblings and the page height stay correct. Nested
            // tables skip this - the outer table's own lattice pass owns the
            // geometry around them.
            if !is_nested {
                if let Some(old) = old_box {
                    let new_h = layout_tree
                        .arena
                        .get(&table_layout_id)
                        .map(|e| e.box_model.border_box.height)
                        .unwrap_or(table_height as f64);
                    let delta = new_h - old.height;
                    if std::env::var("LATTICE_DEBUG").is_ok() {
                        eprintln!(
                            "lattice-dbg: shift table {:?} old_h={} new_h={} delta={}",
                            table_dom_id, old.height, new_h, delta
                        );
                    }
                    if delta.abs() > 0.5 {
                        shift_flow_below(layout_tree, table_layout_id, old.y + old.height, delta);
                    }
                }
            }
        }
        Err(e) => {
            log::warn!("lattice: table layout failed for node {:?}: {:?}", table_dom_id, e);
        }
    }
}

/// Translate every layout element that sits below `old_bottom` by `delta` and
/// grow the resized table's ancestors, approximating the block reflow that the
/// table's new height would cause. The table's own subtree is exempt (lattice
/// already positioned it), as are its ancestors (they grow instead of moving).
fn shift_flow_below(layout_tree: &mut LayoutTree, table_layout_id: LayoutElementId, old_bottom: f64, delta: f64) {
    let mut exempt: HashSet<LayoutElementId> = HashSet::new();
    collect_subtree(layout_tree, table_layout_id, &mut exempt);

    let mut ancestors: Vec<LayoutElementId> = Vec::new();
    let mut cur = layout_tree.arena.get(&table_layout_id).and_then(|e| e.parent);
    while let Some(id) = cur {
        ancestors.push(id);
        cur = layout_tree.arena.get(&id).and_then(|e| e.parent);
    }
    exempt.extend(ancestors.iter().copied());

    let ids: Vec<LayoutElementId> = layout_tree.arena.keys().copied().collect();
    for id in ids {
        if exempt.contains(&id) {
            continue;
        }
        if let Some(el) = layout_tree.arena.get_mut(&id) {
            if el.box_model.border_box.y >= old_bottom - 0.5 {
                translate_box_model(&mut el.box_model, Coordinate::new(0.0, delta));
            }
        }
    }

    for id in ancestors {
        if let Some(el) = layout_tree.arena.get_mut(&id) {
            el.box_model.border_box.height += delta;
            el.box_model.padding_box.height += delta;
            el.box_model.content_box.height += delta;
            el.box_model.margin_box.height += delta;
        }
    }
}

fn collect_subtree(layout_tree: &LayoutTree, id: LayoutElementId, out: &mut HashSet<LayoutElementId>) {
    if !out.insert(id) {
        return;
    }
    let children = layout_tree.arena.get(&id).map(|e| e.children.clone()).unwrap_or_default();
    for child in children {
        collect_subtree(layout_tree, child, out);
    }
}

/// Pre-order DFS that collects all `display: table` nodes into `out`, parents first.
fn collect_tables_preorder(
    doc: &dyn PipelineDocument,
    id: DomNodeId,
    dom_to_layout: &HashMap<DomNodeId, LayoutElementId>,
    out: &mut Vec<(DomNodeId, LayoutElementId)>,
) {
    if matches!(
        doc.get_own_style(id, &StyleProperty::Display),
        Some(Value::Display(Display::Table | Display::InlineTable))
    ) {
        if let Some(&layout_id) = dom_to_layout.get(&id) {
            out.push((id, layout_id));
        }
    }
    for child in doc.children(id) {
        collect_tables_preorder(doc, child, dom_to_layout, out);
    }
}
