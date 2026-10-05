use std::collections::HashMap;

use crate::grid::{PlacedCell, SectionGrid};
use crate::types::{BoxEdges, CollapsedBorders, CssLength, CssProp};
use crate::TableTree;

/// Compute the height of each row in a section.
///
/// A row starts at its own explicit CSS `height`: CSS 2 §17.5.3 makes a row the tallest of that,
/// its cells' heights and what their content needs, so the cells can only grow it. That is what
/// keeps an empty `<tr style="height: 5px">` spacer row 5px tall.
///
/// Pass 1 - non-spanning cells:
/// 1. Call [`TableTree::layout_cell`] to let the implementor run normal layout
///    (block/flex/inline) inside the cell and get the actual content height.
/// 2. Also read any explicit CSS `height` on the cell.
/// 3. Take the maximum of the two, add the cell's own border + padding, and
///    use that as the candidate height for the row.
///
/// Pass 2 - cells with `rowspan > 1`, shortest spans first: if the cell needs
/// more than its spanned rows (plus the gutters between them) currently offer,
/// the deficit is distributed equally over those rows.
///
/// Every measured content height is recorded in `content_heights`, keyed by
/// cell node - `place_cell` uses it to resolve `vertical-align`.
#[allow(clippy::too_many_arguments)]
pub fn compute_row_heights<T: TableTree>(
    tree: &mut T,
    grid: &SectionGrid<T::NodeId>,
    col_widths: &[f64],
    spacing_x: f64,
    spacing_y: f64,
    content_heights: &mut HashMap<T::NodeId, f64>,
    collapsed_borders: &HashMap<T::NodeId, CollapsedBorders>,
    baseline_shifts: &mut HashMap<T::NodeId, f64>,
) -> Vec<f64> {
    let mut heights: Vec<f64> = grid
        .row_nodes
        .iter()
        .map(|&node| match node.map(|node| tree.css_length(node, CssProp::Height)) {
            Some(CssLength::Px(px)) => px.max(0.0),
            _ => 0.0,
        })
        .collect();

    // Baseline alignment (CSS 2 §17.5.3): cells with `vertical-align: baseline` share a
    // row baseline - the deepest first-line baseline among them; shallower cells shift
    // down by the difference, which can grow the row.
    let mut measured: Vec<(&PlacedCell<T::NodeId>, f64)> = Vec::new();
    let mut row_baseline = vec![0.0_f64; grid.n_rows];
    let mut cell_baselines: HashMap<T::NodeId, f64> = HashMap::new();

    for cell in grid.cells() {
        if cell.rowspan != 1 {
            continue;
        }

        let (content_h, cell_h) = measure_cell(tree, cell, col_widths, spacing_x, collapsed_borders);
        content_heights.insert(cell.node, content_h);
        measured.push((cell, cell_h));

        if tree.vertical_align(cell.node) == crate::types::VerticalAlign::Baseline {
            if let Some(b) = tree.cell_baseline(cell.node) {
                cell_baselines.insert(cell.node, b);
                row_baseline[cell.row] = row_baseline[cell.row].max(b);
            }
        }
    }

    for (cell, cell_h) in measured {
        let shift = cell_baselines
            .get(&cell.node)
            .map(|b| (row_baseline[cell.row] - b).max(0.0))
            .unwrap_or(0.0);
        if shift > 0.0 {
            baseline_shifts.insert(cell.node, shift);
        }
        let effective = cell_h + shift;
        if effective > heights[cell.row] {
            heights[cell.row] = effective;
        }
    }

    // Spanning cells, shortest spans first so nested spans stack predictably.
    let mut spanning: Vec<&PlacedCell<T::NodeId>> = grid.cells().iter().filter(|c| c.rowspan > 1).collect();
    spanning.sort_by_key(|c| c.rowspan);

    for cell in spanning {
        let (content_h, cell_h) = measure_cell(tree, cell, col_widths, spacing_x, collapsed_borders);
        content_heights.insert(cell.node, content_h);

        let span = cell.row..(cell.row + cell.rowspan).min(heights.len());
        let n_rows = span.len();
        if n_rows == 0 {
            continue;
        }
        let current: f64 = heights[span.clone()].iter().sum::<f64>() + spacing_y * n_rows.saturating_sub(1) as f64;
        if cell_h > current {
            let add = (cell_h - current) / n_rows as f64;
            for h in &mut heights[span] {
                *h += add;
            }
        }
    }

    heights
}

/// Lay out one cell's children at its final column width and return
/// `(content_height, border_box_height)`, honouring an explicit CSS `height`
/// as a minimum. Collapsed cells measure with their half-width layout borders.
fn measure_cell<T: TableTree>(
    tree: &mut T,
    cell: &PlacedCell<T::NodeId>,
    col_widths: &[f64],
    spacing_x: f64,
    collapsed_borders: &HashMap<T::NodeId, CollapsedBorders>,
) -> (f64, f64) {
    let border = effective_border(tree, cell.node, collapsed_borders);
    let padding = read_padding(tree, cell.node);

    // Inner width available to the cell's children: the spanned columns plus
    // the gutters a colspan cell runs across, minus the cell's own edges.
    let spanned = col_widths.get(cell.col..cell.col + cell.colspan).unwrap_or(&[]);
    let cell_col_w: f64 = spanned.iter().sum::<f64>() + spacing_x * spanned.len().saturating_sub(1) as f64;
    let inner_w = (cell_col_w - border.horizontal() - padding.horizontal()).max(0.0);

    // Ask the implementor to lay out the cell's children and report their height.
    let content_h = tree.layout_cell(cell.node, inner_w);

    // Explicit CSS `height` is a minimum - content can be taller.
    let explicit_h = match tree.css_length(cell.node, CssProp::Height) {
        CssLength::Px(px) => px,
        _ => 0.0,
    };

    let cell_h = content_h.max(explicit_h) + border.vertical() + padding.vertical();
    (content_h, cell_h)
}

// Helpers shared with compute.rs

/// The border widths that actually occupy layout space: under
/// `border-collapse` that is half the resolved boundary width per edge
/// (borders center on the grid lines); otherwise the cell's CSS borders.
pub(crate) fn effective_border<T: TableTree>(
    tree: &T,
    node: T::NodeId,
    collapsed_borders: &HashMap<T::NodeId, CollapsedBorders>,
) -> BoxEdges {
    match collapsed_borders.get(&node) {
        Some(cb) => cb.layout,
        None => read_border(tree, node),
    }
}

pub(crate) fn read_border<T: TableTree>(tree: &T, node: T::NodeId) -> BoxEdges {
    BoxEdges {
        top: tree.css_length(node, CssProp::BorderTopWidth).px_or(0.0),
        right: tree.css_length(node, CssProp::BorderRightWidth).px_or(0.0),
        bottom: tree.css_length(node, CssProp::BorderBottomWidth).px_or(0.0),
        left: tree.css_length(node, CssProp::BorderLeftWidth).px_or(0.0),
    }
}

/// [`read_padding`] for a box whose padding may be a percentage, as a table's may: CSS resolves
/// those against the containing block's width on every side, which is `basis`.
pub(crate) fn read_padding_against<T: TableTree>(tree: &T, node: T::NodeId, basis: f64) -> BoxEdges {
    let side = |prop| tree.css_length(node, prop).resolve(basis).unwrap_or(0.0);
    BoxEdges {
        top: side(CssProp::PaddingTop),
        right: side(CssProp::PaddingRight),
        bottom: side(CssProp::PaddingBottom),
        left: side(CssProp::PaddingLeft),
    }
}

pub(crate) fn read_padding<T: TableTree>(tree: &T, node: T::NodeId) -> BoxEdges {
    BoxEdges {
        top: tree.css_length(node, CssProp::PaddingTop).px_or(0.0),
        right: tree.css_length(node, CssProp::PaddingRight).px_or(0.0),
        bottom: tree.css_length(node, CssProp::PaddingBottom).px_or(0.0),
        left: tree.css_length(node, CssProp::PaddingLeft).px_or(0.0),
    }
}

/// One row group's grid and row heights, as [`distribute_table_height`] sees them.
pub struct SectionRows<'a, N> {
    pub grid: &'a SectionGrid<N>,
    pub heights: &'a mut Vec<f64>,
    /// A `tbody` (or the anonymous group bare rows go in), as opposed to a `thead`/`tfoot`.
    pub body: bool,
}

/// Grow the rows so the grid reaches the table's specified height.
///
/// CSS 2 §17.5.3 makes a table's `height` a minimum for its rows but leaves open where the extra
/// goes. This follows what Chromium does:
///
/// 1. A row with a percentage height grows to that share of the table height.
/// 2. What is left is shared over the `tbody` sections (all sections when there is none), in
///    proportion to their heights, or equally when those are all zero.
/// 3. Inside a section it goes to the auto-height rows that hold cells, else to the auto-height
///    rows without, else to every row - in proportion to their heights, or equally.
///
/// A row with its own `height`, or holding a single-row cell with one, is not auto-height. A
/// section with no rows cannot take its share, so that comes back as empty space for the caller
/// to add below the grid, as it does for a table with nothing but a caption.
///
/// `target` is the height the grid itself is to reach, gutters included and without the table's
/// own border and padding (or perimeter border halves). Returns the unplaced extra.
pub fn distribute_table_height<T: TableTree>(
    tree: &T,
    sections: &mut [SectionRows<'_, T::NodeId>],
    target: f64,
    spacing_y: f64,
) -> f64 {
    // The same stack `compute_table_layout` builds: one gutter above every group and one
    // below the last, and one between adjacent rows inside a group.
    let extent = spacing_y
        + sections
            .iter()
            .map(|s| s.heights.iter().sum::<f64>() + s.heights.len().saturating_sub(1) as f64 * spacing_y + spacing_y)
            .sum::<f64>();
    let mut excess = target - extent;
    if excess <= 0.0 {
        return 0.0;
    }

    for section in sections.iter_mut() {
        for (row, height) in section.heights.iter_mut().enumerate() {
            let pct = section.grid.row_nodes[row].map(|node| tree.css_length(node, CssProp::Height));
            if let Some(CssLength::Percent(p)) = pct {
                let grow = (p / 100.0 * target - *height).clamp(0.0, excess);
                *height += grow;
                excess -= grow;
            }
        }
    }
    if excess <= 0.0 {
        return 0.0;
    }

    let mut takers: Vec<usize> = (0..sections.len()).filter(|&i| sections[i].body).collect();
    if takers.is_empty() {
        takers = (0..sections.len()).collect();
    }
    if takers.is_empty() {
        return excess;
    }
    let weights: Vec<f64> = takers.iter().map(|&i| sections[i].heights.iter().sum()).collect();

    let mut unplaced = 0.0;
    for (&i, share) in takers.iter().zip(shares(&weights, excess)) {
        let section = &mut sections[i];
        if section.heights.is_empty() {
            unplaced += share;
        } else {
            grow_section_rows(tree, section, share);
        }
    }
    unplaced
}

/// Give `extra` to one section's rows, auto-height rows with cells first (see
/// [`distribute_table_height`]).
fn grow_section_rows<T: TableTree>(tree: &T, section: &mut SectionRows<'_, T::NodeId>, extra: f64) {
    let n = section.heights.len();
    let mut has_cells = vec![false; n];
    let mut constrained: Vec<bool> = section
        .grid
        .row_nodes
        .iter()
        .map(|&node| node.is_some_and(|node| !matches!(tree.css_length(node, CssProp::Height), CssLength::Auto)))
        .collect();
    for cell in section.grid.cells() {
        for flag in &mut has_cells[cell.row..(cell.row + cell.rowspan).min(n)] {
            *flag = true;
        }
        if cell.rowspan == 1 && matches!(tree.css_length(cell.node, CssProp::Height), CssLength::Px(_)) {
            constrained[cell.row] = true;
        }
    }

    let auto_with_cells: Vec<usize> = (0..n).filter(|&r| !constrained[r] && has_cells[r]).collect();
    let auto_without: Vec<usize> = (0..n).filter(|&r| !constrained[r] && !has_cells[r]).collect();
    let rows = if !auto_with_cells.is_empty() {
        auto_with_cells
    } else if !auto_without.is_empty() {
        auto_without
    } else {
        (0..n).collect()
    };

    let weights: Vec<f64> = rows.iter().map(|&r| section.heights[r]).collect();
    for (&r, share) in rows.iter().zip(shares(&weights, extra)) {
        section.heights[r] += share;
    }
}

/// Split `total` in proportion to `weights`, or equally when they are all zero.
fn shares(weights: &[f64], total: f64) -> Vec<f64> {
    let sum: f64 = weights.iter().sum();
    if sum > 0.0 {
        weights.iter().map(|w| total * w / sum).collect()
    } else {
        vec![total / weights.len().max(1) as f64; weights.len()]
    }
}
