//! Integration tests for `compute_table_layout`.
//!
//! Each test builds a table via `MockTable`, runs the layout algorithm, then
//! asserts exact positions and sizes.  All coordinates follow the CSS model:
//!   - group position is relative to the table
//!   - row position is relative to its group
//!   - cell position is relative to its row
//!
//! Naming convention: `cell_ids[N]` returns cells in document (allocation) order.

#[cfg(test)]
mod layout_tests {
    use crate::compute::compute_table_layout;
    use crate::mock::{cell, MockTable};
    use crate::types::TableRole;

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.01
    }

    macro_rules! assert_approx {
        ($a:expr, $b:expr, $msg:literal) => {
            assert!(approx($a, $b), "{}: expected {}, got {}", $msg, $b, $a);
        };
    }

    // 1. Basic geometry: 2×2 body-only table, no spacing, no border/padding
    #[test]
    fn basic_2x2_positions() {
        // 100px wide, 2 equal auto columns = 50px each, two rows with explicit heights.
        let (mut tree, root) = MockTable::new(100.0)
            .spacing(0.0, 0.0)
            .body_row(vec![
                cell("A").height(20.0).padding(0.0),
                cell("B").height(30.0).padding(0.0),
            ])
            .body_row(vec![
                cell("C").height(15.0).padding(0.0),
                cell("D").height(10.0).padding(0.0),
            ])
            .into_tree();

        let (w, h) = compute_table_layout(&mut tree, root, 100.0, None).expect("layout must succeed");

        // Table size
        assert_approx!(w, 100.0, "table width");
        assert_approx!(h, 45.0, "table height (30 + 15 rows, no spacing)");

        // Row heights: max per-row across cells.
        let rows = tree.nodes_with_role(TableRole::Row);
        assert_eq!(rows.len(), 2);
        let r0 = tree.layout(rows[0]).expect("row0 layout");
        let r1 = tree.layout(rows[1]).expect("row1 layout");
        assert_approx!(r0.size.height, 30.0, "row0 height = max(20,30)");
        assert_approx!(r1.size.height, 15.0, "row1 height = max(15,10)");

        // Row positions (relative to group, spacing_y=0).
        assert_approx!(r0.position.y, 0.0, "row0 y");
        assert_approx!(r1.position.y, 30.0, "row1 y");

        // Cell positions (relative to their row, spacing_x=0).
        let cells = tree.nodes_with_role(TableRole::Cell);
        assert_eq!(cells.len(), 4);
        let (ca, cb, cc, cd) = (cells[0], cells[1], cells[2], cells[3]);

        let la = tree.layout(ca).expect("cell A");
        assert_approx!(la.position.x, 0.0, "A x");
        assert_approx!(la.position.y, 0.0, "A y");
        assert_approx!(la.size.width, 50.0, "A width");
        assert_approx!(la.size.height, 30.0, "A height = row height");

        let lb = tree.layout(cb).expect("cell B");
        assert_approx!(lb.position.x, 50.0, "B x");
        assert_approx!(lb.size.height, 30.0, "B height = row height");

        let lc = tree.layout(cc).expect("cell C");
        assert_approx!(lc.position.x, 0.0, "C x");
        assert_approx!(lc.size.height, 15.0, "C height = row height");

        let ld = tree.layout(cd).expect("cell D");
        assert_approx!(ld.position.x, 50.0, "D x");
        assert_approx!(ld.size.height, 15.0, "D height = row height");
    }

    // 2. Border-spacing shifts column x-offsets
    #[test]
    fn spacing_shifts_cell_x() {
        // spacing_x=4: gutters are 4px on each side and between columns.
        // 3 gutters for 2 columns → 12px consumed. 100-12 = 88 for content → 44px each.
        // col_x[0] = 4, col_x[1] = 4+44+4 = 52.
        let (mut tree, root) = MockTable::new(100.0)
            .spacing(4.0, 0.0)
            .body_row(vec![
                cell("L").height(10.0).padding(0.0),
                cell("R").height(10.0).padding(0.0),
            ])
            .into_tree();

        compute_table_layout(&mut tree, root, 100.0, None).expect("layout");

        let cells = tree.nodes_with_role(TableRole::Cell);
        let ll = tree.layout(cells[0]).expect("left cell");
        let lr = tree.layout(cells[1]).expect("right cell");

        assert_approx!(ll.position.x, 4.0, "left cell x (first gutter)");
        assert_approx!(ll.size.width, 44.0, "left cell width");
        assert_approx!(lr.position.x, 52.0, "right cell x");
        assert_approx!(lr.size.width, 44.0, "right cell width");
    }

    // 3. Explicit column width + auto remainder
    #[test]
    fn explicit_column_width() {
        // First row: cell with explicit width=30px.
        // Second column: auto → gets (100 - 30) = 70px.
        // spacing = 0.
        let (mut tree, root) = MockTable::new(100.0)
            .spacing(0.0, 0.0)
            .body_row(vec![
                cell("Narrow").width(30.0).height(10.0).padding(0.0),
                cell("Wide").height(10.0).padding(0.0),
            ])
            .into_tree();

        compute_table_layout(&mut tree, root, 100.0, None).expect("layout");

        let cells = tree.nodes_with_role(TableRole::Cell);
        let l0 = tree.layout(cells[0]).expect("narrow cell");
        let l1 = tree.layout(cells[1]).expect("wide cell");

        assert_approx!(l0.size.width, 30.0, "explicit-width col");
        assert_approx!(l1.size.width, 70.0, "auto col gets remainder");
        assert_approx!(l0.position.x, 0.0, "col0 x");
        assert_approx!(l1.position.x, 30.0, "col1 x");
    }

    // 4. Rowspan: cell spans 2 rows, gets combined height
    #[test]
    fn rowspan_cell_height() {
        // Row0: spanning cell (rowspan=2) + regular cell h=20
        // Row1: just a regular cell h=30 (spanning cell occupies col0)
        // Row heights: row0 = max(h_span_content=0, 20) = 20
        //              row1 = 30
        // Spanning cell height = 20 + 30 = 50.
        let (mut tree, root) = MockTable::new(100.0)
            .spacing(0.0, 0.0)
            .body_row(vec![
                cell("Span").rowspan(2).padding(0.0),
                cell("R0C1").height(20.0).padding(0.0),
            ])
            .body_row(vec![cell("R1C1").height(30.0).padding(0.0)])
            .into_tree();

        compute_table_layout(&mut tree, root, 100.0, None).expect("layout");

        let cells = tree.nodes_with_role(TableRole::Cell);
        // cells[0] = Span, cells[1] = R0C1, cells[2] = R1C1
        let span_layout = tree.layout(cells[0]).expect("span cell");
        assert_approx!(span_layout.size.height, 50.0, "rowspan=2 height = 20+30");
        assert_approx!(span_layout.size.width, 50.0, "span col width");
        assert_approx!(span_layout.position.x, 0.0, "span at col 0");
    }

    // 5. Colspan: cell spans 2 columns, gets combined width
    #[test]
    fn colspan_cell_width() {
        // Row0: header has 2 cells → 2 columns each 50px
        // Row1: colspan=2 spanning cell → width = 50+50 = 100px
        let (mut tree, root) = MockTable::new(100.0)
            .spacing(0.0, 0.0)
            .body_row(vec![
                cell("A").height(10.0).padding(0.0),
                cell("B").height(10.0).padding(0.0),
            ])
            .body_row(vec![cell("Wide").colspan(2).height(10.0).padding(0.0)])
            .into_tree();

        compute_table_layout(&mut tree, root, 100.0, None).expect("layout");

        let cells = tree.nodes_with_role(TableRole::Cell);
        // cells[0]=A, cells[1]=B, cells[2]=Wide
        let wide = tree.layout(cells[2]).expect("wide cell");
        assert_approx!(wide.size.width, 100.0, "colspan=2 gets full width");
        assert_approx!(wide.position.x, 0.0, "colspan cell starts at col 0");
    }

    // 6. thead + tbody + tfoot: sections stack vertically
    #[test]
    fn thead_tbody_tfoot_stack() {
        // Each section has 1 row with 1 cell of height=10.
        // No spacing. Total height = 30.
        let (mut tree, root) = MockTable::new(100.0)
            .spacing(0.0, 0.0)
            .header_row(vec![cell("H").height(10.0).padding(0.0)])
            .body_row(vec![cell("B").height(10.0).padding(0.0)])
            .footer_row(vec![cell("F").height(10.0).padding(0.0)])
            .into_tree();

        let (_, total_h) = compute_table_layout(&mut tree, root, 100.0, None).expect("layout");

        assert_approx!(total_h, 30.0, "three sections of 10px each");

        // Groups should stack at y=0, y=10, y=20 (header first, then body, footer).
        let groups: Vec<u32> = {
            let mut v = tree.nodes_with_role(TableRole::HeaderGroup);
            v.extend(tree.nodes_with_role(TableRole::RowGroup));
            v.extend(tree.nodes_with_role(TableRole::FooterGroup));
            v
        };
        assert_eq!(groups.len(), 3);

        let g0 = tree.layout(groups[0]).expect("header group");
        let g1 = tree.layout(groups[1]).expect("body group");
        let g2 = tree.layout(groups[2]).expect("footer group");

        assert_approx!(g0.position.y, 0.0, "header group y");
        assert_approx!(g1.position.y, 10.0, "body group y");
        assert_approx!(g2.position.y, 20.0, "footer group y");
    }

    // 7. Border + padding widen the effective cell box
    #[test]
    fn border_and_padding_in_cell_layout() {
        // Cell with border=1, padding=2 on every side.
        // The CellLayout.border and .padding fields should reflect those values.
        let (mut tree, root) = MockTable::new(100.0)
            .spacing(0.0, 0.0)
            .body_row(vec![cell("Bordered").height(20.0).border(1.0).padding(2.0)])
            .into_tree();

        compute_table_layout(&mut tree, root, 100.0, None).expect("layout");

        let cells = tree.nodes_with_role(TableRole::Cell);
        let l = tree.layout(cells[0]).expect("cell layout");

        assert_approx!(l.border.top, 1.0, "border top");
        assert_approx!(l.border.right, 1.0, "border right");
        assert_approx!(l.padding.left, 2.0, "padding left");
        assert_approx!(l.padding.bottom, 2.0, "padding bottom");
    }

    // 8. Empty table returns (0, 0) without panicking
    #[test]
    fn empty_table_returns_zero() {
        let (mut tree, root) = MockTable::new(100.0).into_tree();
        let (w, h) = compute_table_layout(&mut tree, root, 100.0, None).expect("empty table must not fail");
        assert_approx!(w, 0.0, "empty table width");
        assert_approx!(h, 0.0, "empty table height");
    }
}
