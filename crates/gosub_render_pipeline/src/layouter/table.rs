use gosub_lattice::{BorderCollapse, BoxEdges, CellLayout, CssLength, CssProp, TableRole, TableTree};

use crate::common::document::node::{NodeId as DomNodeId, NodeType};
use crate::common::document::pipeline_doc::PipelineDocument;
use crate::common::document::style::{Display, StyleProperty, Unit, Value};
use crate::common::geo::{Coordinate, Rect};
use crate::layouter::box_model::{BoxModel, Edges};
use crate::layouter::{LayoutElementId, LayoutElementNode, LayoutTree};
use std::collections::HashMap;
use std::sync::Arc;

/// Adapter that bridges `gosub_lattice`'s `TableTree` with the render pipeline's
/// `LayoutTree`/`PipelineDocument`. Layout results are staged in `pending` and
/// converted to absolute `BoxModel`s by `apply_positions()` after
/// `compute_table_layout` returns.
pub struct PipelineTableTree<'a> {
    doc: &'a dyn PipelineDocument,
    layout_tree: &'a mut LayoutTree,
    dom_to_layout: &'a HashMap<DomNodeId, LayoutElementId>,
    /// Relative CellLayouts written by `compute_table_layout`.
    pending: HashMap<DomNodeId, CellLayout>,
}

impl<'a> PipelineTableTree<'a> {
    pub fn new(
        doc: &'a dyn PipelineDocument,
        layout_tree: &'a mut LayoutTree,
        dom_to_layout: &'a HashMap<DomNodeId, LayoutElementId>,
    ) -> Self {
        Self {
            doc,
            layout_tree,
            dom_to_layout,
            pending: HashMap::new(),
        }
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
            &pending,
            self.dom_to_layout,
            &mut self.layout_tree.arena,
        );
    }
}

fn apply_recursive(
    doc: &dyn PipelineDocument,
    id: DomNodeId,
    parent_abs: Coordinate,
    pending: &HashMap<DomNodeId, CellLayout>,
    dom_to_layout: &HashMap<DomNodeId, LayoutElementId>,
    arena: &mut HashMap<LayoutElementId, LayoutElementNode>,
) {
    for child_id in doc.children(id) {
        match pending.get(&child_id) {
            None => {
                // Node not positioned by lattice — recurse in case descendants are.
                apply_recursive(doc, child_id, parent_abs, pending, dom_to_layout, arena);
            }
            Some(cell_layout) => {
                let abs = Coordinate::new(
                    parent_abs.x + cell_layout.position.x as f64,
                    parent_abs.y + cell_layout.position.y as f64,
                );
                if let Some(&layout_id) = dom_to_layout.get(&child_id) {
                    if let Some(element) = arena.get_mut(&layout_id) {
                        element.box_model = cell_layout_to_box_model(cell_layout, abs);
                    }
                }
                apply_recursive(doc, child_id, abs, pending, dom_to_layout, arena);
            }
        }
    }
}

fn cell_layout_to_box_model(layout: &CellLayout, abs: Coordinate) -> BoxModel {
    let border_box = Rect::new(
        abs.x,
        abs.y,
        layout.size.width as f64,
        layout.size.height as f64,
    );
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
            // Keyword-only properties: map via get_style to get inherited value
            CssProp::BorderCollapse
            | CssProp::BorderSpacingX
            | CssProp::BorderSpacingY
            | CssProp::TableLayout
            | CssProp::VerticalAlign
            | CssProp::CaptionSide => return CssLength::Auto,
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

    fn layout_cell(&mut self, id: DomNodeId, _available_width: f32) -> f32 {
        // Re-use the content height from the Taffy first pass, which correctly
        // measured text via Parley. This is an approximation — cell content
        // was measured in a flex context rather than block — but it is far better
        // than 0 and covers the most common case (single column of text).
        if let Some(&layout_id) = self.dom_to_layout.get(&id) {
            if let Some(element) = self.layout_tree.arena.get(&layout_id) {
                return element.box_model.content_box.height as f32;
            }
        }
        0.0
    }
}

/// Post-process all `display: table` nodes in the layout tree after the
/// Taffy first pass. Correct positions are written back via `gosub_lattice`.
pub fn post_process_tables(
    layout_tree: &mut LayoutTree,
    dom_to_layout: &HashMap<DomNodeId, LayoutElementId>,
) {
    // Clone the doc Arc up front so we don't hold a borrow on layout_tree
    // when we later pass it mutably to PipelineTableTree.
    let doc: Arc<dyn PipelineDocument> = Arc::clone(&layout_tree.render_tree.doc);

    // Collect table nodes without holding a borrow on layout_tree.
    let table_nodes: Vec<(DomNodeId, LayoutElementId)> = layout_tree
        .arena
        .values()
        .filter_map(|el| {
            let dom_id = el.dom_node_id;
            if matches!(
                doc.get_own_style(dom_id, &StyleProperty::Display),
                Some(Value::Display(Display::Table))
            ) {
                Some((dom_id, el.id))
            } else {
                None
            }
        })
        .collect();

    for (table_dom_id, table_layout_id) in table_nodes {
        let available_width = layout_tree
            .arena
            .get(&table_layout_id)
            .map(|e| e.box_model.content_box.width as f32)
            .unwrap_or(0.0);

        // &*doc borrows from our local Arc, not from layout_tree — no conflict.
        let mut tree = PipelineTableTree::new(&*doc, layout_tree, dom_to_layout);

        match gosub_lattice::compute_table_layout(&mut tree, table_dom_id, available_width, None) {
            Ok((_, table_height)) => {
                tree.apply_positions(table_dom_id);
                // Update the table node's own height with the lattice result.
                if let Some(el) = layout_tree.arena.get_mut(&table_layout_id) {
                    let bb = el.box_model.border_box;
                    el.box_model = BoxModel::new(
                        Rect::new(bb.x, bb.y, bb.width, table_height as f64),
                        el.box_model.padding,
                        el.box_model.border,
                        el.box_model.margin,
                    );
                }
            }
            Err(e) => {
                log::warn!("Table layout failed for node {:?}: {:?}", table_dom_id, e);
            }
        }
    }
}
