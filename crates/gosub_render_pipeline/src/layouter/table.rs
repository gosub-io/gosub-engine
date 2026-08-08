use gosub_lattice::{CellLayout, CssLength, CssProp, TableRole, TableTree, VerticalAlign};

use crate::common::document::node::{NodeId as DomNodeId, NodeType};
use crate::common::document::pipeline_doc::PipelineDocument;
use crate::common::document::style::{lookup, Display, StyleProperty, Unit, Value};
use crate::common::geo::{Coordinate, Rect};
use crate::layouter::box_model::{BoxModel, Edges};
use crate::layouter::taffy::TaffyLayouter;
use crate::layouter::{LayoutElementId, LayoutElementNode, LayoutTree};
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
                Some(Value::Display(Display::Table))
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
                Some(Value::Display(Display::Table))
            );
            if is_table {
                // Self-contained nested table - stop here, don't double-count its inner tables.
                total += child.box_model.border_box.height as f32;
            } else {
                // The table may be wrapped (e.g. in an anonymous box); keep descending.
                total += self.nested_table_height(child_id);
            }
        }
        total
    }

    /// Convert pending relative positions to absolute `BoxModel`s in the arena.
    /// Must be called after `compute_table_layout` returns.
    pub fn apply_positions(&mut self, table_dom_id: DomNodeId) {
        let table_abs = self
            .dom_to_layout
            .get(&table_dom_id)
            .and_then(|id| self.layout_tree.arena.get(id))
            .map(|e| Coordinate::new(e.box_model.content_box.x, e.box_model.content_box.y))
            .unwrap_or(Coordinate::ZERO);

        let pending = std::mem::take(&mut self.pending);
        apply_recursive(
            self.doc,
            table_dom_id,
            table_abs,
            Coordinate::ZERO,
            &pending,
            &self.relaid,
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
    relaid: &HashSet<DomNodeId>,
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
                apply_recursive(doc, child_id, parent_abs, offset, pending, relaid, dom_to_layout, arena);
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
                    }
                }
                // vertical-align: only cells whose subtree was re-anchored at the
                // cell top this pass get the shift, so it applies exactly once.
                let valign_shift = if relaid.contains(&child_id) {
                    cell_layout.content_offset_y as f64
                } else {
                    0.0
                };
                let child_offset = Coordinate::new(abs.x - old_abs.x, abs.y - old_abs.y + valign_shift);
                apply_recursive(doc, child_id, abs, child_offset, pending, relaid, dom_to_layout, arena);
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
                Display::Table => TableRole::Table,
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
                    "baseline" | "text-top" | "text-bottom" | "sub" | "super" => return VerticalAlign::Top,
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

    fn cell_intrinsic_widths(&mut self, id: DomNodeId) -> (f32, f32) {
        let Some(&layout_id) = self.dom_to_layout.get(&id) else {
            return (0.0, 0.0);
        };
        self.layouter.measure_intrinsic_widths(layout_id).unwrap_or((0.0, 0.0))
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
    for pass in 0..2 {
        let order: Vec<(DomNodeId, LayoutElementId)> = if pass == 0 {
            table_nodes.clone()
        } else {
            table_nodes.iter().rev().copied().collect()
        };
        for (table_dom_id, table_layout_id) in order {
            lay_out_one_table(&*doc, layouter, layout_tree, &dom_to_layout, table_dom_id, table_layout_id);
        }
    }
}

/// Run lattice for a single table node and write the computed cell positions and the table's
/// own size back into the layout tree.
fn lay_out_one_table(
    doc: &dyn PipelineDocument,
    layouter: &mut TaffyLayouter,
    layout_tree: &mut LayoutTree,
    dom_to_layout: &HashMap<DomNodeId, LayoutElementId>,
    table_dom_id: DomNodeId,
    table_layout_id: LayoutElementId,
) {
    // Use the parent element's content width as available_width. For nested
    // tables the parent is a table cell whose box model was already updated
    // by the outer table's apply_positions call, giving us the correct width.
    // Fall back to the table's own Taffy-computed width for root-level tables.
    let available_width = doc
        .parent(table_dom_id)
        .and_then(|p| dom_to_layout.get(&p))
        .and_then(|&pid| layout_tree.arena.get(&pid))
        .map(|el| el.box_model.content_box.width as f32)
        .unwrap_or_else(|| {
            layout_tree
                .arena
                .get(&table_layout_id)
                .map(|e| e.box_model.content_box.width as f32)
                .unwrap_or(0.0)
        });

    let mut tree = PipelineTableTree::new(doc, layouter, layout_tree, dom_to_layout);

    match gosub_lattice::compute_table_layout(&mut tree, table_dom_id, available_width, None) {
        Ok((table_width, table_height)) => {
            tree.apply_positions(table_dom_id);
            // Write back both dimensions so deeply-nested tables can read the
            // correct width from this table's box model via their parent lookup.
            if let Some(el) = layout_tree.arena.get_mut(&table_layout_id) {
                let bb = el.box_model.border_box;
                el.box_model = BoxModel::new(
                    Rect::new(bb.x, bb.y, table_width as f64, table_height as f64),
                    el.box_model.padding,
                    el.box_model.border,
                    el.box_model.margin,
                );
            }
        }
        Err(e) => {
            log::warn!("lattice: table layout failed for node {:?}: {:?}", table_dom_id, e);
        }
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
        Some(Value::Display(Display::Table))
    ) {
        if let Some(&layout_id) = dom_to_layout.get(&id) {
            out.push((id, layout_id));
        }
    }
    for child in doc.children(id) {
        collect_tables_preorder(doc, child, dom_to_layout, out);
    }
}
