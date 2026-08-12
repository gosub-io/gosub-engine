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

    // 9. Vertical border-spacing: one gutter above the first row, one between
    //    rows, one below the last - never doubled at group boundaries.
    #[test]
    fn vertical_spacing_gutters() {
        let (mut tree, root) = MockTable::new(100.0)
            .spacing(0.0, 5.0)
            .body_row(vec![cell("A").height(10.0).padding(0.0)])
            .body_row(vec![cell("B").height(20.0).padding(0.0)])
            .into_tree();

        let (_, h) = compute_table_layout(&mut tree, root, 100.0, None).expect("layout");
        assert_approx!(h, 45.0, "5 + 10 + 5 + 20 + 5");

        let groups = tree.nodes_with_role(TableRole::RowGroup);
        let g = tree.layout(groups[0]).expect("group layout");
        assert_approx!(g.position.y, 5.0, "group starts below the top gutter");
        assert_approx!(g.size.height, 35.0, "10 + 5 + 20 (internal gutter only)");

        let rows = tree.nodes_with_role(TableRole::Row);
        let r0 = tree.layout(rows[0]).expect("row0");
        let r1 = tree.layout(rows[1]).expect("row1");
        assert_approx!(r0.position.y, 0.0, "row0 at group top");
        assert_approx!(r1.position.y, 15.0, "row1 after row0 + gutter");
    }

    // 10. Vertical spacing across group boundaries: exactly one gutter between
    //     the last row of one section and the first row of the next.
    #[test]
    fn vertical_spacing_between_sections() {
        let (mut tree, root) = MockTable::new(100.0)
            .spacing(0.0, 5.0)
            .header_row(vec![cell("H").height(10.0).padding(0.0)])
            .body_row(vec![cell("B").height(10.0).padding(0.0)])
            .into_tree();

        let (_, h) = compute_table_layout(&mut tree, root, 100.0, None).expect("layout");
        assert_approx!(h, 35.0, "5 + 10 + 5 + 10 + 5");

        let hg = tree.nodes_with_role(TableRole::HeaderGroup);
        let bg = tree.nodes_with_role(TableRole::RowGroup);
        assert_approx!(tree.layout(hg[0]).expect("hg").position.y, 5.0, "header group y");
        assert_approx!(
            tree.layout(bg[0]).expect("bg").position.y,
            20.0,
            "body group y = 5 + 10 + 5"
        );
    }

    // 11. A colspan cell covers the horizontal gutters between its columns.
    #[test]
    fn colspan_covers_gutter() {
        // spacing_x=4 → available = 100 - 3*4 = 88 → two 44px columns.
        let (mut tree, root) = MockTable::new(100.0)
            .spacing(4.0, 0.0)
            .body_row(vec![
                cell("A").height(10.0).padding(0.0),
                cell("B").height(10.0).padding(0.0),
            ])
            .body_row(vec![cell("Wide").colspan(2).height(10.0).padding(0.0)])
            .into_tree();

        compute_table_layout(&mut tree, root, 100.0, None).expect("layout");

        let cells = tree.nodes_with_role(TableRole::Cell);
        let wide = tree.layout(cells[2]).expect("wide cell");
        assert_approx!(wide.size.width, 92.0, "44 + 4 (gutter) + 44");
        assert_approx!(wide.position.x, 4.0, "starts after the first gutter");
    }

    // 12. A rowspan cell covers the vertical gutters between its rows.
    #[test]
    fn rowspan_covers_gutter() {
        let (mut tree, root) = MockTable::new(100.0)
            .spacing(0.0, 6.0)
            .body_row(vec![
                cell("Span").rowspan(2).padding(0.0),
                cell("R0").height(20.0).padding(0.0),
            ])
            .body_row(vec![cell("R1").height(30.0).padding(0.0)])
            .into_tree();

        compute_table_layout(&mut tree, root, 100.0, None).expect("layout");

        let cells = tree.nodes_with_role(TableRole::Cell);
        let span = tree.layout(cells[0]).expect("span cell");
        assert_approx!(span.size.height, 56.0, "20 + 6 (gutter) + 30");
    }

    // 13. Sections render header → body → footer regardless of source order.
    #[test]
    fn sections_reordered_from_source_order() {
        use crate::mock::MockTree;

        // Source order: footer, body, header - like <tfoot> before <tbody> in HTML.
        let mut tree = MockTree::new(0.0, 0.0);
        let root = tree.alloc(TableRole::Table, None, 1, 1, None, None, 0.0, 0.0);
        for (role, label, h) in [
            (TableRole::FooterGroup, "F", 30.0),
            (TableRole::RowGroup, "B", 20.0),
            (TableRole::HeaderGroup, "H", 10.0),
        ] {
            let group = tree.alloc(role, None, 1, 1, None, None, 0.0, 0.0);
            tree.add_child(root, group);
            let row = tree.alloc(TableRole::Row, None, 1, 1, None, None, 0.0, 0.0);
            tree.add_child(group, row);
            let c = tree.alloc_cell(cell(label).height(h).padding(0.0));
            tree.add_child(row, c);
        }

        let (_, total_h) = compute_table_layout(&mut tree, root, 100.0, None).expect("layout");
        assert_approx!(total_h, 60.0, "10 + 20 + 30");

        let hg = tree.nodes_with_role(TableRole::HeaderGroup);
        let bg = tree.nodes_with_role(TableRole::RowGroup);
        let fg = tree.nodes_with_role(TableRole::FooterGroup);
        assert_approx!(tree.layout(hg[0]).expect("header").position.y, 0.0, "header first");
        assert_approx!(tree.layout(bg[0]).expect("body").position.y, 10.0, "body second");
        assert_approx!(tree.layout(fg[0]).expect("footer").position.y, 30.0, "footer last");
    }

    // 14. Anonymous fixup: a bare row directly under the table gets an
    //     anonymous body group (CSS 2.1 §17.2.1).
    #[test]
    fn anonymous_group_for_bare_row() {
        use crate::mock::MockTree;

        let mut tree = MockTree::new(0.0, 0.0);
        let root = tree.alloc(TableRole::Table, None, 1, 1, None, None, 0.0, 0.0);
        let row = tree.alloc(TableRole::Row, None, 1, 1, None, None, 0.0, 0.0);
        tree.add_child(root, row);
        for label in ["A", "B"] {
            let c = tree.alloc_cell(cell(label).height(10.0).padding(0.0));
            tree.add_child(row, c);
        }

        let (w, h) = compute_table_layout(&mut tree, root, 100.0, None).expect("layout");
        assert_approx!(w, 100.0, "table width");
        assert_approx!(h, 10.0, "row height");

        let cells = tree.nodes_with_role(TableRole::Cell);
        let lb = tree.layout(cells[1]).expect("cell B laid out");
        assert_approx!(lb.size.width, 50.0, "two equal auto columns");
        assert_approx!(lb.position.x, 50.0, "second column x");
        assert!(tree.layout(row).is_some(), "the real row node gets a layout");
    }

    // 15. Anonymous fixup: a bare cell directly under a row group gets an
    //     anonymous row.
    #[test]
    fn anonymous_row_for_bare_cell() {
        use crate::mock::MockTree;

        let mut tree = MockTree::new(0.0, 0.0);
        let root = tree.alloc(TableRole::Table, None, 1, 1, None, None, 0.0, 0.0);
        let group = tree.alloc(TableRole::RowGroup, None, 1, 1, None, None, 0.0, 0.0);
        tree.add_child(root, group);
        let c = tree.alloc_cell(cell("Bare").height(12.0).padding(0.0));
        tree.add_child(group, c);

        let (_, h) = compute_table_layout(&mut tree, root, 100.0, None).expect("layout");
        assert_approx!(h, 12.0, "cell height drives the table");

        let l = tree.layout(c).expect("bare cell laid out");
        assert_approx!(l.size.width, 100.0, "single column takes full width");
    }

    // 16. Rowspan never escapes its section: a rowspan=3 in a 1-row header is
    //     clamped and does not reach into the body rows.
    #[test]
    fn rowspan_clamped_to_section() {
        let (mut tree, root) = MockTable::new(100.0)
            .spacing(0.0, 0.0)
            .header_row(vec![cell("H").rowspan(3).height(10.0).padding(0.0)])
            .body_row(vec![cell("B").height(30.0).padding(0.0)])
            .into_tree();

        compute_table_layout(&mut tree, root, 100.0, None).expect("layout");

        let cells = tree.nodes_with_role(TableRole::Cell);
        let h_cell = tree.layout(cells[0]).expect("header cell");
        assert_approx!(h_cell.size.height, 10.0, "clamped to the header's single row");
    }

    // 17. Auto columns share space proportionally to their natural content width.
    #[test]
    fn proportional_content_width_distribution() {
        // Both columns are "content" columns (natural >= 50px threshold).
        let (mut tree, root) = MockTable::new(200.0)
            .spacing(0.0, 0.0)
            .body_row(vec![
                cell("A").content_width(60.0).height(10.0).padding(0.0),
                cell("B").content_width(140.0).height(10.0).padding(0.0),
            ])
            .into_tree();

        compute_table_layout(&mut tree, root, 200.0, None).expect("layout");

        let cells = tree.nodes_with_role(TableRole::Cell);
        assert_approx!(tree.layout(cells[0]).expect("A").size.width, 60.0, "200 * 60/200");
        assert_approx!(tree.layout(cells[1]).expect("B").size.width, 140.0, "200 * 140/200");
    }

    // 18. Narrow columns (natural < 50px) keep their natural width; wide
    //     content columns absorb the remaining space.
    #[test]
    fn narrow_column_keeps_natural_width() {
        let (mut tree, root) = MockTable::new(200.0)
            .spacing(0.0, 0.0)
            .body_row(vec![
                cell("rank").content_width(20.0).height(10.0).padding(0.0),
                cell("story").content_width(100.0).height(10.0).padding(0.0),
            ])
            .into_tree();

        compute_table_layout(&mut tree, root, 200.0, None).expect("layout");

        let cells = tree.nodes_with_role(TableRole::Cell);
        assert_approx!(
            tree.layout(cells[0]).expect("rank").size.width,
            20.0,
            "narrow keeps natural"
        );
        assert_approx!(
            tree.layout(cells[1]).expect("story").size.width,
            180.0,
            "content col absorbs rest"
        );
    }

    // 19. Very narrow columns are floored at 14px so they stay visible.
    #[test]
    fn narrow_column_floor() {
        let (mut tree, root) = MockTable::new(200.0)
            .spacing(0.0, 0.0)
            .body_row(vec![
                cell("dot").content_width(5.0).height(10.0).padding(0.0),
                cell("text").content_width(100.0).height(10.0).padding(0.0),
            ])
            .into_tree();

        compute_table_layout(&mut tree, root, 200.0, None).expect("layout");

        let cells = tree.nodes_with_role(TableRole::Cell);
        assert_approx!(tree.layout(cells[0]).expect("dot").size.width, 14.0, "floored at 14px");
        assert_approx!(tree.layout(cells[1]).expect("text").size.width, 186.0, "200 - 14");
    }

    // 20. An explicit CSS width cannot shrink a column below its content's
    //     natural width (used width = max(specified, min-content)).
    #[test]
    fn explicit_width_clamped_to_content() {
        let (mut tree, root) = MockTable::new(100.0)
            .spacing(0.0, 0.0)
            .body_row(vec![
                cell("img").width(18.0).content_width(20.0).height(10.0).padding(0.0),
                cell("rest").height(10.0).padding(0.0),
            ])
            .into_tree();

        compute_table_layout(&mut tree, root, 100.0, None).expect("layout");

        let cells = tree.nodes_with_role(TableRole::Cell);
        assert_approx!(
            tree.layout(cells[0]).expect("img").size.width,
            20.0,
            "width:18 clamped up to 20px content"
        );
        assert_approx!(
            tree.layout(cells[1]).expect("rest").size.width,
            80.0,
            "auto col gets remainder"
        );
    }

    // 21. Explicit CSS height is a minimum: taller content grows the row.
    #[test]
    fn explicit_height_is_minimum() {
        let (mut tree, root) = MockTable::new(100.0)
            .spacing(0.0, 0.0)
            .body_row(vec![cell("tall").height(10.0).content_height(30.0).padding(0.0)])
            .into_tree();

        let (_, h) = compute_table_layout(&mut tree, root, 100.0, None).expect("layout");
        assert_approx!(h, 30.0, "content height 30 wins over explicit 10");
    }

    // 22. Content height comes from layout_cell and gets border + padding added.
    #[test]
    fn content_height_plus_border_padding() {
        let (mut tree, root) = MockTable::new(100.0)
            .spacing(0.0, 0.0)
            .body_row(vec![cell("boxed").content_height(10.0).border(1.0).padding(2.0)])
            .into_tree();

        let (_, h) = compute_table_layout(&mut tree, root, 100.0, None).expect("layout");
        assert_approx!(h, 16.0, "10 content + 2*1 border + 2*2 padding");
    }

    // 23. Nested tables: lattice treats a nested table as opaque cell content -
    //     the host's `layout_cell` recurses (as the pipeline's post_process_tables
    //     does). This exercises the cooperation contract: the outer column width
    //     flows *down* as the inner table's available width, and the inner
    //     table's computed height flows back *up* into the outer row height.
    #[test]
    fn nested_table_via_layout_cell() {
        use crate::mock::MockTree;
        use crate::types::{CellLayout, CssLength, CssProp};
        use crate::TableTree;

        /// Delegates everything to `outer`, except that laying out `host_cell`
        /// runs a full table layout on `inner` - a table inside a table.
        struct NestedTree {
            outer: MockTree,
            inner: MockTree,
            inner_root: u32,
            host_cell: u32,
            inner_size: Option<(f32, f32)>,
        }

        impl TableTree for NestedTree {
            type NodeId = u32;

            fn children(&self, id: u32) -> Vec<u32> {
                self.outer.children(id)
            }
            fn table_role(&self, id: u32) -> crate::types::TableRole {
                self.outer.table_role(id)
            }
            fn css_length(&self, id: u32, prop: CssProp) -> CssLength {
                self.outer.css_length(id, prop)
            }
            fn attr_usize(&self, id: u32, attr: &str) -> Option<usize> {
                self.outer.attr_usize(id, attr)
            }
            fn set_layout(&mut self, id: u32, layout: CellLayout) {
                self.outer.set_layout(id, layout);
            }
            fn cell_content_width(&self, id: u32) -> f32 {
                self.outer.cell_content_width(id)
            }

            fn layout_cell(&mut self, id: u32, available_width: f32) -> f32 {
                if id == self.host_cell {
                    let (w, h) = compute_table_layout(&mut self.inner, self.inner_root, available_width, None)
                        .expect("inner layout");
                    self.inner_size = Some((w, h));
                    h
                } else {
                    self.outer.layout_cell(id, available_width)
                }
            }
        }

        // Outer: 200px wide, one row, two auto columns (100px each).
        let (outer, outer_root) = MockTable::new(200.0)
            .spacing(0.0, 0.0)
            .body_row(vec![cell("host").padding(0.0), cell("plain").height(10.0).padding(0.0)])
            .into_tree();
        let host_cell = outer.nodes_with_role(TableRole::Cell)[0];

        // Inner: two columns × two rows, heights 15 and 25.
        let (inner, inner_root) = MockTable::new(0.0)
            .spacing(0.0, 0.0)
            .body_row(vec![
                cell("a").height(15.0).padding(0.0),
                cell("b").height(15.0).padding(0.0),
            ])
            .body_row(vec![
                cell("c").height(25.0).padding(0.0),
                cell("d").height(25.0).padding(0.0),
            ])
            .into_tree();

        let mut tree = NestedTree {
            outer,
            inner,
            inner_root,
            host_cell,
            inner_size: None,
        };

        let (_, outer_h) = compute_table_layout(&mut tree, outer_root, 200.0, None).expect("outer layout");

        // Width flowed down: the inner table was laid out at the host column's width.
        let (inner_w, inner_h) = tree.inner_size.expect("inner table was laid out");
        assert_approx!(inner_w, 100.0, "inner table width = outer column width");
        assert_approx!(inner_h, 40.0, "inner table height = 15 + 25");

        // Inner columns split the host cell's width, not the outer table's.
        let inner_cells = tree.inner.nodes_with_role(TableRole::Cell);
        let ia = tree.inner.layout(inner_cells[0]).expect("inner cell a");
        assert_approx!(ia.size.width, 50.0, "inner column = 100 / 2");

        // Height flowed up: the outer row grew to hold the inner table.
        assert_approx!(outer_h, 40.0, "outer row height = inner table height");
        let host_layout = tree.outer.layout(host_cell).expect("host cell");
        assert_approx!(host_layout.size.height, 40.0, "host cell height");
    }
}
