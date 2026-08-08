use crate::geo::{Point, Size};
use anyhow::Result;
use std::collections::HashMap;

use crate::grid::{build_section_grid, PlacedCell, SectionGrid};
use crate::model::{build_model, RowGroup};
use crate::sizing::columns::{column_specs, compute_column_widths};
use crate::sizing::rows::{compute_row_heights, read_border, read_padding};
use crate::types::{BorderCollapse, CellLayout, CssLength, CssProp, TableSizing};
use crate::TableTree;

/// Entry point for the CSS table layout algorithm.
///
/// Reads the table structure from `tree` starting at `table_node`, computes
/// positions for every table-internal node (groups, rows, cells), and writes
/// them back via [`TableTree::set_layout`].
///
/// Returns `(content_width, content_height)` - the border-box size that the
/// table occupies.  The caller is responsible for writing the table node's own
/// layout (its position in the surrounding flow).
pub fn compute_table_layout<T: TableTree>(
    tree: &mut T,
    table_node: T::NodeId,
    available_width: f32,
    _available_height: Option<f32>,
) -> Result<(f32, f32)> {
    let model = build_model(tree, table_node);
    let collapse = model.border_collapse == BorderCollapse::Collapse;
    // Collapsed borders have no gutters; adjacent cells share their border
    // instead (realized below by overlapping their boxes).
    let (spacing_x, spacing_y) = if collapse { (0.0, 0.0) } else { model.border_spacing };

    // Build per-section grids
    let header_grids: Vec<SectionGrid<T::NodeId>> = model
        .header_groups
        .iter()
        .map(|g| build_section_grid(&g.rows))
        .collect();

    let body_grids: Vec<SectionGrid<T::NodeId>> =
        model.row_groups.iter().map(|g| build_section_grid(&g.rows)).collect();

    let footer_grids: Vec<SectionGrid<T::NodeId>> = model
        .footer_groups
        .iter()
        .map(|g| build_section_grid(&g.rows))
        .collect();

    // Determine column count across all sections
    let n_cols = header_grids
        .iter()
        .chain(body_grids.iter())
        .chain(footer_grids.iter())
        .map(|g| g.n_cols)
        .max()
        .unwrap_or(0);

    if n_cols == 0 {
        return Ok((0.0, 0.0));
    }

    // Explicit widths from <colgroup>/<col> elements. Under fixed layout the
    // col elements can define columns beyond those implied by any cell.
    let col_specs = column_specs(tree, &model);
    let n_cols = if model.sizing == TableSizing::Fixed {
        n_cols.max(col_specs.len())
    } else {
        n_cols
    };

    // Resolve the explicit table width, if any; auto tables shrink-to-fit
    // inside `compute_column_widths`.
    let explicit_table_width = match tree.css_length(model.node, CssProp::Width) {
        CssLength::Px(w) => Some(w),
        CssLength::Percent(p) => Some(p / 100.0 * available_width),
        _ => None,
    };

    // Column widths
    let all_grids: Vec<&SectionGrid<T::NodeId>> = header_grids
        .iter()
        .chain(body_grids.iter())
        .chain(footer_grids.iter())
        .collect();

    let (col_widths, table_width) = compute_column_widths(
        tree,
        n_cols,
        explicit_table_width,
        available_width,
        spacing_x,
        &all_grids,
        model.sizing,
        &col_specs,
    );

    // Precompute cumulative column x-offsets (relative to the row's left edge).
    // col_x[i] = x of the left edge of column i (within a row). Under collapse,
    // adjacent columns overlap by their shared border width so the borders of
    // neighbouring cells coincide.
    let col_overlaps = if collapse {
        column_border_overlaps(tree, n_cols, &all_grids)
    } else {
        vec![0.0; n_cols]
    };
    let col_x = col_x_offsets(&col_widths, spacing_x, &col_overlaps);

    // Row heights per section
    //
    // `compute_row_heights` takes `&mut tree` so it can call `layout_cell` to
    // run the normal layout engine inside each cell.  We use for-loops rather
    // than iterator combinators because a closure can't hold `&mut tree` while
    // the model is also borrowed.
    let mut content_heights: HashMap<T::NodeId, f32> = HashMap::new();

    let mut header_heights: Vec<Vec<f32>> = Vec::with_capacity(header_grids.len());
    for grid in &header_grids {
        header_heights.push(compute_row_heights(
            tree,
            grid,
            &col_widths,
            spacing_x,
            spacing_y,
            &mut content_heights,
        ));
    }

    let mut body_heights: Vec<Vec<f32>> = Vec::with_capacity(body_grids.len());
    for grid in &body_grids {
        body_heights.push(compute_row_heights(
            tree,
            grid,
            &col_widths,
            spacing_x,
            spacing_y,
            &mut content_heights,
        ));
    }

    let mut footer_heights: Vec<Vec<f32>> = Vec::with_capacity(footer_grids.len());
    for grid in &footer_grids {
        footer_heights.push(compute_row_heights(
            tree,
            grid,
            &col_widths,
            spacing_x,
            spacing_y,
            &mut content_heights,
        ));
    }

    // Per-section vertical border overlaps (collapse only): rows within a
    // section overlap by their shared border; the section edge borders are
    // kept so adjacent sections can overlap too.
    let all_row_overlaps: Vec<Vec<RowOverlaps>> = [&header_grids, &body_grids, &footer_grids]
        .into_iter()
        .map(|grids| grids.iter().map(|g| row_border_overlaps(tree, g, collapse)).collect())
        .collect();

    // Caption: measured like a cell spanning the full table width; placed
    // above (default) or below the grid per `caption-side`.
    let caption_bottom = model
        .caption
        .map(|cap| matches!(tree.css_length(cap, CssProp::CaptionSide), CssLength::Px(v) if v == 1.0))
        .unwrap_or(false);
    let caption_height = match model.caption {
        Some(cap) => {
            let border = read_border(tree, cap);
            let padding = read_padding(tree, cap);
            let inner_w = (table_width - border.horizontal() - padding.horizontal()).max(0.0);
            let content_h = tree.layout_cell(cap, inner_w);
            let explicit_h = match tree.css_length(cap, CssProp::Height) {
                CssLength::Px(px) => px,
                _ => 0.0,
            };
            content_h.max(explicit_h) + border.vertical() + padding.vertical()
        }
        None => 0.0,
    };

    // Apply positions
    //
    //    Per CSS: sections are rendered in the order header → body → footer,
    //    regardless of their source position.  Each group is positioned
    //    relative to the table.  Each row is positioned relative to its group.
    //    Each cell is positioned relative to its row.
    let inner_width = match (col_x.last(), col_widths.last()) {
        (Some(&x), Some(&w)) => x + w + spacing_x,
        _ => 0.0,
    };

    // Y offset of the next group, relative to the table. Vertically the table is one
    // flat stack of rows: one gutter above the first row, one between any two adjacent
    // rows (also across group boundaries), one below the last. Groups therefore carry
    // only the (n_rows - 1) *internal* gutters; the shared boundary gutters live here.
    let mut group_y = if caption_bottom { 0.0 } else { caption_height } + spacing_y;

    #[allow(clippy::type_complexity)]
    let section_data: &[(&[RowGroup<T::NodeId>], &[SectionGrid<T::NodeId>], &[Vec<f32>], &[RowOverlaps])] = &[
        (&model.header_groups, &header_grids, &header_heights, &all_row_overlaps[0]),
        (&model.row_groups, &body_grids, &body_heights, &all_row_overlaps[1]),
        (&model.footer_groups, &footer_grids, &footer_heights, &all_row_overlaps[2]),
    ];

    // Bottom border of the previous non-empty section's last row, for
    // collapsing the boundary between adjacent sections.
    let mut prev_bottom: Option<f32> = None;

    for (groups, grids, heights, overlapses) in section_data {
        for (((group, grid), row_heights), overlaps) in
            groups.iter().zip(grids.iter()).zip(heights.iter()).zip(overlapses.iter())
        {
            if collapse && grid.n_rows > 0 {
                if let Some(prev) = prev_bottom {
                    group_y -= prev.min(overlaps.first_top);
                }
            }

            let row_y = row_y_offsets(row_heights, spacing_y, &overlaps.between);
            let group_height = section_height(row_heights, &row_y);

            if let Some(node) = group.node {
                tree.set_layout(
                    node,
                    CellLayout {
                        position: Point::new(0.0, group_y),
                        size: Size::new(inner_width, group_height),
                        border: BOX_EDGES_ZERO,
                        padding: BOX_EDGES_ZERO,
                        content_offset_y: 0.0,
                    },
                );
            }

            place_rows(tree, group, grid, row_heights, &row_y, &col_x, &col_widths, &content_heights);

            if grid.n_rows > 0 {
                prev_bottom = Some(overlaps.last_bottom);
            }
            group_y += group_height + spacing_y;
        }
    }

    // A top caption's height is already part of group_y; a bottom caption
    // extends the table below the grid.
    let mut total_height = group_y;
    if let Some(cap) = model.caption {
        let y = if caption_bottom { total_height } else { 0.0 };
        let border = read_border(tree, cap);
        let padding = read_padding(tree, cap);
        tree.set_layout(
            cap,
            CellLayout {
                position: Point::new(0.0, y),
                size: Size::new(table_width, caption_height),
                border,
                padding,
                content_offset_y: 0.0,
            },
        );
        if caption_bottom {
            total_height += caption_height;
        }
    }

    Ok((table_width, total_height))
}

/// Write layouts for every row and cell within one section.
#[allow(clippy::too_many_arguments)]
fn place_rows<T: TableTree>(
    tree: &mut T,
    group: &RowGroup<T::NodeId>,
    grid: &SectionGrid<T::NodeId>,
    row_heights: &[f32],
    row_y: &[f32],
    col_x: &[f32],
    col_widths: &[f32],
    content_heights: &HashMap<T::NodeId, f32>,
) {
    let inner_width: f32 = match (col_x.last(), col_widths.last()) {
        (Some(&x), Some(&w)) => x + w - col_x.first().copied().unwrap_or(0.0),
        _ => 0.0,
    };

    for (row_idx, row) in group.rows.iter().enumerate() {
        let ry = row_y[row_idx];
        let rh = row_heights[row_idx];

        if let Some(node) = row.node {
            tree.set_layout(
                node,
                CellLayout {
                    position: Point::new(0.0, ry),
                    size: Size::new(inner_width, rh),
                    border: BOX_EDGES_ZERO,
                    padding: BOX_EDGES_ZERO,
                    content_offset_y: 0.0,
                },
            );
        }

        // Cells for this row.
        for cell in grid.cells_in_row(row_idx) {
            place_cell(tree, cell, row_heights, col_x, col_widths, row_y, content_heights);
        }
    }
}

/// Write the layout for one placed cell.
fn place_cell<T: TableTree>(
    tree: &mut T,
    cell: &PlacedCell<T::NodeId>,
    row_heights: &[f32],
    col_x: &[f32],
    col_widths: &[f32],
    row_y: &[f32],
    content_heights: &HashMap<T::NodeId, f32>,
) {
    // Extents come from the offset tables so gutters (separate borders) and
    // overlaps (collapsed borders) are both handled: a spanning cell runs from
    // its first column/row's start to its last column/row's end.
    let last_col = (cell.col + cell.colspan - 1).min(col_widths.len().saturating_sub(1));
    let x = col_x.get(cell.col).copied().unwrap_or(0.0);
    let cell_width = col_x.get(last_col).copied().unwrap_or(0.0) + col_widths.get(last_col).copied().unwrap_or(0.0) - x;

    let last_row = (cell.row + cell.rowspan - 1).min(row_heights.len().saturating_sub(1));
    let cell_row_y = row_y.get(cell.row).copied().unwrap_or(0.0);
    let cell_height =
        row_y.get(last_row).copied().unwrap_or(0.0) + row_heights.get(last_row).copied().unwrap_or(0.0) - cell_row_y;

    // Cell y is relative to its own row's top.
    let y_within_row = 0.0;

    let border = read_border(tree, cell.node);
    let padding = read_padding(tree, cell.node);

    // vertical-align: shift the cell's children down within the free space
    // between its content and its (row-driven) height.
    let inner_h = (cell_height - border.vertical() - padding.vertical()).max(0.0);
    let content_h = content_heights.get(&cell.node).copied().unwrap_or(0.0);
    let free = (inner_h - content_h).max(0.0);
    let content_offset_y = match tree.vertical_align(cell.node) {
        crate::types::VerticalAlign::Top => 0.0,
        crate::types::VerticalAlign::Middle => free / 2.0,
        crate::types::VerticalAlign::Bottom => free,
    };

    tree.set_layout(
        cell.node,
        CellLayout {
            position: Point::new(x, y_within_row),
            size: Size::new(cell_width, cell_height),
            border,
            padding,
            content_offset_y,
        },
    );
}

// Offset helpers

/// Vertical border-overlap data for one section under `border-collapse`.
struct RowOverlaps {
    /// `between[r]` = overlap between row `r-1` and row `r` (index 0 unused).
    between: Vec<f32>,
    /// Max top border of the section's first row (for cross-section collapse).
    first_top: f32,
    /// Max bottom border of the section's last row.
    last_bottom: f32,
}

/// `col_x[i]` = x of the left edge of column `i` within a row, in px.
/// Accounts for the border-spacing gutter to the left of each column
/// (separate borders) or the shared-border overlap (collapsed borders).
fn col_x_offsets(col_widths: &[f32], spacing_x: f32, overlaps: &[f32]) -> Vec<f32> {
    let mut offsets = Vec::with_capacity(col_widths.len());
    let mut x = spacing_x;
    for (c, &w) in col_widths.iter().enumerate() {
        if c > 0 {
            x -= overlaps.get(c).copied().unwrap_or(0.0);
        }
        offsets.push(x);
        x += w + spacing_x;
    }
    offsets
}

/// `row_y[i]` = y of the top edge of row `i` within its group, in px.
/// The first row starts at 0 - the gutter above it belongs to the table
/// (or to the previous group's bottom boundary), not to this group.
fn row_y_offsets(row_heights: &[f32], spacing_y: f32, overlaps: &[f32]) -> Vec<f32> {
    let mut offsets = Vec::with_capacity(row_heights.len());
    let mut y = 0.0;
    for (r, &h) in row_heights.iter().enumerate() {
        if r > 0 {
            y -= overlaps.get(r).copied().unwrap_or(0.0);
        }
        offsets.push(y);
        y += h + spacing_y;
    }
    offsets
}

/// Total height of a section: from the top of the first row to the bottom of
/// the last (gutters and collapse overlaps are baked into `row_y`). Boundary
/// gutters (above the first row / below the last) are added by the caller
/// when stacking groups, so they are not counted here.
fn section_height(row_heights: &[f32], row_y: &[f32]) -> f32 {
    match (row_y.last(), row_heights.last()) {
        (Some(&y), Some(&h)) => y + h,
        _ => 0.0,
    }
}

/// Per-boundary horizontal overlap between adjacent columns under
/// `border-collapse`: the shared border width, i.e. the smaller of the widest
/// right border ending at the boundary and the widest left border starting
/// there. Overlapping the cell boxes by this amount makes equal borders
/// coincide exactly; unequal ones stack with the later-painted cell on top.
fn column_border_overlaps<T: TableTree>(tree: &T, n_cols: usize, grids: &[&SectionGrid<T::NodeId>]) -> Vec<f32> {
    let mut left = vec![0.0_f32; n_cols];
    let mut right = vec![0.0_f32; n_cols];
    for grid in grids {
        for cell in grid.cells() {
            let b = read_border(tree, cell.node);
            let first = cell.col;
            let last = cell.col + cell.colspan - 1;
            if first < n_cols {
                left[first] = left[first].max(b.left);
            }
            if last < n_cols {
                right[last] = right[last].max(b.right);
            }
        }
    }
    let mut overlaps = vec![0.0_f32; n_cols];
    for c in 1..n_cols {
        overlaps[c] = right[c - 1].min(left[c]);
    }
    overlaps
}

/// Row-boundary equivalent of [`column_border_overlaps`], for one section.
/// Returns zeroed data when `collapse` is off.
fn row_border_overlaps<T: TableTree>(tree: &T, grid: &SectionGrid<T::NodeId>, collapse: bool) -> RowOverlaps {
    let n_rows = grid.n_rows;
    if !collapse || n_rows == 0 {
        return RowOverlaps {
            between: vec![0.0; n_rows],
            first_top: 0.0,
            last_bottom: 0.0,
        };
    }
    let mut top = vec![0.0_f32; n_rows];
    let mut bottom = vec![0.0_f32; n_rows];
    for cell in grid.cells() {
        let b = read_border(tree, cell.node);
        top[cell.row] = top[cell.row].max(b.top);
        let last = (cell.row + cell.rowspan - 1).min(n_rows - 1);
        bottom[last] = bottom[last].max(b.bottom);
    }
    let mut between = vec![0.0_f32; n_rows];
    for r in 1..n_rows {
        between[r] = bottom[r - 1].min(top[r]);
    }
    RowOverlaps {
        between,
        first_top: top.first().copied().unwrap_or(0.0),
        last_bottom: bottom.last().copied().unwrap_or(0.0),
    }
}

// Zero-value BoxEdges constant (avoids Default derive noise in call sites).
const BOX_EDGES_ZERO: crate::types::BoxEdges = crate::types::BoxEdges {
    top: 0.0,
    right: 0.0,
    bottom: 0.0,
    left: 0.0,
};
