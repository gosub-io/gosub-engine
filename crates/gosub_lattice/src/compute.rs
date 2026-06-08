use anyhow::Result;
use gosub_shared::geo::{Point, Size};

use crate::grid::{build_section_grid, PlacedCell, SectionGrid};
use crate::model::{build_model, RowGroup};
use crate::sizing::rows::{compute_row_heights, read_border, read_padding};
use crate::sizing::{columns::compute_column_widths};
use crate::types::{CellLayout, CssProp, CssLength};
use crate::TableTree;

/// Entry point for the CSS table layout algorithm.
///
/// Reads the table structure from `tree` starting at `table_node`, computes
/// positions for every table-internal node (groups, rows, cells), and writes
/// them back via [`TableTree::set_layout`].
///
/// Returns `(content_width, content_height)` — the border-box size that the
/// table occupies.  The caller is responsible for writing the table node's own
/// layout (its position in the surrounding flow).
pub fn compute_table_layout<T: TableTree>(
    tree: &mut T,
    table_node: T::NodeId,
    available_width: f32,
    _available_height: Option<f32>,
) -> Result<(f32, f32)> {
    let model = build_model(tree, table_node);
    let (spacing_x, spacing_y) = model.border_spacing;

    // -----------------------------------------------------------------------
    // 1. Build per-section grids
    // -----------------------------------------------------------------------
    let header_grids: Vec<SectionGrid<T::NodeId>> = model
        .header_groups
        .iter()
        .map(|g| build_section_grid(&g.rows))
        .collect();

    let body_grids: Vec<SectionGrid<T::NodeId>> = model
        .row_groups
        .iter()
        .map(|g| build_section_grid(&g.rows))
        .collect();

    let footer_grids: Vec<SectionGrid<T::NodeId>> = model
        .footer_groups
        .iter()
        .map(|g| build_section_grid(&g.rows))
        .collect();

    // -----------------------------------------------------------------------
    // 2. Determine column count across all sections
    // -----------------------------------------------------------------------
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

    // -----------------------------------------------------------------------
    // 3. Resolve table width
    // -----------------------------------------------------------------------
    let table_width = match tree.css_length(model.node, CssProp::Width) {
        CssLength::Px(w) => w,
        CssLength::Percent(p) => p / 100.0 * available_width,
        _ => available_width,
    };

    // -----------------------------------------------------------------------
    // 4. Column widths
    // -----------------------------------------------------------------------
    let all_grids: Vec<&SectionGrid<T::NodeId>> = header_grids
        .iter()
        .chain(body_grids.iter())
        .chain(footer_grids.iter())
        .collect();

    let col_widths =
        compute_column_widths(tree, n_cols, table_width, spacing_x, &all_grids);

    // Precompute cumulative column x-offsets (relative to the row's left edge).
    // col_x[i] = x of the left edge of column i (within a row).
    let col_x = col_x_offsets(&col_widths, spacing_x);

    // -----------------------------------------------------------------------
    // 5. Row heights per section
    //
    // `compute_row_heights` takes `&mut tree` so it can call `layout_cell` to
    // run the normal layout engine inside each cell.  We use for-loops rather
    // than iterator combinators because a closure can't hold `&mut tree` while
    // the model is also borrowed.
    // -----------------------------------------------------------------------
    let mut header_heights: Vec<Vec<f32>> = Vec::with_capacity(header_grids.len());
    for grid in &header_grids {
        header_heights.push(compute_row_heights(tree, grid, &col_widths));
    }

    let mut body_heights: Vec<Vec<f32>> = Vec::with_capacity(body_grids.len());
    for grid in &body_grids {
        body_heights.push(compute_row_heights(tree, grid, &col_widths));
    }

    let mut footer_heights: Vec<Vec<f32>> = Vec::with_capacity(footer_grids.len());
    for grid in &footer_grids {
        footer_heights.push(compute_row_heights(tree, grid, &col_widths));
    }

    // -----------------------------------------------------------------------
    // 6. Apply positions
    //
    //    Per CSS: sections are rendered in the order header → body → footer,
    //    regardless of their source position.  Each group is positioned
    //    relative to the table.  Each row is positioned relative to its group.
    //    Each cell is positioned relative to its row.
    // -----------------------------------------------------------------------
    let inner_width = col_widths.iter().sum::<f32>()
        + (n_cols as f32 + 1.0) * spacing_x;

    let mut group_y = spacing_y; // Y offset of the next group, relative to the table

    let section_data: &[(&[RowGroup<T::NodeId>], &[SectionGrid<T::NodeId>], &[Vec<f32>])] = &[
        (&model.header_groups, &header_grids, &header_heights),
        (&model.row_groups, &body_grids, &body_heights),
        (&model.footer_groups, &footer_grids, &footer_heights),
    ];

    for (groups, grids, heights) in section_data {
        for ((group, grid), row_heights) in groups.iter().zip(grids.iter()).zip(heights.iter()) {
            let group_height = section_height(row_heights, spacing_y);

            if let Some(node) = group.node {
                tree.set_layout(
                    node,
                    CellLayout {
                        position: Point::new(0.0, group_y),
                        size: Size::new(inner_width, group_height),
                        border: BOX_EDGES_ZERO,
                        padding: BOX_EDGES_ZERO,
                    },
                );
            }

            place_rows(tree, group, grid, row_heights, &col_x, &col_widths, spacing_y);

            group_y += group_height;
        }
    }

    let total_height = group_y + spacing_y;

    Ok((table_width, total_height))
}

/// Write layouts for every row and cell within one section.
fn place_rows<T: TableTree>(
    tree: &mut T,
    group: &RowGroup<T::NodeId>,
    grid: &SectionGrid<T::NodeId>,
    row_heights: &[f32],
    col_x: &[f32],
    col_widths: &[f32],
    spacing_y: f32,
) {
    // Precompute y offset of each row within the group.
    let row_y = row_y_offsets(row_heights, spacing_y);
    let inner_width: f32 = col_widths.iter().sum::<f32>();

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
                },
            );
        }

        // Cells for this row.
        for cell in grid.cells_in_row(row_idx) {
            place_cell(tree, cell, row_heights, col_x, col_widths, &row_y);
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
) {
    // Width = sum of spanned column widths (col_widths already excludes spacing).
    let cell_width: f32 = col_widths
        .get(cell.col..cell.col + cell.colspan)
        .unwrap_or(&[])
        .iter()
        .sum();

    // Height = sum of spanned row heights.
    let cell_height: f32 = row_heights
        .get(cell.row..cell.row + cell.rowspan)
        .unwrap_or(&[])
        .iter()
        .sum();

    let x = col_x.get(cell.col).copied().unwrap_or(0.0);

    // Cell y is relative to its own row's top.
    let cell_row_y = row_y.get(cell.row).copied().unwrap_or(0.0);
    let start_row_y = row_y.get(cell.row).copied().unwrap_or(0.0);
    let y_within_row = cell_row_y - start_row_y; // always 0.0 for row-relative coords

    let border = read_border(tree, cell.node);
    let padding = read_padding(tree, cell.node);

    tree.set_layout(
        cell.node,
        CellLayout {
            position: Point::new(x, y_within_row),
            size: Size::new(cell_width, cell_height),
            border,
            padding,
        },
    );
}

// ---------------------------------------------------------------------------
// Offset helpers
// ---------------------------------------------------------------------------

/// `col_x[i]` = x of the left edge of column `i` within a row, in px.
/// Accounts for the border-spacing gutter to the left of each column.
fn col_x_offsets(col_widths: &[f32], spacing_x: f32) -> Vec<f32> {
    let mut offsets = Vec::with_capacity(col_widths.len());
    let mut x = spacing_x;
    for &w in col_widths {
        offsets.push(x);
        x += w + spacing_x;
    }
    offsets
}

/// `row_y[i]` = y of the top edge of row `i` within its group, in px.
fn row_y_offsets(row_heights: &[f32], spacing_y: f32) -> Vec<f32> {
    let mut offsets = Vec::with_capacity(row_heights.len());
    let mut y = spacing_y;
    for &h in row_heights {
        offsets.push(y);
        y += h + spacing_y;
    }
    offsets
}

/// Total height of a section including surrounding border-spacing gutters.
fn section_height(row_heights: &[f32], spacing_y: f32) -> f32 {
    let rows_h: f32 = row_heights.iter().sum();
    let gaps = (row_heights.len() as f32 + 1.0) * spacing_y;
    rows_h + gaps
}

// Zero-value BoxEdges constant (avoids Default derive noise in call sites).
const BOX_EDGES_ZERO: crate::types::BoxEdges = crate::types::BoxEdges {
    top: 0.0,
    right: 0.0,
    bottom: 0.0,
    left: 0.0,
};
