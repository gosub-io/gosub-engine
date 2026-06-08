use crate::model::TableRow;

/// A cell after it has been placed at a concrete `(row, col)` position in the grid.
#[derive(Debug, Clone)]
pub struct PlacedCell<N> {
    pub node: N,
    /// Zero-based row index within the **section** (not the whole table).
    pub row: usize,
    /// Zero-based column index within the table.
    pub col: usize,
    /// Effective colspan (>= 1).
    pub colspan: usize,
    /// Effective rowspan, already clamped to the section boundary (>= 1).
    pub rowspan: usize,
}

/// The resolved grid for a single table section (header, body, or footer).
pub struct SectionGrid<N> {
    cells: Vec<PlacedCell<N>>,
    pub n_cols: usize,
    pub n_rows: usize,
}

impl<N: Copy> SectionGrid<N> {
    /// All placed cells in the section.
    pub fn cells(&self) -> &[PlacedCell<N>] {
        &self.cells
    }

    /// Iterate over cells whose `row` field equals `row_idx`.
    pub fn cells_in_row(&self, row_idx: usize) -> impl Iterator<Item = &PlacedCell<N>> {
        self.cells.iter().filter(move |c| c.row == row_idx)
    }

    /// For each column, returns `true` if a spanning cell crosses the horizontal
    /// boundary between `row_idx` and `row_idx + 1`.
    ///
    /// Always returns all-`false` when `row_idx` is the last row of the section
    /// because rowspan is clamped — nothing can cross a section boundary.
    pub fn cols_spanned_at_boundary(&self, row_idx: usize) -> Vec<bool> {
        let mut spanned = vec![false; self.n_cols];
        if row_idx + 1 >= self.n_rows {
            return spanned;
        }
        for cell in &self.cells {
            // Spans through if the cell occupies both row_idx and row_idx+1.
            if cell.row <= row_idx && cell.row + cell.rowspan > row_idx + 1 {
                for c in cell.col..cell.col + cell.colspan {
                    if c < spanned.len() {
                        spanned[c] = true;
                    }
                }
            }
        }
        spanned
    }
}

/// Run the CSS table-slot filling algorithm (HTML spec §4.9.11) on one section.
///
/// Rules:
/// - Cells are placed left-to-right, skipping slots already occupied by a
///   spanning cell from an earlier row in the **same section**.
/// - `rowspan` is clamped to the number of rows remaining in the section,
///   enforcing that spans never cross section boundaries.
pub fn build_section_grid<N: Copy>(rows: &[TableRow<N>]) -> SectionGrid<N> {
    let n_rows = rows.len();
    let mut cells: Vec<PlacedCell<N>> = Vec::new();

    // slot_remaining[col] = rows (including the current one) for which that
    // column is still occupied by a spanning cell placed in an earlier row.
    // Value of 0 means the slot is free.
    let mut slot_remaining: Vec<usize> = Vec::new();

    for (row_idx, row) in rows.iter().enumerate() {
        // Each new row consumes one unit of every active span.
        decrement_slots(&mut slot_remaining);

        let mut col = 0;

        for source_cell in &row.cells {
            // Advance past any slots still occupied by a spanning cell.
            col = next_free_col(&slot_remaining, col);

            let colspan = source_cell.colspan.max(1);

            // Rowspan is clamped: a cell can only span within the current section.
            let rows_left = n_rows.saturating_sub(row_idx);
            let rowspan = source_cell.rowspan.max(1).min(rows_left);

            // Grow the slot tracker to cover all columns this cell occupies.
            grow_slots(&mut slot_remaining, col + colspan);

            // Mark each slot the cell occupies.
            for c in col..col + colspan {
                slot_remaining[c] = rowspan;
            }

            cells.push(PlacedCell {
                node: source_cell.node,
                row: row_idx,
                col,
                colspan,
                rowspan,
            });

            col += colspan;
        }
    }

    let n_cols = slot_remaining.len();
    SectionGrid { cells, n_cols, n_rows }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn decrement_slots(slots: &mut [usize]) {
    for s in slots.iter_mut() {
        *s = s.saturating_sub(1);
    }
}

fn next_free_col(slots: &[usize], start: usize) -> usize {
    let mut col = start;
    while col < slots.len() && slots[col] > 0 {
        col += 1;
    }
    col
}

fn grow_slots(slots: &mut Vec<usize>, min_len: usize) {
    while slots.len() < min_len {
        slots.push(0);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::SourceCell;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn make_row(cells: &[(usize, usize)]) -> TableRow<u32> {
        static NEXT_ID: AtomicU32 = AtomicU32::new(1);
        TableRow {
            node: None,
            cells: cells
                .iter()
                .map(|&(colspan, rowspan)| {
                    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
                    SourceCell { node: id, colspan, rowspan }
                })
                .collect(),
        }
    }

    #[test]
    fn simple_2x2_no_spans() {
        let rows = vec![make_row(&[(1, 1), (1, 1)]), make_row(&[(1, 1), (1, 1)])];
        let grid = build_section_grid(&rows);
        assert_eq!(grid.n_cols, 2);
        assert_eq!(grid.n_rows, 2);
        let placed: Vec<_> = grid.cells().iter().map(|c| (c.row, c.col)).collect();
        assert_eq!(placed, [(0, 0), (0, 1), (1, 0), (1, 1)]);
    }

    #[test]
    fn rowspan_within_section() {
        // Row 0: A (colspan=1, rowspan=2), B (colspan=1, rowspan=1)
        // Row 1: (col 0 occupied by A)  C (colspan=1, rowspan=1)
        let rows = vec![make_row(&[(1, 2), (1, 1)]), make_row(&[(1, 1)])];
        let grid = build_section_grid(&rows);
        assert_eq!(grid.n_cols, 2);
        // A at (0,0), B at (0,1), C at (1,1) — col 1 because col 0 is spanned
        let row1_cells: Vec<_> = grid.cells_in_row(1).collect();
        assert_eq!(row1_cells.len(), 1);
        assert_eq!(row1_cells[0].col, 1);
    }

    #[test]
    fn rowspan_clamped_at_section_boundary() {
        // Section has 2 rows; a cell in row 0 claims rowspan=5 → clamped to 2.
        let rows = vec![make_row(&[(1, 5), (1, 1)]), make_row(&[(1, 1)])];
        let grid = build_section_grid(&rows);
        let spanning = grid.cells_in_row(0).next().expect("first cell");
        assert_eq!(spanning.rowspan, 2, "rowspan must be clamped to section size");
    }

    #[test]
    fn colspan_places_correctly() {
        // Row 0: A (colspan=2), B (colspan=1)  → 3 columns
        let rows = vec![make_row(&[(2, 1), (1, 1)])];
        let grid = build_section_grid(&rows);
        assert_eq!(grid.n_cols, 3);
        let cells: Vec<_> = grid.cells().iter().collect();
        assert_eq!(cells[0].col, 0);
        assert_eq!(cells[0].colspan, 2);
        assert_eq!(cells[1].col, 2);
    }
}
