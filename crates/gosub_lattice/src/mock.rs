/// A simple in-memory table tree used for testing and console rendering.
///
/// Build a table with [`MockTable`], call [`MockTable::render`] to get an ASCII
/// diagram, or [`MockTable::into_tree`] to get the raw [`MockTree`] and run
/// `compute_table_layout` yourself.
use std::collections::HashMap;

use crate::compute::compute_table_layout;
use crate::grid::{build_section_grid, PlacedCell, SectionGrid};
use crate::model::{build_model, RowGroup};
use crate::types::{CellLayout, CssLength, CssProp, TableRole};
use crate::TableTree;

// MockCell — a single cell specification used by the builder

/// Cell specification used with [`MockTable`].
#[derive(Clone)]
pub struct MockCell {
    pub label: String,
    pub colspan: usize,
    pub rowspan: usize,
    /// Explicit pixel width, if any.
    pub width: Option<f32>,
    /// Explicit pixel height, if any.
    pub height: Option<f32>,
    /// Uniform border width on all sides.
    pub border: f32,
    /// Uniform padding on all sides.
    pub padding: f32,
}

impl MockCell {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            colspan: 1,
            rowspan: 1,
            width: None,
            height: None,
            border: 0.0,
            padding: 1.0,
        }
    }

    pub fn colspan(mut self, n: usize) -> Self {
        self.colspan = n;
        self
    }
    pub fn rowspan(mut self, n: usize) -> Self {
        self.rowspan = n;
        self
    }
    pub fn width(mut self, w: f32) -> Self {
        self.width = Some(w);
        self
    }
    pub fn height(mut self, h: f32) -> Self {
        self.height = Some(h);
        self
    }
    pub fn border(mut self, b: f32) -> Self {
        self.border = b;
        self
    }
    pub fn padding(mut self, p: f32) -> Self {
        self.padding = p;
        self
    }
}

/// Shorthand: `cell("label")`.
pub fn cell(label: impl Into<String>) -> MockCell {
    MockCell::new(label)
}

// MockTable — fluent builder

#[derive(Default)]
pub struct MockTable {
    available_width: f32,
    border_spacing_x: f32,
    border_spacing_y: f32,
    header_rows: Vec<Vec<MockCell>>,
    body_rows: Vec<Vec<MockCell>>,
    footer_rows: Vec<Vec<MockCell>>,
}

impl MockTable {
    pub fn new(available_width: f32) -> Self {
        Self {
            available_width,
            border_spacing_x: 1.0,
            border_spacing_y: 0.0,
            ..Default::default()
        }
    }

    pub fn spacing(mut self, x: f32, y: f32) -> Self {
        self.border_spacing_x = x;
        self.border_spacing_y = y;
        self
    }

    pub fn header_row(mut self, cells: Vec<MockCell>) -> Self {
        self.header_rows.push(cells);
        self
    }

    pub fn body_row(mut self, cells: Vec<MockCell>) -> Self {
        self.body_rows.push(cells);
        self
    }

    pub fn footer_row(mut self, cells: Vec<MockCell>) -> Self {
        self.footer_rows.push(cells);
        self
    }

    /// Build a [`MockTree`], run layout, then produce an ASCII table string.
    pub fn render(self) -> String {
        let available_width = self.available_width;
        let (mut tree, root) = self.into_tree();
        let _ = compute_table_layout(&mut tree, root, available_width, None);
        render_tree(&tree, root)
    }

    /// Convert into a raw [`MockTree`] (root NodeId is returned alongside).
    pub fn into_tree(self) -> (MockTree, u32) {
        let mut tree = MockTree::new(self.border_spacing_x, self.border_spacing_y);
        let root = tree.alloc(TableRole::Table, None, 1, 1, None, None, 0.0, 0.0);

        if !self.header_rows.is_empty() {
            let hg = tree.alloc(TableRole::HeaderGroup, None, 1, 1, None, None, 0.0, 0.0);
            tree.add_child(root, hg);
            for row_cells in self.header_rows {
                let row = tree.alloc(TableRole::Row, None, 1, 1, None, None, 0.0, 0.0);
                tree.add_child(hg, row);
                for mc in row_cells {
                    let cell_id = tree.alloc(
                        TableRole::Cell,
                        Some(mc.label),
                        mc.colspan,
                        mc.rowspan,
                        mc.width,
                        mc.height,
                        mc.border,
                        mc.padding,
                    );
                    tree.add_child(row, cell_id);
                }
            }
        }

        if !self.body_rows.is_empty() {
            let bg = tree.alloc(TableRole::RowGroup, None, 1, 1, None, None, 0.0, 0.0);
            tree.add_child(root, bg);
            for row_cells in self.body_rows {
                let row = tree.alloc(TableRole::Row, None, 1, 1, None, None, 0.0, 0.0);
                tree.add_child(bg, row);
                for mc in row_cells {
                    let cell_id = tree.alloc(
                        TableRole::Cell,
                        Some(mc.label),
                        mc.colspan,
                        mc.rowspan,
                        mc.width,
                        mc.height,
                        mc.border,
                        mc.padding,
                    );
                    tree.add_child(row, cell_id);
                }
            }
        }

        if !self.footer_rows.is_empty() {
            let fg = tree.alloc(TableRole::FooterGroup, None, 1, 1, None, None, 0.0, 0.0);
            tree.add_child(root, fg);
            for row_cells in self.footer_rows {
                let row = tree.alloc(TableRole::Row, None, 1, 1, None, None, 0.0, 0.0);
                tree.add_child(fg, row);
                for mc in row_cells {
                    let cell_id = tree.alloc(
                        TableRole::Cell,
                        Some(mc.label),
                        mc.colspan,
                        mc.rowspan,
                        mc.width,
                        mc.height,
                        mc.border,
                        mc.padding,
                    );
                    tree.add_child(row, cell_id);
                }
            }
        }

        (tree, root)
    }
}

// MockTree — the in-memory node tree

struct MockNode {
    role: TableRole,
    label: Option<String>,
    colspan: usize,
    rowspan: usize,
    children: Vec<u32>,
    layout: Option<CellLayout>,
    width: Option<f32>,
    height: Option<f32>,
    border: f32,
    padding: f32,
}

pub struct MockTree {
    nodes: HashMap<u32, MockNode>,
    next_id: u32,
    border_spacing_x: f32,
    border_spacing_y: f32,
}

impl MockTree {
    fn new(border_spacing_x: f32, border_spacing_y: f32) -> Self {
        Self {
            nodes: HashMap::new(),
            next_id: 0,
            border_spacing_x,
            border_spacing_y,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn alloc(
        &mut self,
        role: TableRole,
        label: Option<String>,
        colspan: usize,
        rowspan: usize,
        width: Option<f32>,
        height: Option<f32>,
        border: f32,
        padding: f32,
    ) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        self.nodes.insert(
            id,
            MockNode {
                role,
                label,
                colspan,
                rowspan,
                children: Vec::new(),
                layout: None,
                width,
                height,
                border,
                padding,
            },
        );
        id
    }

    fn add_child(&mut self, parent: u32, child: u32) {
        if let Some(node) = self.nodes.get_mut(&parent) {
            node.children.push(child);
        }
    }

    pub fn layout(&self, id: u32) -> Option<&CellLayout> {
        self.nodes.get(&id)?.layout.as_ref()
    }

    pub fn label(&self, id: u32) -> Option<&str> {
        self.nodes.get(&id)?.label.as_deref()
    }

    /// Returns all node IDs whose role matches `role`, sorted by ID (= allocation order).
    pub fn nodes_with_role(&self, role: TableRole) -> Vec<u32> {
        let mut ids: Vec<u32> = self
            .nodes
            .iter()
            .filter(|(_, n)| n.role == role)
            .map(|(&id, _)| id)
            .collect();
        ids.sort_unstable();
        ids
    }
}

impl TableTree for MockTree {
    type NodeId = u32;

    fn children(&self, id: u32) -> Vec<u32> {
        self.nodes.get(&id).map(|n| n.children.clone()).unwrap_or_default()
    }

    fn table_role(&self, id: u32) -> TableRole {
        self.nodes.get(&id).map(|n| n.role).unwrap_or(TableRole::Other)
    }

    fn css_length(&self, id: u32, prop: CssProp) -> CssLength {
        let Some(node) = self.nodes.get(&id) else {
            return CssLength::Auto;
        };
        match prop {
            CssProp::Width => node.width.map(CssLength::Px).unwrap_or(CssLength::Auto),
            CssProp::Height => node.height.map(CssLength::Px).unwrap_or(CssLength::Auto),
            CssProp::BorderTopWidth
            | CssProp::BorderRightWidth
            | CssProp::BorderBottomWidth
            | CssProp::BorderLeftWidth => CssLength::Px(node.border),
            CssProp::PaddingTop | CssProp::PaddingRight | CssProp::PaddingBottom | CssProp::PaddingLeft => {
                CssLength::Px(node.padding)
            }
            CssProp::BorderSpacingX => CssLength::Px(self.border_spacing_x),
            CssProp::BorderSpacingY => CssLength::Px(self.border_spacing_y),
            _ => CssLength::Auto,
        }
    }

    fn attr_usize(&self, id: u32, attr: &str) -> Option<usize> {
        let node = self.nodes.get(&id)?;
        match attr {
            "colspan" => Some(node.colspan),
            "rowspan" => Some(node.rowspan),
            _ => None,
        }
    }

    fn set_layout(&mut self, id: u32, layout: CellLayout) {
        if let Some(node) = self.nodes.get_mut(&id) {
            node.layout = Some(layout);
        }
    }

    fn layout_cell(&mut self, _id: u32, _available_width: f32) -> f32 {
        // MockTree carries no real child content — row heights are driven by
        // explicit CSS `height` values set on the cell nodes instead.
        0.0
    }
}

// ASCII renderer

/// Render the table stored in `tree` as an ASCII-art string.
///
/// Requires that `compute_table_layout` has already been called on the tree.
pub fn render_tree(tree: &MockTree, root: u32) -> String {
    let model = build_model(tree, root);

    let header_grids: Vec<SectionGrid<u32>> = model
        .header_groups
        .iter()
        .map(|g| build_section_grid(&g.rows))
        .collect();
    let body_grids: Vec<SectionGrid<u32>> = model.row_groups.iter().map(|g| build_section_grid(&g.rows)).collect();
    let footer_grids: Vec<SectionGrid<u32>> = model
        .footer_groups
        .iter()
        .map(|g| build_section_grid(&g.rows))
        .collect();

    let n_cols = header_grids
        .iter()
        .chain(body_grids.iter())
        .chain(footer_grids.iter())
        .map(|g| g.n_cols)
        .max()
        .unwrap_or(0);

    if n_cols == 0 {
        return String::from("(empty table)");
    }

    // Determine character width for each column from computed layouts.
    // Use the first single-column cell found for each column.
    let mut col_char_w: Vec<usize> = vec![0; n_cols];

    for grids in [&header_grids, &body_grids, &footer_grids] {
        for grid in grids {
            for cell in grid.cells() {
                if cell.colspan == 1 {
                    if let Some(layout) = tree.layout(cell.node) {
                        let w = layout.size.width.round() as usize;
                        if col_char_w[cell.col] < w {
                            col_char_w[cell.col] = w;
                        }
                    }
                }
            }
        }
    }
    // Fallback: ensure every column is at least 5 chars.
    for w in &mut col_char_w {
        if *w == 0 {
            *w = 5;
        }
    }

    let mut out = String::new();

    #[allow(clippy::type_complexity)]
    let sections: &[(&[RowGroup<u32>], &[SectionGrid<u32>], SectionKind)] = &[
        (&model.header_groups, &header_grids, SectionKind::Header),
        (&model.row_groups, &body_grids, SectionKind::Body),
        (&model.footer_groups, &footer_grids, SectionKind::Footer),
    ];

    // Draw the very first top border once, then each row only draws its bottom.
    // The bottom of the last row in a section doubles as the visual separator to the
    // next section — so we never get a double-line between sections.
    let no_spans = vec![false; n_cols];
    let first_non_empty = sections.iter().find(|(g, _, _)| !g.is_empty());
    if let Some((_, grids, _)) = first_non_empty {
        if let Some(grid) = grids.first() {
            out.push_str(&h_border(&col_char_w, n_cols, '=', &no_spans, 0, grid));
            out.push('\n');
        }
    }

    for (groups, grids, kind) in sections {
        if groups.is_empty() {
            continue;
        }

        for (_group, grid) in groups.iter().zip(grids.iter()) {
            for row_idx in 0..grid.n_rows {
                // Content row.
                out.push_str(&content_row(tree, grid, row_idx, &col_char_w, n_cols));
                out.push('\n');

                // Bottom border: '=' at section edges (last row of group), '-' between rows.
                let is_last_row = row_idx == grid.n_rows - 1;
                let bottom_ch = if is_last_row || *kind != SectionKind::Body {
                    '='
                } else {
                    '-'
                };
                // Which columns have a rowspan cell crossing this bottom border.
                let row_spanned = grid.cols_spanned_at_boundary(row_idx);
                out.push_str(&h_border(&col_char_w, n_cols, bottom_ch, &row_spanned, row_idx, grid));
                out.push('\n');
            }
        }
    }

    out
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum SectionKind {
    Header,
    Body,
    Footer,
}

/// Draw a horizontal border line, correctly handling both colspan (no `+` inside
/// a spanning cell horizontally) and rowspan (no `---` where a cell continues
/// vertically into the next row).
///
/// `row_spanned[c]` = `true` means column `c` has a rowspan cell that crosses
/// this boundary — so the separator for that column is spaces, not dashes.
fn h_border(
    col_char_w: &[usize],
    n_cols: usize,
    fill: char,
    col_spanned: &[bool], // rowspan through this boundary, per column
    colspan_row: usize,   // row index to read colspan info from
    grid: &SectionGrid<u32>,
) -> String {
    // Which columns have NO internal right wall due to colspan.
    let mut no_right_wall: Vec<bool> = vec![false; n_cols];
    for cell in grid.cells_in_row(colspan_row) {
        for c in cell.col..cell.col + cell.colspan.saturating_sub(1) {
            if c < no_right_wall.len() {
                no_right_wall[c] = true;
            }
        }
    }

    // Junction character between column `left` and column `right` (right = left+1).
    // Also used for the leftmost (+0) and rightmost (+n_cols) junctions.
    let junction = |pos: usize| -> char {
        let left_spanned = pos > 0 && col_spanned.get(pos - 1).copied().unwrap_or(false);
        let right_spanned = pos < n_cols && col_spanned.get(pos).copied().unwrap_or(false);

        if pos == 0 {
            // Leftmost edge
            if right_spanned {
                '|'
            } else {
                '+'
            }
        } else if pos == n_cols {
            // Rightmost edge — always a real corner
            '+'
        } else if left_spanned && right_spanned {
            // Both neighbours are spanning cells → vertical wall between them
            '|'
        } else if no_right_wall.get(pos - 1).copied().unwrap_or(false) {
            // Inside a colspan — suppress the junction
            if left_spanned || right_spanned {
                ' '
            } else {
                fill
            }
        } else {
            '+'
        }
    };

    let mut s = String::new();
    s.push(junction(0));
    for col in 0..n_cols {
        let w = col_char_w.get(col).copied().unwrap_or(5);
        let f = if col_spanned.get(col).copied().unwrap_or(false) {
            ' '
        } else {
            fill
        };
        s.extend(std::iter::repeat_n(f, w));
        s.push(junction(col + 1));
    }
    s
}

/// Draw one content row (cells and their labels).
fn content_row(
    tree: &MockTree,
    grid: &SectionGrid<u32>,
    row_idx: usize,
    col_char_w: &[usize],
    n_cols: usize,
) -> String {
    // Map col → placed cell for cells that START in this row.
    let mut cell_at: HashMap<usize, &PlacedCell<u32>> = HashMap::new();
    for cell in grid.cells_in_row(row_idx) {
        cell_at.insert(cell.col, cell);
    }

    let mut s = String::from("|");
    let mut col = 0;

    while col < n_cols {
        if let Some(placed) = cell_at.get(&col) {
            let colspan = placed.colspan;
            // Total character width = sum of column widths + internal '|'s consumed.
            let inner_w: usize =
                col_char_w.get(col..col + colspan).unwrap_or(&[]).iter().sum::<usize>() + colspan.saturating_sub(1);
            let label = tree.label(placed.node).unwrap_or("");
            s.push_str(&pad_center(label, inner_w));
            s.push('|');
            col += colspan;
        } else {
            // Column occupied by a rowspan cell that started in an earlier row.
            // Show as blank — the cell's wall on the right is `|` as usual.
            let w = col_char_w.get(col).copied().unwrap_or(5);
            s.extend(std::iter::repeat_n(' ', w));
            s.push('|');
            col += 1;
        }
    }

    s
}

fn pad_center(text: &str, width: usize) -> String {
    if text.len() >= width {
        return text[..width].to_string();
    }
    let total_pad = width - text.len();
    let left = total_pad / 2;
    let right = total_pad - left;
    format!("{}{}{}", " ".repeat(left), text, " ".repeat(right))
}
