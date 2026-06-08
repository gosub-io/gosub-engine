use crate::grid::SectionGrid;
use crate::types::{BoxEdges, CssProp, CssLength};
use crate::TableTree;

/// Compute the height of each row in a section.
///
/// For each non-spanning cell we:
/// 1. Call [`TableTree::layout_cell`] to let the implementor run normal layout
///    (block/flex/inline) inside the cell and get the actual content height.
/// 2. Also read any explicit CSS `height` on the cell.
/// 3. Take the maximum of the two, add the cell's own border + padding, and
///    use that as the candidate height for the row.
///
/// Cells with `rowspan > 1` are skipped here; their height distribution across
/// multiple rows is a Phase 2 concern.
pub fn compute_row_heights<T: TableTree>(
    tree: &mut T,
    grid: &SectionGrid<T::NodeId>,
    col_widths: &[f32],
) -> Vec<f32> {
    let mut heights = vec![0.0_f32; grid.n_rows];

    for cell in grid.cells() {
        if cell.rowspan != 1 {
            continue;
        }

        let border  = read_border(tree, cell.node);
        let padding = read_padding(tree, cell.node);

        // Inner width available to the cell's children.
        let cell_col_w: f32 = col_widths
            .get(cell.col..cell.col + cell.colspan)
            .unwrap_or(&[])
            .iter()
            .sum();
        let inner_w = (cell_col_w - border.horizontal() - padding.horizontal()).max(0.0);

        // Ask the implementor to lay out the cell's children and report their height.
        let content_h = tree.layout_cell(cell.node, inner_w);

        // Explicit CSS `height` is a minimum — content can be taller.
        let explicit_h = match tree.css_length(cell.node, CssProp::Height) {
            CssLength::Px(px) => px,
            CssLength::Zero => 0.0,
            _ => 0.0,
        };

        let cell_h = content_h.max(explicit_h) + border.vertical() + padding.vertical();
        if cell_h > heights[cell.row] {
            heights[cell.row] = cell_h;
        }
    }

    heights
}

// ---------------------------------------------------------------------------
// Helpers shared with compute.rs
// ---------------------------------------------------------------------------

pub(crate) fn read_border<T: TableTree>(tree: &T, node: T::NodeId) -> BoxEdges {
    BoxEdges {
        top:    tree.css_length(node, CssProp::BorderTopWidth).px_or(0.0),
        right:  tree.css_length(node, CssProp::BorderRightWidth).px_or(0.0),
        bottom: tree.css_length(node, CssProp::BorderBottomWidth).px_or(0.0),
        left:   tree.css_length(node, CssProp::BorderLeftWidth).px_or(0.0),
    }
}

pub(crate) fn read_padding<T: TableTree>(tree: &T, node: T::NodeId) -> BoxEdges {
    BoxEdges {
        top:    tree.css_length(node, CssProp::PaddingTop).px_or(0.0),
        right:  tree.css_length(node, CssProp::PaddingRight).px_or(0.0),
        bottom: tree.css_length(node, CssProp::PaddingBottom).px_or(0.0),
        left:   tree.css_length(node, CssProp::PaddingLeft).px_or(0.0),
    }
}
