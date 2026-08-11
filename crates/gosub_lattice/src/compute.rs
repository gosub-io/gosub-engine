use crate::geo::{Point, Size};
use anyhow::Result;
use std::collections::HashMap;

use crate::grid::{build_section_grid, PlacedCell, SectionGrid};
use crate::model::{build_model, RowGroup};
use crate::sizing::columns::{column_specs, compute_column_widths};
use crate::sizing::rows::{compute_row_heights, effective_border, read_border, read_padding};
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

    // Colspans may reach into columns other sections define, but never past
    // the table's last column (n_cols excludes phantom trailing columns).
    let mut header_grids = header_grids;
    let mut body_grids = body_grids;
    let mut footer_grids = footer_grids;
    for grid in header_grids
        .iter_mut()
        .chain(body_grids.iter_mut())
        .chain(footer_grids.iter_mut())
    {
        grid.clamp_colspans(n_cols);
    }

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

    // Border-conflict resolution (collapse only) runs BEFORE any sizing:
    // suppressed edges take no layout space, so intrinsic measurement, row
    // heights and cell boxes must all use the effective (post-conflict)
    // borders. The implementor is notified so its layout engine agrees.
    let suppressed_borders: HashMap<T::NodeId, [bool; 4]> = if collapse {
        let resolved = resolve_border_conflicts(tree, n_cols, &all_grids);
        for (&node, &edges) in &resolved {
            tree.suppress_cell_borders(node, edges);
        }
        resolved
    } else {
        HashMap::new()
    };

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

    if std::env::var_os("LATTICE_DEBUG").is_some() {
        eprintln!(
            "lattice: table {:?} n_cols={} table_width={} col_widths={:?}",
            table_node, n_cols, table_width, col_widths
        );
    }

    // Precompute cumulative column x-offsets (relative to the row's left edge).
    // col_x[i] = x of the left edge of column i (within a row). Under collapse
    // the cells sit flush (spacing 0) and conflict resolution below decides
    // which cell paints each shared border.
    let col_x = col_x_offsets(&col_widths, spacing_x);

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
            &suppressed_borders,
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
            &suppressed_borders,
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
            &suppressed_borders,
        ));
    }

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
    let section_data: &[(&[RowGroup<T::NodeId>], &[SectionGrid<T::NodeId>], &[Vec<f32>])] = &[
        (&model.header_groups, &header_grids, &header_heights),
        (&model.row_groups, &body_grids, &body_heights),
        (&model.footer_groups, &footer_grids, &footer_heights),
    ];

    for (groups, grids, heights) in section_data {
        for ((group, grid), row_heights) in groups.iter().zip(grids.iter()).zip(heights.iter()) {
            let row_y = row_y_offsets(row_heights, spacing_y);
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
                        suppressed_borders: [false; 4],
                    },
                );
            }

            place_rows(
                tree,
                group,
                grid,
                row_heights,
                &row_y,
                &col_x,
                &col_widths,
                &content_heights,
                &suppressed_borders,
            );

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
                suppressed_borders: [false; 4],
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
    suppressed_borders: &HashMap<T::NodeId, [bool; 4]>,
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
                    suppressed_borders: [false; 4],
                },
            );
        }

        // Cells for this row.
        for cell in grid.cells_in_row(row_idx) {
            place_cell(
                tree,
                cell,
                row_heights,
                col_x,
                col_widths,
                row_y,
                content_heights,
                suppressed_borders,
            );
        }
    }
}

/// Write the layout for one placed cell.
#[allow(clippy::too_many_arguments)]
fn place_cell<T: TableTree>(
    tree: &mut T,
    cell: &PlacedCell<T::NodeId>,
    row_heights: &[f32],
    col_x: &[f32],
    col_widths: &[f32],
    row_y: &[f32],
    content_heights: &HashMap<T::NodeId, f32>,
    suppressed_borders: &HashMap<T::NodeId, [bool; 4]>,
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

    let border = effective_border(tree, cell.node, suppressed_borders);
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
            suppressed_borders: suppressed_borders.get(&cell.node).copied().unwrap_or([false; 4]),
        },
    );
}

// Offset helpers

/// `col_x[i]` = x of the left edge of column `i` within a row, in px.
/// Accounts for the border-spacing gutter to the left of each column
/// (zero under `border-collapse`, where cells sit flush).
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
/// The first row starts at 0 - the gutter above it belongs to the table
/// (or to the previous group's bottom boundary), not to this group.
fn row_y_offsets(row_heights: &[f32], spacing_y: f32) -> Vec<f32> {
    let mut offsets = Vec::with_capacity(row_heights.len());
    let mut y = 0.0;
    for &h in row_heights {
        offsets.push(y);
        y += h + spacing_y;
    }
    offsets
}

/// Total height of a section: from the top of the first row to the bottom of
/// the last (gutters are baked into `row_y`). Boundary gutters (above the
/// first row / below the last) are added by the caller when stacking groups,
/// so they are not counted here.
fn section_height(row_heights: &[f32], row_y: &[f32]) -> f32 {
    match (row_y.last(), row_heights.last()) {
        (Some(&y), Some(&h)) => y + h,
        _ => 0.0,
    }
}

/// CSS border-conflict resolution for `border-collapse` (simplified): every
/// boundary shared by two cells is painted by exactly one of them - the wider
/// border wins; ties go to the left/top cell (matching CSS 2 §17.6.2.1 for
/// same-style borders; the style-rank tiebreak is not implemented). A cell
/// edge is suppressed when it loses against EVERY neighbouring segment along
/// that edge; mixed outcomes keep the edge painted (overlap behaviour).
///
/// Grids must be in render order (header -> body -> footer) so cross-section
/// adjacency is resolved too. Edge index order: `[top, right, bottom, left]`.
fn resolve_border_conflicts<T: TableTree>(
    tree: &T,
    n_cols: usize,
    grids: &[&SectionGrid<T::NodeId>],
) -> HashMap<T::NodeId, [bool; 4]> {
    // Flat slot occupancy across all sections, in render order.
    let total_rows: usize = grids.iter().map(|g| g.n_rows).sum();
    let mut slots: Vec<Vec<Option<T::NodeId>>> = vec![vec![None; n_cols]; total_rows];
    let mut borders: HashMap<T::NodeId, crate::types::BoxEdges> = HashMap::new();

    let mut row_offset = 0;
    for grid in grids {
        for cell in grid.cells() {
            borders.entry(cell.node).or_insert_with(|| read_border(tree, cell.node));
            for r in cell.row..(cell.row + cell.rowspan).min(grid.n_rows) {
                for c in cell.col..(cell.col + cell.colspan).min(n_cols) {
                    slots[row_offset + r][c] = Some(cell.node);
                }
            }
        }
        row_offset += grid.n_rows;
    }

    // Per (cell, edge): whether it faced any neighbour and whether it lost
    // every segment. `wins[edge]` flips to true as soon as one segment wins.
    #[derive(Default, Clone, Copy)]
    struct EdgeState {
        has_seg: [bool; 4],
        won_any: [bool; 4],
    }
    let mut states: HashMap<T::NodeId, EdgeState> = HashMap::new();
    let zero = crate::types::BoxEdges::default();

    const TOP: usize = 0;
    const RIGHT: usize = 1;
    const BOTTOM: usize = 2;
    const LEFT: usize = 3;

    for r in 0..total_rows {
        for c in 0..n_cols {
            let Some(cur) = slots[r][c] else { continue };

            // Vertical boundary with the cell to the right.
            if c + 1 < n_cols {
                if let Some(next) = slots[r][c + 1] {
                    if next != cur {
                        let left_w = borders.get(&cur).unwrap_or(&zero).right;
                        let right_w = borders.get(&next).unwrap_or(&zero).left;
                        let left_wins = left_w >= right_w;
                        let s = states.entry(cur).or_default();
                        s.has_seg[RIGHT] = true;
                        if left_wins {
                            s.won_any[RIGHT] = true;
                        }
                        let s = states.entry(next).or_default();
                        s.has_seg[LEFT] = true;
                        if !left_wins {
                            s.won_any[LEFT] = true;
                        }
                    }
                }
            }

            // Horizontal boundary with the cell below.
            if r + 1 < total_rows {
                if let Some(below) = slots[r + 1][c] {
                    if below != cur {
                        let top_w = borders.get(&cur).unwrap_or(&zero).bottom;
                        let bottom_w = borders.get(&below).unwrap_or(&zero).top;
                        let top_wins = top_w >= bottom_w;
                        let s = states.entry(cur).or_default();
                        s.has_seg[BOTTOM] = true;
                        if top_wins {
                            s.won_any[BOTTOM] = true;
                        }
                        let s = states.entry(below).or_default();
                        s.has_seg[TOP] = true;
                        if !top_wins {
                            s.won_any[TOP] = true;
                        }
                    }
                }
            }
        }
    }

    states
        .into_iter()
        .map(|(node, s)| {
            let mut suppressed = [false; 4];
            for e in 0..4 {
                suppressed[e] = s.has_seg[e] && !s.won_any[e];
            }
            (node, suppressed)
        })
        .collect()
}

// Zero-value BoxEdges constant (avoids Default derive noise in call sites).
const BOX_EDGES_ZERO: crate::types::BoxEdges = crate::types::BoxEdges {
    top: 0.0,
    right: 0.0,
    bottom: 0.0,
    left: 0.0,
};
