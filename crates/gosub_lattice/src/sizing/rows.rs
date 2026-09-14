use std::collections::HashMap;

use crate::grid::{PlacedCell, SectionGrid};
use crate::types::{BoxEdges, CollapsedBorders, CssLength, CssProp};
use crate::TableTree;

/// Compute the height of each row in a section.
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
    col_widths: &[f32],
    spacing_x: f32,
    spacing_y: f32,
    content_heights: &mut HashMap<T::NodeId, f32>,
    collapsed_borders: &HashMap<T::NodeId, CollapsedBorders>,
    baseline_shifts: &mut HashMap<T::NodeId, f32>,
) -> Vec<f32> {
    let mut heights = vec![0.0_f32; grid.n_rows];

    // Baseline alignment (CSS 2 §17.5.3): cells with `vertical-align: baseline` share a
    // row baseline - the deepest first-line baseline among them; shallower cells shift
    // down by the difference, which can grow the row.
    let mut measured: Vec<(&PlacedCell<T::NodeId>, f32)> = Vec::new();
    let mut row_baseline = vec![0.0_f32; grid.n_rows];
    let mut cell_baselines: HashMap<T::NodeId, f32> = HashMap::new();

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
        let current: f32 = heights[span.clone()].iter().sum::<f32>() + spacing_y * n_rows.saturating_sub(1) as f32;
        if cell_h > current {
            let add = (cell_h - current) / n_rows as f32;
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
    col_widths: &[f32],
    spacing_x: f32,
    collapsed_borders: &HashMap<T::NodeId, CollapsedBorders>,
) -> (f32, f32) {
    let border = effective_border(tree, cell.node, collapsed_borders);
    let padding = read_padding(tree, cell.node);

    // Inner width available to the cell's children: the spanned columns plus
    // the gutters a colspan cell runs across, minus the cell's own edges.
    let spanned = col_widths.get(cell.col..cell.col + cell.colspan).unwrap_or(&[]);
    let cell_col_w: f32 = spanned.iter().sum::<f32>() + spacing_x * spanned.len().saturating_sub(1) as f32;
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

pub(crate) fn read_padding<T: TableTree>(tree: &T, node: T::NodeId) -> BoxEdges {
    BoxEdges {
        top: tree.css_length(node, CssProp::PaddingTop).px_or(0.0),
        right: tree.css_length(node, CssProp::PaddingRight).px_or(0.0),
        bottom: tree.css_length(node, CssProp::PaddingBottom).px_or(0.0),
        left: tree.css_length(node, CssProp::PaddingLeft).px_or(0.0),
    }
}
