use crate::grid::SectionGrid;
use crate::types::{CssLength, CssProp};
use crate::TableTree;
use std::collections::HashMap;

/// Compute column widths for a table with `n_cols` columns.
///
/// Algorithm:
/// 1. The available space is `table_width` minus the horizontal border-spacing
///    gutters (one between each pair of columns plus the outer two).
/// 2. Scan every row of every provided grid (header first, then body, then
///    footer).  For each single-column cell:
///    - If it has an explicit CSS `width` in px or % and its column has no
///      width yet, assign that to the column.
///    - Keep the widest pre-pass natural width (from `cell_content_width`)
///      seen in the column, for use in step 3.
/// 3. Remaining space is distributed to auto columns proportionally to their
///    natural content width. Falls back to equal distribution if no content
///    width information is available.
pub fn compute_column_widths<T: TableTree>(
    tree: &mut T,
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
    let mut natural: Vec<f32> = vec![0.0; n_cols];
    let mut min_content: Vec<f32> = vec![0.0; n_cols];

    // Scan every row for explicit widths and natural content widths.
    //
    // A column's natural width is the widest of its cells, so all of them have to be looked at.
    // Reading a single row instead cannot describe a table whose columns are not all introduced at
    // once: Wikipedia's dialect table heads columns 0-6 in its first row and puts columns 7 and 8
    // in a *second* header row, under a `colspan=2` title. The first row says nothing about those
    // two, so they measured 0 and collapsed to the narrow floor - 14 px each - while their content
    // spilled out to the right. Only cells spanning one column are counted: a spanning cell covers
    // several at once and cannot tell them apart.
    for grid in grids {
        for row_idx in 0..grid.n_rows {
            for cell in grid.cells_in_row(row_idx) {
                if cell.colspan != 1 || cell.col >= n_cols {
                    continue;
                }
                let cw = tree.cell_content_width(cell.node);
                let mcw = tree.cell_min_content_width(cell.node);
                if mcw > min_content[cell.col] {
                    min_content[cell.col] = mcw;
                }
                if explicit[cell.col].is_none() {
                    // A specified width cannot shrink a cell below its content's min-width
                    // (CSS: used width = max(specified, min-content)). Without this, e.g. a
                    // `width:18px` cell holding a 20px image clips it and eats the padding.
                    match tree.css_length(cell.node, CssProp::Width) {
                        CssLength::Px(px) => explicit[cell.col] = Some(px.max(cw)),
                        CssLength::Percent(p) => explicit[cell.col] = Some((p / 100.0 * table_width).max(cw)),
                        _ => {}
                    }
                }
                if cw > natural[cell.col] {
                    natural[cell.col] = cw;
                }
            }
        }
    }

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
                let content_cols: Vec<usize> = auto_cols
                    .iter()
                    .copied()
                    .filter(|&c| natural[c] >= NARROW_THRESHOLD)
                    .collect();
                let shares = distribute_with_floor(
                    content_remaining,
                    &content_cols,
                    &natural,
                    &min_content,
                    content_natural_total,
                );
                for &col in &auto_cols {
                    if natural[col] < NARROW_THRESHOLD {
                        explicit[col] = Some(natural[col].max(NARROW_FLOOR).max(min_content[col]));
                    } else {
                        explicit[col] = Some(shares[&col]);
                    }
                }
            } else {
                // All auto columns are narrow - distribute remaining proportionally.
                for &col in &auto_cols {
                    explicit[col] = Some((remaining * natural[col] / total_natural).max(min_content[col]));
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

/// Share `space` among `cols` proportionally to their natural width, but never below a column's
/// min-content width.
///
/// A plain proportional split can hand a column less than its content can ever occupy, and the
/// content then spills out of the cell - Wikipedia's dialect table gave the "Windows" column 73px
/// for a word that is 87px wide including its padding. This follows the shape of CSS 2.1
/// §17.5.2.2: every column takes its min-content first, and what is left over is shared out in
/// proportion to how much *more* than that each column wants. When even the min-contents do not
/// fit, each column takes its min-content and the table overflows, which is what browsers do.
fn distribute_with_floor(
    space: f32,
    cols: &[usize],
    natural: &[f32],
    min_content: &[f32],
    natural_total: f32,
) -> HashMap<usize, f32> {
    // A column can want less than its min-content (a long word inside a cell whose other content
    // is narrower), so the ceiling is the larger of the two.
    let want = |c: usize| natural[c].max(min_content[c]);
    let floor_total: f32 = cols.iter().map(|&c| min_content[c]).sum();

    if floor_total <= 0.0 {
        // Nothing to floor against - the plain proportional split, unchanged.
        return cols.iter().map(|&c| (c, space * natural[c] / natural_total)).collect();
    }
    if space <= floor_total {
        return cols.iter().map(|&c| (c, min_content[c])).collect();
    }

    let want_total: f32 = cols.iter().map(|&c| want(c)).sum();
    let slack_total = want_total - floor_total;
    let surplus = space - floor_total;
    if slack_total <= 0.0 {
        // Every column is at its floor; share what is left equally rather than by a zero ratio.
        let each = surplus / cols.len() as f32;
        return cols.iter().map(|&c| (c, min_content[c] + each)).collect();
    }
    // Every column reaches its floor first, then takes a share of what is left in proportion to
    // how much *more* than that it asked for.
    let ratio = (surplus / slack_total).min(1.0);
    // Space beyond what the columns asked for altogether. It is shared in proportion to `want`,
    // which is what the unfloored split did with all of it.
    let extra = (surplus - slack_total).max(0.0);
    cols.iter()
        .map(|&c| {
            let share = min_content[c] + (want(c) - min_content[c]) * ratio + extra * want(c) / want_total;
            (c, share)
        })
        .collect()
}

/// The table's max-content width.
///
/// Every column at its cells' natural width, plus the gutters.
///
/// Used for a shrink-to-fit table, where the used width is `min(available, max-content)` rather
/// than all the space on offer. A cell's natural width comes from the layout engine's earlier
/// pass, so this is only as good as that measurement - which is exactly what the auto column
/// distribution below already relies on.
#[must_use]
pub fn max_content_width<T: TableTree>(
    tree: &T,
    n_cols: usize,
    border_spacing_x: f32,
    grids: &[&SectionGrid<T::NodeId>],
) -> f32 {
    if n_cols == 0 {
        return 0.0;
    }
    let width_of = |cell: &crate::grid::PlacedCell<T::NodeId>| match tree.css_length(cell.node, CssProp::Width) {
        CssLength::Px(px) => px.max(tree.cell_content_width(cell.node)),
        _ => tree.cell_content_width(cell.node),
    };

    let mut natural = vec![0.0_f32; n_cols];
    for grid in grids {
        for row_idx in 0..grid.n_rows {
            for cell in grid.cells_in_row(row_idx) {
                if cell.colspan != 1 || cell.col >= n_cols {
                    continue;
                }
                let width = width_of(cell);
                natural[cell.col] = natural[cell.col].max(width);
            }
        }
    }

    // A spanning cell has to fit too. Here the answer *is* the table's width - a floated table is
    // sized from it - so leaving spanning cells out is not the harmless approximation it is when
    // distributing a width that is already known: a table whose content lives entirely in spanning
    // cells measured nothing but its gutters and every column was then sized from near zero.
    //
    // Only the shortfall is added, spread evenly over the columns the cell covers, so a spanning
    // cell that already fits changes nothing.
    for grid in grids {
        for row_idx in 0..grid.n_rows {
            for cell in grid.cells_in_row(row_idx) {
                let last = cell.col + cell.colspan;
                if cell.colspan <= 1 || last > n_cols {
                    continue;
                }
                let covered = &mut natural[cell.col..last];
                let spanned_gutters = (cell.colspan as f32 - 1.0) * border_spacing_x;
                let width = width_of(cell);
                let shortfall = width - spanned_gutters - covered.iter().sum::<f32>();
                if shortfall <= 0.0 {
                    continue;
                }
                let share = shortfall / cell.colspan as f32;
                for col in covered {
                    *col += share;
                }
            }
        }
    }

    natural.iter().sum::<f32>() + (n_cols as f32 + 1.0) * border_spacing_x
}
