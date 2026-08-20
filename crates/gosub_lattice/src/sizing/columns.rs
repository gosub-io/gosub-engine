use crate::grid::SectionGrid;
use crate::sizing::rows::{read_border, read_padding};
use crate::types::{CssLength, CssProp};
use crate::TableTree;

/// Per-column measurements. `explicit` is a specified CSS width (first one wins per column);
/// `natural` is the max-content outer width - content plus the cell's own padding and border,
/// since column widths are border-box widths - maximised over every row of every section.
struct ColumnMeasure {
    explicit: Vec<Option<f32>>,
    natural: Vec<f32>,
}

/// Scan every row of every section. `table_width` resolves percentage widths; pass `None` when
/// the table's own width is still unknown (percentages then count as auto).
fn measure_columns<T: TableTree>(
    tree: &T,
    n_cols: usize,
    table_width: Option<f32>,
    grids: &[&SectionGrid<T::NodeId>],
) -> ColumnMeasure {
    let mut explicit: Vec<Option<f32>> = vec![None; n_cols];
    let mut natural: Vec<f32> = vec![0.0; n_cols];
    for grid in grids {
        for cell in grid.cells() {
            if cell.colspan != 1 {
                continue;
            }
            let outer = tree.cell_content_width(cell.node)
                + read_padding(tree, cell.node).horizontal()
                + read_border(tree, cell.node).horizontal();
            if explicit[cell.col].is_none() {
                // A specified width cannot shrink a cell below its content's min-width
                // (CSS: used width = max(specified, min-content)). Without this, e.g. a
                // `width:18px` cell holding a 20px image clips it and eats the padding.
                match tree.css_length(cell.node, CssProp::Width) {
                    CssLength::Px(px) => explicit[cell.col] = Some(px.max(outer)),
                    CssLength::Percent(p) => {
                        if let Some(tw) = table_width {
                            explicit[cell.col] = Some((p / 100.0 * tw).max(outer));
                        }
                    }
                    _ => {}
                }
            }
            if outer > natural[cell.col] {
                natural[cell.col] = outer;
            }
        }
    }
    ColumnMeasure { explicit, natural }
}

/// The table's max-content width: every column at its widest (specified or natural), plus the
/// border-spacing gutters. An auto-width table shrinks to this when it fits the available space.
pub fn intrinsic_table_width<T: TableTree>(
    tree: &T,
    n_cols: usize,
    border_spacing_x: f32,
    grids: &[&SectionGrid<T::NodeId>],
) -> f32 {
    if n_cols == 0 {
        return 0.0;
    }
    let m = measure_columns(tree, n_cols, None, grids);
    let cols: f32 = (0..n_cols)
        .map(|c| m.explicit[c].unwrap_or(0.0).max(m.natural[c]))
        .sum();
    cols + (n_cols as f32 + 1.0) * border_spacing_x
}

/// Compute column widths for a table with `n_cols` columns.
///
/// Algorithm:
/// 1. The available space is `table_width` minus the horizontal border-spacing
///    gutters (one between each pair of columns plus the outer two).
/// 2. Measure every column across all sections ([`measure_columns`]): specified
///    CSS widths and max-content natural widths (including cell padding/border).
/// 3. Remaining space is distributed to auto columns proportionally to their
///    natural content width. Falls back to equal distribution if no content
///    width information is available.
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

    let ColumnMeasure { mut explicit, natural } = measure_columns(tree, n_cols, Some(table_width), grids);

    let fixed_total: f32 = explicit.iter().filter_map(|&w| w).sum();
    let remaining = (available - fixed_total).max(0.0);

    let auto_cols: Vec<usize> = (0..n_cols).filter(|&c| explicit[c].is_none()).collect();
    if !auto_cols.is_empty() {
        let total_natural: f32 = auto_cols.iter().map(|&c| natural[c]).sum();
        if total_natural > 0.0 {
            // Threshold-based distribution:
            //   - Narrow auto columns (intrinsic < 50 px) are structural (rank
            //     numbers, vote buttons) - give them their natural width with a
            //     14 px floor so they stay visible.
            //   - Wide auto columns are content columns - they share whatever
            //     space remains after the narrow columns have taken their share.
            //     Multiple content columns share proportionally to their natural
            //     widths; if there are none, fall through to equal distribution.
            const NARROW_THRESHOLD: f32 = 50.0;
            const NARROW_FLOOR: f32 = 14.0;

            let narrow_total: f32 = auto_cols
                .iter()
                .filter(|&&c| natural[c] < NARROW_THRESHOLD)
                .map(|&c| natural[c].max(NARROW_FLOOR))
                .sum();

            let content_natural_total: f32 = auto_cols
                .iter()
                .filter(|&&c| natural[c] >= NARROW_THRESHOLD)
                .map(|&c| natural[c])
                .sum();

            if content_natural_total > 0.0 {
                let content_remaining = (remaining - narrow_total).max(0.0);
                for &col in &auto_cols {
                    if natural[col] < NARROW_THRESHOLD {
                        explicit[col] = Some(natural[col].max(NARROW_FLOOR));
                    } else {
                        explicit[col] = Some(content_remaining * natural[col] / content_natural_total);
                    }
                }
            } else {
                // All auto columns are narrow - distribute remaining proportionally.
                for &col in &auto_cols {
                    explicit[col] = Some(remaining * natural[col] / total_natural);
                }
            }
        } else {
            // No content width data (mock trees) - fall back to equal distribution.
            let equal = remaining / auto_cols.len() as f32;
            for &col in &auto_cols {
                explicit[col] = Some(equal);
            }
        }
    }

    explicit.iter().map(|w| w.unwrap_or(0.0)).collect()
}
