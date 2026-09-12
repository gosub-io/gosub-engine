use gosub_lattice::{CellLayout, CssLength, CssProp, TableRole, TableTree};

use crate::common::document::node::{NodeId as DomNodeId, NodeType};
use crate::common::document::pipeline_doc::PipelineDocument;
use crate::common::document::style::{Display, StyleProperty, Unit, Value};
use crate::common::geo::{Coordinate, Rect};
use crate::layouter::box_model::{BoxModel, Edges};
use crate::layouter::float::float_side;
use crate::layouter::taffy::MAX_CONTENT_WIDTH;
use crate::layouter::text::get_text_layout;
use crate::layouter::{ElementContext, LayoutElementId, LayoutElementNode, LayoutTree};
use gosub_interface::font_system::FontSystem;
use parking_lot::Mutex;
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
    /// Border-box width the column algorithm gave each cell, harvested for the layouter to pin on
    /// the next pass. Taffy sizes a cell by flex before the columns are known, so its width is
    /// indefinite and the anonymous line boxes inside it have nothing to wrap against - the text
    /// then takes its max-content width and runs past the cell. Feeding the settled width back is
    /// the same trick already used for the table's own width.
    cell_widths: HashMap<DomNodeId, f32>,
    /// The font system the layouter measured with, so a cell's min-content width can be asked of
    /// the same shaper. A column may not be narrower than its widest unbreakable word, and that
    /// width is not recoverable from the laid-out boxes: a text box reports the width it *was*
    /// given, not the width it needs.
    font_system: Arc<Mutex<dyn FontSystem>>,
    /// Memo for `cell_min_content_width`, which is asked once per cell per lattice run and would
    /// otherwise re-shape every word of every table on the page each time.
    min_content_cache: HashMap<DomNodeId, f32>,
    /// Cell widths the previous pass settled on and the layouter pinned on the taffy boxes.
    /// Empty on the first pass.
    pinned: &'a HashMap<DomNodeId, f32>,
}

impl<'a> PipelineTableTree<'a> {
    pub fn new(
        doc: &'a dyn PipelineDocument,
        layout_tree: &'a mut LayoutTree,
        dom_to_layout: &'a HashMap<DomNodeId, LayoutElementId>,
        font_system: Arc<Mutex<dyn FontSystem>>,
        pinned: &'a HashMap<DomNodeId, f32>,
    ) -> Self {
        Self {
            doc,
            layout_tree,
            dom_to_layout,
            pending: HashMap::new(),
            cell_widths: HashMap::new(),
            font_system,
            min_content_cache: HashMap::new(),
            pinned,
        }
    }

    /// Widest unbreakable run of content in a layout subtree - its min-content width.
    ///
    /// Text contributes its longest word, measured unconstrained: that is the narrowest a text box
    /// can be without the shaper breaking inside a word. A replaced element contributes its whole
    /// border-box width, since an image has no break opportunities at all. The laid-out boxes
    /// cannot answer this - a text box carries the width it was allotted, which may be anything
    /// from one word to the whole run - so the words are re-measured through the same font system
    /// the layouter used.
    fn subtree_min_content_width(&mut self, id: LayoutElementId) -> f32 {
        let Some(el) = self.layout_tree.arena.get(&id) else {
            return 0.0;
        };
        match &el.context {
            ElementContext::Text(text_ctx) => {
                let (text, font_info) = (text_ctx.text.clone(), text_ctx.font_info.clone());
                let mut fs = self.font_system.lock();
                text.split_ascii_whitespace()
                    .filter_map(|word| get_text_layout(word, &font_info, MAX_CONTENT_WIDTH, &mut *fs).ok())
                    .map(|d| d.width as f32)
                    .fold(0.0_f32, f32::max)
            }
            ElementContext::Image(_) | ElementContext::Svg(_) => el.box_model.border_box.width as f32,
            ElementContext::None => {
                let children = el.children.clone();
                children
                    .into_iter()
                    .map(|cid| self.subtree_min_content_width(cid))
                    .fold(0.0_f32, f32::max)
            }
        }
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
            &pending,
            self.dom_to_layout,
            self.layout_tree,
        );
    }
}

/// Walks the DOM under `id` looking for nodes lattice gave a position to, and moves each one -
/// with everything laid out inside it - to where lattice put it.
///
/// The move is done on the *layout* tree, not by walking the DOM again. A text node is laid out
/// as one box per word and `dom_to_layout` deliberately holds none of them (its comment says so:
/// one text node, many word boxes, one slot), and inline content sits under anonymous wrappers
/// that have no DOM node at all. Translating what the DOM walk could reach therefore left every
/// caption's and cell's text behind at its old position while the box moved out from under it.
/// `shift_subtree` moves the whole layout subtree, which is exactly the set of boxes that should
/// travel with the node.
fn apply_recursive(
    doc: &dyn PipelineDocument,
    id: DomNodeId,
    parent_abs: Coordinate,
    pending: &HashMap<DomNodeId, CellLayout>,
    dom_to_layout: &HashMap<DomNodeId, LayoutElementId>,
    layout_tree: &mut LayoutTree,
) {
    for child_id in doc.children(id) {
        let Some(cell_layout) = pending.get(&child_id) else {
            // Not positioned by lattice: an ancestor's shift has already carried it along, so
            // only keep looking for positioned nodes deeper down.
            apply_recursive(doc, child_id, parent_abs, pending, dom_to_layout, layout_tree);
            continue;
        };

        let abs = Coordinate::new(
            parent_abs.x + cell_layout.position.x as f64,
            parent_abs.y + cell_layout.position.y as f64,
        );

        if let Some(&layout_id) = dom_to_layout.get(&child_id) {
            // Read the old origin before moving, so the subtree travels by the same delta.
            // A descendant that lattice positions too is shifted here and then set absolutely by
            // its own turn below, which lands it in the same place either way.
            if let Some(element) = layout_tree.arena.get(&layout_id) {
                let old = element.box_model.border_box;
                layout_tree.shift_subtree(layout_id, abs.x - old.x, abs.y - old.y);
            }
            if let Some(element) = layout_tree.arena.get_mut(&layout_id) {
                element.box_model = cell_layout_to_box_model(cell_layout, abs);
            }
        }

        apply_recursive(doc, child_id, abs, pending, dom_to_layout, layout_tree);
    }
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

/// Returns the intrinsic content width of a layout subtree - the actual measured
/// width of text/image leaf nodes, not the container's allocated width.
///
/// Text leaf nodes carry the Parley-measured line width (e.g. "1." → ~20 px),
/// which is much narrower than the equal-distributed Taffy cell width.
/// This lets `compute_column_widths` keep narrow structural columns narrow.
fn intrinsic_content_width(el: &LayoutElementNode, arena: &HashMap<LayoutElementId, LayoutElementNode>) -> f32 {
    match &el.context {
        ElementContext::Text(_) => el.box_model.content_box.width as f32,
        // Replaced elements: use the laid-out border-box width so the column is wide enough for
        // the image *including its own CSS border* (the bare `dimension` omits it). Images are
        // never stretched to the cell width, so the border box is the true intrinsic width.
        ElementContext::Image(_) | ElementContext::Svg(_) => el.box_model.border_box.width as f32,
        ElementContext::None => {
            let from_children = el
                .children
                .iter()
                .filter_map(|&cid| arena.get(&cid))
                .map(|child| intrinsic_content_width(child, arena))
                .fold(0.0f32, f32::max);
            if from_children > 0.0 {
                return from_children;
            }
            // Nothing measurable underneath: an inline box's children are laid out inside an
            // anonymous wrapper that has no `LayoutElementNode`, so the walk bottoms out at
            // zero even though the box itself was measured. Wikipedia thumbnails hit this - the
            // image sits inside an `<a>`, so the cell reported no width at all and the table
            // collapsed to its border-spacing.
            el.box_model.border_box.width as f32
        }
    }
}

impl TableTree for PipelineTableTree<'_> {
    type NodeId = DomNodeId;

    fn children(&self, id: DomNodeId) -> Vec<DomNodeId> {
        // Whitespace between table-internal boxes is discarded (CSS 2.1 §17.2.1). The newline and
        // indentation between two `<tr>`s is a text node like any other, and since a run of
        // non-table children is wrapped in an anonymous cell, keeping them invented a row made of
        // nothing but indentation - which then became the first row the column scan found, so the
        // real cells' widths were never measured and every column fell back to an equal share.
        self.doc
            .children(id)
            .into_iter()
            .filter(|child| match self.doc.get_node_by_id(*child) {
                Some(node) => match &node.node_type {
                    NodeType::Text(text) => !text.trim_matches(|c: char| c.is_ascii_whitespace()).is_empty(),
                    _ => true,
                },
                None => true,
            })
            .collect()
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
        if self.table_role(id) == TableRole::Cell {
            self.cell_widths.insert(id, layout.size.width);
        }
        self.pending.insert(id, layout);
    }

    fn layout_cell(&mut self, id: DomNodeId, _available_width: f32) -> f32 {
        // Re-use the content height from the Taffy first pass, which correctly
        // measured text via Parley. This is an approximation - cell content
        // was measured in a flex context rather than block - but it is far better
        // than 0 and covers the most common case (single column of text).
        if let Some(&layout_id) = self.dom_to_layout.get(&id) {
            if let Some(element) = self.layout_tree.arena.get(&layout_id) {
                let taffy_h = element.box_model.content_box.height as f32;
                // A cell containing a nested table must be at least as tall as that table.
                // The nested table's real height is only known after lattice lays it out, so
                // the second (bottom-up) pass in `post_process_tables` propagates it up here.
                return taffy_h.max(self.nested_table_height(layout_id));
            }
        }
        0.0
    }

    fn table_shrink_to_fit(&self, id: DomNodeId) -> bool {
        // A float is always shrink-to-fit, and it is the case that matters here: Wikipedia
        // thumbnails are `figure { display: table; float: right }`, and stretching them to the
        // article column's width is what pushed their captions across the text.
        float_side(self.doc, id).is_some()
    }

    fn caption_at_bottom(&self, id: DomNodeId) -> bool {
        matches!(
            self.doc.get_style(id, &StyleProperty::CaptionSide),
            Value::Keyword(kw) if crate::common::document::style::lookup(kw) == "bottom"
        )
    }

    fn caption_height(&mut self, id: DomNodeId, _width: f32) -> f32 {
        // Measured, not re-laid-out: the caption's height comes from the taffy pass, as cell
        // heights do. The layouter re-runs that pass with the table's computed width pinned on
        // the box, so by the second pass the measurement is the one taken at `width`.
        self.dom_to_layout
            .get(&id)
            .and_then(|layout_id| self.layout_tree.arena.get(layout_id))
            .map(|el| el.box_model.margin_box.height as f32)
            .unwrap_or(0.0)
    }

    fn cell_min_content_width(&mut self, id: DomNodeId) -> f32 {
        if let Some(&cached) = self.min_content_cache.get(&id) {
            return cached;
        }
        let width = match self.dom_to_layout.get(&id).copied() {
            Some(layout_id) => {
                let pad = self
                    .layout_tree
                    .arena
                    .get(&layout_id)
                    .map(|el| (el.box_model.padding.left + el.box_model.padding.right) as f32)
                    .unwrap_or(0.0);
                self.subtree_min_content_width(layout_id) + pad
            }
            None => 0.0,
        };
        self.min_content_cache.insert(id, width);
        width
    }

    fn cell_content_width(&self, id: DomNodeId) -> f32 {
        // Once a width has been pinned, that *is* this cell's settled width - report it rather
        // than re-measuring. The pin makes taffy lay the contents out at the column width, so a
        // fresh measurement comes back as "whatever it was given" and the algorithm re-derives a
        // narrower column from its own previous answer. The contents were then laid out a few
        // pixels wider than the cell they ended up in, which is what still clipped the last of the
        // table headers.
        if let Some(&pinned) = self.pinned.get(&id) {
            return pinned;
        }
        if let Some(&layout_id) = self.dom_to_layout.get(&id) {
            if let Some(element) = self.layout_tree.arena.get(&layout_id) {
                // Include the cell's own horizontal padding so the column is wide enough to hold
                // the content *and* its padding (e.g. HN's logo cell: 20px image + 4px padding-right).
                let pad = (element.box_model.padding.left + element.box_model.padding.right) as f32;
                return intrinsic_content_width(element, &self.layout_tree.arena) + pad;
            }
        }
        0.0
    }
}

/// What a table pass accumulates as it goes: the border-box widths to pin on the next pass, and
/// how much taller lattice made each table than the box taffy sized everything around it against.
struct TablePassOutput {
    settled: HashMap<DomNodeId, f32>,
    growth: HashMap<LayoutElementId, f64>,
}

/// What a table pass needs beyond the trees themselves: the shaper to ask for min-content widths,
/// and the cell widths the previous pass settled on (empty on the first pass).
#[derive(Clone, Copy)]
pub struct TablePassInputs<'a> {
    pub font_system: &'a Arc<Mutex<dyn FontSystem>>,
    pub pinned: &'a HashMap<DomNodeId, f32>,
}

/// Post-process all `display: table` nodes in the layout tree after the
/// Taffy first pass. Correct positions are written back via `gosub_lattice`.
pub fn post_process_tables(
    layout_tree: &mut LayoutTree,
    dom_to_layout: &HashMap<DomNodeId, LayoutElementId>,
    inputs: TablePassInputs<'_>,
) -> HashMap<DomNodeId, f32> {
    // Clone the doc Arc up front so we don't hold a borrow on layout_tree
    // when we later pass it mutably to PipelineTableTree.
    let doc: Arc<dyn PipelineDocument> = Arc::clone(&layout_tree.render_tree.doc);

    // Collect table nodes in pre-order DOM traversal so outer tables are always
    // processed before any nested tables they contain. This is required so that
    // when we process an inner table, the parent cell's box model has already
    // been updated by the outer table's apply_positions call.
    let mut table_nodes: Vec<(DomNodeId, LayoutElementId)> = Vec::new();
    if let Some(root_dom_id) = doc.root() {
        collect_tables_preorder(&*doc, root_dom_id, dom_to_layout, &mut table_nodes);
    }

    log::info!("lattice: post_process_tables found {} table node(s)", table_nodes.len());

    // Two passes. Pass 1 is pre-order (outer→inner): it establishes column widths, which flow
    // top-down (a nested table reads its width from its already-sized parent cell). Pass 2 is
    // post-order (inner→outer): each table is re-laid-out *after* the tables nested inside its
    // cells, so an outer cell's height now reflects its nested table's true height - height
    // flows bottom-up. A single reverse pass propagates through any table-nesting depth.
    let mut out = TablePassOutput {
        settled: HashMap::new(),
        growth: HashMap::new(),
    };
    for pass in 0..2 {
        let order: Vec<(DomNodeId, LayoutElementId)> = if pass == 0 {
            table_nodes.clone()
        } else {
            table_nodes.iter().rev().copied().collect()
        };
        for (table_dom_id, table_layout_id) in order {
            if let Some(size) = lay_out_one_table(
                &*doc,
                layout_tree,
                dom_to_layout,
                table_dom_id,
                table_layout_id,
                &mut out,
                inputs,
            ) {
                out.settled.insert(table_dom_id, size);
            }
        }
    }

    // Every table has its settled height by now; the blocks around them have not heard about it.
    // Innermost first, so an outer table absorbs a nested one's growth as part of its own.
    for (table_dom_id, table_layout_id) in table_nodes.iter().rev() {
        // A float or an absolutely positioned box is out of flow: what follows it in the source is
        // laid out beside or behind it, not after it, so its height must not push anything down.
        // Wikipedia's infobox and its thumbnails are floated tables, and absorbing their growth
        // shoved the whole article body 400px down the page.
        if float_side(&*doc, *table_dom_id).is_some() || is_out_of_flow(&*doc, *table_dom_id) {
            continue;
        }
        if let Some(&delta) = out.growth.get(table_layout_id) {
            absorb_table_growth(&*doc, layout_tree, *table_layout_id, delta);
        }
    }

    out.settled
}

/// Run lattice for a single table node and write the computed cell positions and the table's
/// own size back into the layout tree.
fn lay_out_one_table(
    doc: &dyn PipelineDocument,
    layout_tree: &mut LayoutTree,
    dom_to_layout: &HashMap<DomNodeId, LayoutElementId>,
    table_dom_id: DomNodeId,
    table_layout_id: LayoutElementId,
    out: &mut TablePassOutput,
    inputs: TablePassInputs<'_>,
) -> Option<f32> {
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

    let mut tree = PipelineTableTree::new(
        doc,
        layout_tree,
        dom_to_layout,
        Arc::clone(inputs.font_system),
        inputs.pinned,
    );

    match gosub_lattice::compute_table_layout(&mut tree, table_dom_id, available_width, None) {
        Ok((table_width, table_height)) => {
            // A table with no columns computes to 0x0 - honest for its own model, but it would
            // erase a box taffy had already sized and leave the children painting outside a
            // collapsed parent. Keep what taffy produced instead.
            if table_width <= 0.0 && table_height <= 0.0 {
                return None;
            }
            out.settled.extend(tree.cell_widths.drain());
            tree.apply_positions(table_dom_id);
            // Write back both dimensions so deeply-nested tables can read the
            // correct width from this table's box model via their parent lookup.
            if let Some(el) = layout_tree.arena.get_mut(&table_layout_id) {
                let bb = el.box_model.border_box;
                // A table whose rows all measure zero - every cell empty, no spacing, no borders,
                // no padding - would collapse the box taffy had already sized. The width is still
                // the column algorithm's to decide, so only the height falls back.
                let height = if table_height > 0.0 {
                    table_height as f64
                } else {
                    bb.height
                };
                // What the box grows by here is what the surrounding blocks were never told about.
                *out.growth.entry(table_layout_id).or_insert(0.0) += height - bb.height;
                el.box_model = BoxModel::new(
                    Rect::new(bb.x, bb.y, table_width as f64, height),
                    el.box_model.padding,
                    el.box_model.border,
                    el.box_model.margin,
                );
            }
            Some(table_width)
        }
        Err(e) => {
            log::warn!("lattice: table layout failed for node {:?}: {:?}", table_dom_id, e);
            None
        }
    }
}

/// Whether a sibling has to travel with a box that just grew.
///
/// `child_bottom` is where the grown box now ends and `delta` how much of that is new, so
/// `child_bottom - delta` is the edge everything around it was laid out against. Only what starts
/// at or after that edge was placed *after* the box and has to move; a sibling laid out beside it
/// (a flex row, a float, the hatnote above a table) starts earlier and stays where it is. The 1px
/// slack absorbs the rounding between a box's bottom and the next box's top.
fn sits_below(sibling_top: f64, child_bottom: f64, delta: f64) -> bool {
    sibling_top >= child_bottom - delta - 1.0
}

/// Whether a box is taken out of the normal flow by `position: absolute` or `fixed`.
fn is_out_of_flow(doc: &dyn PipelineDocument, id: DomNodeId) -> bool {
    matches!(
        doc.get_style(id, &StyleProperty::Position),
        Value::Keyword(kw) if matches!(crate::common::document::style::lookup(kw).as_str(), "absolute" | "fixed")
    )
}

/// Push a table's settled height out into the ordinary blocks around it.
///
/// A table's height is lattice's to decide - border-spacing and the row-height rules are not
/// taffy's, and a nested table's height is only known after it has been laid out - so the box ends
/// up `delta` taller than the one taffy sized everything around it against. Whatever was laid out
/// below it stayed where it was: that is what printed Wikipedia's "This section is an excerpt
/// from" line across the last row of the table above it.
///
/// The walk goes up from the table. At each level whatever sat below the box that grew moves down
/// by `delta`, and the parent grows by however much its children now overrun it - which becomes
/// the `delta` for the level above. It stops when a parent turns out to have had room to spare
/// (nothing outside it moved, so nothing outside it needs to know), at a box lattice owns - a
/// cell, row or table, whose height the table pass propagates itself - and at a box whose height
/// the author fixed, where overflowing is what the page asked for.
fn absorb_table_growth(
    doc: &dyn PipelineDocument,
    layout_tree: &mut LayoutTree,
    table_layout_id: LayoutElementId,
    delta: f64,
) {
    let mut child = table_layout_id;
    let mut delta = delta;
    loop {
        if delta <= 0.5 {
            return;
        }
        let Some(node) = layout_tree.arena.get(&child) else {
            return;
        };
        let child_bottom = node.box_model.margin_box.y + node.box_model.margin_box.height;
        let Some(parent_id) = node.parent else {
            return;
        };
        let Some(parent) = layout_tree.arena.get(&parent_id) else {
            return;
        };

        let parent_dom_id = parent.dom_node_id;
        let siblings = parent.children.clone();

        // Inside a table, heights belong to the table pass.
        if !matches!(
            doc.get_own_style(parent_dom_id, &StyleProperty::Display),
            None | Some(Value::Display(
                Display::Block
                    | Display::InlineBlock
                    | Display::Flex
                    | Display::InlineFlex
                    | Display::Grid
                    | Display::InlineGrid
            ))
        ) {
            return;
        }

        for sibling in siblings {
            if sibling == child {
                continue;
            }
            let below = layout_tree
                .arena
                .get(&sibling)
                .is_some_and(|s| sits_below(s.box_model.margin_box.y, child_bottom, delta));
            if below {
                layout_tree.shift_subtree(sibling, 0.0, delta);
            }
        }

        // An explicit height means the author chose to clip or overflow: the siblings inside have
        // moved, but the box itself does not grow and nothing outside it shifts.
        if matches!(
            doc.get_style(parent_dom_id, &StyleProperty::Height),
            Value::Unit(h, Unit::Px | Unit::Percent) if h > 0.0
        ) {
            return;
        }

        let Some(parent) = layout_tree.arena.get(&parent_id) else {
            return;
        };
        let content = parent.box_model.content_box;
        let children_bottom = parent
            .children
            .iter()
            .filter_map(|cid| layout_tree.arena.get(cid))
            .map(|c| c.box_model.margin_box.y + c.box_model.margin_box.height)
            .fold(f64::NEG_INFINITY, f64::max);
        let grow = children_bottom - (content.y + content.height);
        if !grow.is_finite() || grow <= 0.5 {
            // The parent had room to spare, so its own box is unchanged and the walk ends here.
            return;
        }

        if let Some(parent) = layout_tree.arena.get_mut(&parent_id) {
            let bm = &mut parent.box_model;
            for rect in [
                &mut bm.content_box,
                &mut bm.padding_box,
                &mut bm.border_box,
                &mut bm.margin_box,
            ] {
                rect.height += grow;
            }
        }

        child = parent_id;
        delta = grow;
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

#[cfg(test)]
mod tests {
    use super::sits_below;

    // The rule that decides what travels with a table that grew. Getting it wrong is what shoved
    // the article body down the page when the floated infobox's height was absorbed.
    #[test]
    fn only_what_was_laid_out_after_the_box_moves() {
        // A 100px-tall box that grew by 20: everything was laid out against a bottom of 80.
        let (bottom, delta) = (100.0, 20.0);

        assert!(sits_below(80.0, bottom, delta), "the block that followed it moves");
        assert!(sits_below(400.0, bottom, delta), "and so does everything after that");
        assert!(
            !sits_below(0.0, bottom, delta),
            "a sibling beside it - a flex row, a float - stays where it is"
        );
        assert!(!sits_below(40.0, bottom, delta), "so does one that overlaps it");
    }

    #[test]
    fn a_pixel_of_slack_at_the_boundary() {
        // Box bottoms and the next box's top do not land on exactly the same fraction.
        assert!(
            sits_below(79.4, 100.0, 20.0),
            "just above the edge still counts as after"
        );
        assert!(!sits_below(78.5, 100.0, 20.0), "but not a whole pixel above it");
    }
}
