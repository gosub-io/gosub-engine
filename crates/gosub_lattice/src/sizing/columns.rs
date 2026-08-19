use crate::grid::SectionGrid;
use crate::model::TableModel;
use crate::sizing::rows::{effective_border, read_padding};
use crate::types::{CollapsedBorders, CssLength, CssProp, TableSizing};
use crate::TableTree;
use std::collections::HashMap;

/// Per-column width specs from `<colgroup>`/`<col>` elements, in document
/// order, expanded by their `span` attributes. A `<colgroup>` without `<col>`
/// children contributes its own width once per column it spans. Columns not
/// covered by any col element are simply absent from the returned vec.
pub fn column_specs<T: TableTree>(tree: &T, model: &TableModel<T::NodeId>) -> Vec<CssLength> {
    let mut specs = Vec::new();
    for group in &model.column_groups {
        if group.columns.is_empty() {
            let span = tree.attr_usize(group.node, "span").unwrap_or(1).max(1);
            let w = tree.css_length(group.node, CssProp::Width);
            specs.extend(std::iter::repeat(w).take(span));
        } else {
            for &col in &group.columns {
                let span = tree.attr_usize(col, "span").unwrap_or(1).max(1);
                let w = tree.css_length(col, CssProp::Width);
                specs.extend(std::iter::repeat(w).take(span));
            }
        }
    }
    specs
}

/// Compute column widths and the used table width.
///
/// `table-layout: fixed` (CSS 2 §17.5.2.1): widths come from `<col>` elements
/// first, then from the cells of the first row (a colspan cell's width divides
/// evenly over its columns); content is never measured. Remaining space is
/// split equally over the width-less columns.
///
/// `table-layout: auto` (CSS 2 §17.5.2.2): every cell contributes its
/// min-content and max-content width (via [`TableTree::cell_intrinsic_widths`])
/// and any specified width to its column(s); colspan cells distribute their
/// requirement over the spanned columns. The used table width is the explicit
/// width (floored at the min-content total) or, when auto, shrink-to-fit
/// between the min- and max-content totals capped by `available_width`.
/// Columns then grow from their min toward their max, with any extra space
/// distributed over the auto columns.
///
/// Returns `(column_widths, used_table_width)`.
#[allow(clippy::too_many_arguments)]
pub fn compute_column_widths<T: TableTree>(
    tree: &mut T,
    n_cols: usize,
    explicit_table_width: Option<f32>,
    available_width: f32,
    border_spacing_x: f32,
    grids: &[&SectionGrid<T::NodeId>],
    sizing: TableSizing,
    col_specs: &[CssLength],
    collapsed_borders: &HashMap<T::NodeId, CollapsedBorders>,
    // CAPMIN: the caption's minimum border-box width. CSS 2 §17.5.2 makes the used
    // table width the greater of the width the columns require and CAPMIN, in both
    // layout modes. 0.0 when there is no caption.
    capmin: f32,
) -> (Vec<f32>, f32) {
    if n_cols == 0 {
        return (Vec::new(), capmin.max(0.0));
    }

    // Total space consumed by border-spacing gutters.
    let spacing_total = (n_cols as f32 + 1.0) * border_spacing_x;

    if sizing == TableSizing::Fixed {
        let table_width = explicit_table_width.unwrap_or(available_width).max(capmin);
        let available = (table_width - spacing_total).max(0.0);
        let widths = fixed_column_widths(tree, n_cols, available, grids, col_specs, collapsed_borders);
        return (widths, table_width);
    }

    // Percentages resolve against the explicit table width when there is one,
    // otherwise against the containing block.
    let percent_basis = explicit_table_width.unwrap_or(available_width);

    let mut min = vec![0.0_f32; n_cols];
    let mut max = vec![0.0_f32; n_cols];
    let mut spec: Vec<Option<f32>> = vec![None; n_cols];

    for (i, s) in col_specs.iter().take(n_cols).enumerate() {
        if let Some(px) = s.resolve(percent_basis) {
            spec[i] = Some(px);
        }
    }

    // Single-column cells contribute directly; colspan cells are collected and
    // distributed afterwards, shortest spans first.
    let mut spanners: Vec<(usize, usize, f32, f32, Option<f32>)> = Vec::new();
    for grid in grids {
        for cell in grid.cells() {
            let (min_c, max_c) = tree.cell_intrinsic_widths(cell.node);
            let max_c = max_c.max(min_c);
            let w = tree.css_length(cell.node, CssProp::Width).resolve(percent_basis);
            if cell.colspan == 1 {
                let c = cell.col;
                min[c] = min[c].max(min_c);
                max[c] = max[c].max(max_c);
                if let Some(w) = w {
                    spec[c] = Some(spec[c].map_or(w, |prev| prev.max(w)));
                }
            } else {
                spanners.push((cell.col, cell.colspan, min_c, max_c, w));
            }
        }
    }

    spanners.sort_by_key(|s| s.1);
    for (col, colspan, min_c, max_c, w) in spanners {
        let range = col..(col + colspan).min(n_cols);
        if range.is_empty() {
            continue;
        }
        // The spanning cell runs across the gutters between its columns.
        let gutters = border_spacing_x * range.len().saturating_sub(1) as f32;
        distribute_deficit(&mut min, range.clone(), min_c - gutters, &max);
        distribute_deficit(&mut max, range.clone(), max_c - gutters, &min);
        // A specified width on a spanning cell divides evenly over columns
        // that have no specified width of their own.
        if let Some(w) = w {
            let open: Vec<usize> = range.clone().filter(|&c| spec[c].is_none()).collect();
            if open.len() == range.len() {
                let share = (w - gutters) / open.len() as f32;
                for c in open {
                    spec[c] = Some(share.max(0.0));
                }
            }
        }
    }

    // A specified width cannot shrink a column below its min-content width; a
    // specified column contributes that fixed width as both its min and max.
    let contrib_min: Vec<f32> = (0..n_cols).map(|c| spec[c].map_or(min[c], |s| s.max(min[c]))).collect();
    let contrib_max: Vec<f32> = (0..n_cols)
        .map(|c| spec[c].map_or(max[c].max(min[c]), |s| s.max(min[c])))
        .collect();
    let cmin: f32 = contrib_min.iter().sum();
    let cmax: f32 = contrib_max.iter().sum();

    // With no intrinsic information at all shrink-to-fit would collapse the table -
    // fill the available width instead. Only for mock trees using the intrinsics
    // stub: a measuring implementor's all-zero result means genuinely empty cells,
    // which DO shrink to fit (an empty auto table is CAPMIN/spacing wide, CSS 2
    // §17.5.2.2).
    let has_intrinsic =
        tree.measures_intrinsics() || max.iter().any(|&m| m > 0.0) || min.iter().any(|&m| m > 0.0);

    let used_width = match explicit_table_width {
        Some(w) => w.max(cmin + spacing_total),
        None if !has_intrinsic => available_width,
        // Shrink-to-fit: as wide as the content wants, capped by the containing
        // block, but never below the min-content total.
        None => (cmax + spacing_total).min(available_width).max(cmin + spacing_total),
    }
    .max(capmin);

    // Distribute the inner width: start every column at its min contribution,
    // grow toward the max contributions, then hand any extra to auto columns.
    let inner = (used_width - spacing_total).max(0.0);
    let mut widths = contrib_min.clone();
    let mut extra = inner - cmin;
    if extra > 0.0 {
        let growth: Vec<f32> = (0..n_cols).map(|c| contrib_max[c] - contrib_min[c]).collect();
        let growth_total: f32 = growth.iter().sum();
        if growth_total > 0.0 {
            let g = extra.min(growth_total);
            for c in 0..n_cols {
                widths[c] += g * growth[c] / growth_total;
            }
            extra -= g;
        }
        if extra > 0.0 {
            // Space beyond every column's max: prefer auto columns, weighted by
            // their max-content width so content-heavy columns absorb more.
            let autos: Vec<usize> = (0..n_cols).filter(|&c| spec[c].is_none()).collect();
            let targets = if autos.is_empty() { (0..n_cols).collect() } else { autos };
            let weight_total: f32 = targets.iter().map(|&c| contrib_max[c]).sum();
            if weight_total > 0.0 {
                for &c in &targets {
                    widths[c] += extra * contrib_max[c] / weight_total;
                }
            } else {
                let equal = extra / targets.len() as f32;
                for &c in &targets {
                    widths[c] += equal;
                }
            }
        }
    }

    (widths, used_width)
}

/// Raise the values in `range` so they sum to at least `required`. The deficit
/// is split proportionally to `weights` (content-heavy columns absorb more),
/// equally when the weights are all zero.
fn distribute_deficit(vals: &mut [f32], range: std::ops::Range<usize>, required: f32, weights: &[f32]) {
    let current: f32 = vals[range.clone()].iter().sum();
    if required <= current {
        return;
    }
    let deficit = required - current;
    let weight_total: f32 = weights[range.clone()].iter().sum();
    if weight_total > 0.0 {
        for c in range {
            vals[c] += deficit * weights[c] / weight_total;
        }
    } else {
        let equal = deficit / range.len() as f32;
        for c in range {
            vals[c] += equal;
        }
    }
}

/// The fixed table layout algorithm: column widths are fully determined by
/// `<col>` elements and the first row's cells - later rows and content play no
/// part, which is what makes fixed layout single-pass and overflow-prone.
#[allow(clippy::too_many_arguments)]
fn fixed_column_widths<T: TableTree>(
    tree: &T,
    n_cols: usize,
    available: f32,
    grids: &[&SectionGrid<T::NodeId>],
    col_specs: &[CssLength],
    collapsed_borders: &HashMap<T::NodeId, CollapsedBorders>,
) -> Vec<f32> {
    let mut explicit: Vec<Option<f32>> = vec![None; n_cols];

    // Percentages resolve against the space actually available to columns:
    // the table width minus the border-spacing gutters (CSS 2 §17.5.2.1
    // "minus borders or cell spacing").
    for (i, spec) in col_specs.iter().take(n_cols).enumerate() {
        if let Some(px) = spec.resolve(available) {
            explicit[i] = Some(px);
        }
    }

    // First non-empty row: cell widths claim any columns the col elements left
    // open. A colspan cell's width divides evenly over its columns.
    'outer: for grid in grids {
        for row_idx in 0..grid.n_rows {
            let mut found_any = false;
            for cell in grid.cells_in_row(row_idx) {
                found_any = true;
                let Some(w) = tree.css_length(cell.node, CssProp::Width).resolve(available) else {
                    continue;
                };
                // The width property is the CONTENT width; the column gets the
                // cell's border-box (CSS 2 §17.5.2.1 as clarified on www-style:
                // padding and borders - halved under collapse - are added).
                let border = effective_border(tree, cell.node, collapsed_borders);
                let padding = read_padding(tree, cell.node);
                let outer = w + padding.left + padding.right + border.left + border.right;
                let share = outer / cell.colspan as f32;
                for c in cell.col..(cell.col + cell.colspan).min(n_cols) {
                    if explicit[c].is_none() {
                        explicit[c] = Some(share);
                    }
                }
            }
            if found_any {
                break 'outer;
            }
        }
    }

    let set_total: f32 = explicit.iter().flatten().sum();
    let auto_cols: Vec<usize> = (0..n_cols).filter(|&c| explicit[c].is_none()).collect();

    if !auto_cols.is_empty() {
        // Remaining space divides EQUALLY over the auto columns (fixed layout
        // has no content information to weight by).
        let equal = (available - set_total).max(0.0) / auto_cols.len() as f32;
        for c in auto_cols {
            explicit[c] = Some(equal);
        }
    } else if set_total > 0.0 && set_total < available {
        // Every column has a width but they under-fill the table: scale up
        // pro-rata so the columns span the full table width.
        let scale = available / set_total;
        for w in explicit.iter_mut().flatten() {
            *w *= scale;
        }
    }
    // set_total > available: keep the specified widths; the table overflows.

    explicit.iter().map(|w| w.unwrap_or(0.0)).collect()
}
