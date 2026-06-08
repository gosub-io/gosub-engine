use crate::grid::SectionGrid;
use crate::types::{CssProp, CssLength};
use crate::TableTree;

/// Compute column widths for a table with `n_cols` columns.
///
/// **Phase 1 — fixed layout only.**
///
/// Algorithm:
/// 1. The available space is `table_width` minus the horizontal border-spacing
///    gutters (one between each pair of columns plus the outer two).
/// 2. We scan the first non-empty row across all provided grids (header first,
///    then body, then footer).  For each single-column cell in that row, if
///    it has an explicit `width` in pixels, that width is assigned to its column.
/// 3. Remaining space is distributed equally among columns without an explicit
///    width.
pub fn compute_column_widths<T: TableTree>(
    tree: &T,
    n_cols: usize,
    table_width: f32,
    border_spacing_x: f32,
    grids: &[&SectionGrid<T::NodeId>],
) -> Vec<f32> {
    if n_cols == 0 {
        return Vec::new();
    }

    // Total space consumed by border-spacing gutters.
    let spacing_total = (n_cols as f32 + 1.0) * border_spacing_x;
    let available = (table_width - spacing_total).max(0.0);

    let mut explicit: Vec<Option<f32>> = vec![None; n_cols];

    // Find the first row that contains at least one cell.
    'outer: for grid in grids {
        for row_idx in 0..grid.n_rows {
            let mut found_any = false;
            for cell in grid.cells_in_row(row_idx) {
                found_any = true;
                // Only single-column cells determine column widths in fixed layout.
                if cell.colspan == 1 && explicit[cell.col].is_none() {
                    if let CssLength::Px(px) = tree.css_length(cell.node, CssProp::Width) {
                        explicit[cell.col] = Some(px);
                    }
                }
            }
            if found_any {
                break 'outer;
            }
        }
    }

    let fixed_total: f32 = explicit.iter().filter_map(|&w| w).sum();
    let auto_count = explicit.iter().filter(|w| w.is_none()).count();
    let remaining = (available - fixed_total).max(0.0);
    let auto_width = if auto_count > 0 {
        remaining / auto_count as f32
    } else {
        0.0
    };

    explicit.iter().map(|w| w.unwrap_or(auto_width)).collect()
}
