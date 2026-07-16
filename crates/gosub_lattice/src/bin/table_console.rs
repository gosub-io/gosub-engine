/// Console table renderer - run with:
///   cargo run --bin table_console -p gosub_lattice
///
/// Each demo mirrors an integration test in `src/tests.rs`, so the same
/// scenarios the tests assert numerically can be eyeballed here.
use gosub_lattice::mock::{cell, render_tree, MockTable, MockTree};
use gosub_lattice::{compute_table_layout, TableRole};

fn main() {
    // 1. Simple 3-column table, no spanning
    println!("=== Simple 3-column table ===");
    let out = MockTable::new(60.0)
        .header_row(vec![cell("Name"), cell("Age"), cell("City")])
        .body_row(vec![cell("Alice"), cell("30"), cell("Amsterdam")])
        .body_row(vec![cell("Bob"), cell("25"), cell("Berlin")])
        .body_row(vec![cell("Carol"), cell("42"), cell("Brussels")])
        .footer_row(vec![cell("3 people"), cell(""), cell("")])
        .render();
    println!("{out}");

    // 2. colspan: Bob's name spans 2 columns
    println!("=== colspan demo ===");
    let out = MockTable::new(60.0)
        .header_row(vec![cell("Name"), cell("Info")])
        .body_row(vec![cell("Alice"), cell("Amsterdam")])
        .body_row(vec![cell("Bob — long name that spans both columns").colspan(2)])
        .body_row(vec![cell("Carol"), cell("Brussels")])
        .render();
    println!("{out}");

    // 3. rowspan: Bob spans 2 body rows (clamped to the body section)
    println!("=== rowspan demo (clamped to body section) ===");
    let out = MockTable::new(60.0)
        .header_row(vec![cell("Name"), cell("Score")])
        .body_row(vec![cell("Alice (rowspan=2)").rowspan(2), cell("100")])
        .body_row(vec![cell("80")]) // col 0 occupied by rowspan
        .body_row(vec![cell("Carol"), cell("90")])
        .footer_row(vec![cell("Total"), cell("270")])
        .render();
    println!("{out}");

    // 4. rowspan that would cross thead→tbody boundary - must be clamped
    println!("=== rowspan clamped at section boundary ===");
    let out = MockTable::new(60.0)
        .header_row(vec![
            cell("Header rowspan=99").rowspan(99), // only 1 row in thead → clamped to 1
            cell("H2"),
        ])
        .body_row(vec![cell("Body A"), cell("Body B")])
        .render();
    println!("{out}");

    // 5. thead/tbody/tfoot all present, rowspan within tbody
    println!("=== thead + rowspan in tbody + tfoot ===");
    let out = MockTable::new(60.0)
        .header_row(vec![cell("Col A"), cell("Col B"), cell("Col C")])
        .body_row(vec![cell("Spans 2 rows").rowspan(2), cell("B1"), cell("C1")])
        .body_row(vec![cell("B2"), cell("C2")]) // col 0 spanned
        .body_row(vec![cell("A3"), cell("B3"), cell("C3")])
        .footer_row(vec![cell("Footer").colspan(3)])
        .render();
    println!("{out}");

    // 6. Content-driven column widths (tests 18/19): narrow columns keep their
    //    natural width (with a 14px floor), the wide content column absorbs the
    //    rest. Column sizing scans the first row, so content widths go there.
    println!("=== content-proportional columns (narrow cols keep natural width) ===");
    let out = MockTable::new(80.0)
        .body_row(vec![
            cell("1.").content_width(3.0),
            cell("A story title that wants all the room").content_width(52.0),
            cell("312").content_width(6.0),
        ])
        .body_row(vec![cell("2."), cell("Shorter title"), cell("41")])
        .render();
    println!("{out}");

    // 7. Explicit width clamped to content (test 20): width=6 on a cell whose
    //    content is naturally 12 wide - the column comes out 12, not 6.
    println!("=== explicit width clamped up to content width ===");
    let out = MockTable::new(50.0)
        .body_row(vec![
            cell("img w=6 c=12").width(6.0).content_width(12.0),
            cell("caption gets the rest"),
        ])
        .render();
    println!("{out}");

    // 8. Anonymous-box fixups (tests 14/15): bare rows directly under the table
    //    get an anonymous body group - the table renders as if wrapped in <tbody>.
    println!("=== anonymous group: bare rows directly under the table ===");
    let mut tree = MockTree::new(1.0, 0.0);
    let root = tree.alloc(TableRole::Table, None, 1, 1, None, None, 0.0, 0.0);
    for labels in [["bare", "row"], ["no", "tbody"]] {
        let row = tree.alloc(TableRole::Row, None, 1, 1, None, None, 0.0, 0.0);
        tree.add_child(root, row);
        for label in labels {
            let c = tree.alloc_cell(cell(label));
            tree.add_child(row, c);
        }
    }
    if compute_table_layout(&mut tree, root, 30.0, None).is_ok() {
        println!("{}", render_tree(&tree, root));
    }

    // 9. Section order normalization (test 13): source order is tfoot, tbody,
    //    thead - the layout renders header → body → footer regardless.
    println!("=== source order tfoot/tbody/thead → renders head/body/foot ===");
    let mut tree = MockTree::new(1.0, 0.0);
    let root = tree.alloc(TableRole::Table, None, 1, 1, None, None, 0.0, 0.0);
    for (role, label) in [
        (TableRole::FooterGroup, "footer (first in source)"),
        (TableRole::RowGroup, "body (second in source)"),
        (TableRole::HeaderGroup, "header (last in source)"),
    ] {
        let group = tree.alloc(role, None, 1, 1, None, None, 0.0, 0.0);
        tree.add_child(root, group);
        let row = tree.alloc(TableRole::Row, None, 1, 1, None, None, 0.0, 0.0);
        tree.add_child(group, row);
        let c = tree.alloc_cell(cell(label));
        tree.add_child(row, c);
    }
    if compute_table_layout(&mut tree, root, 40.0, None).is_ok() {
        println!("{}", render_tree(&tree, root));
    }

    // 10. Nested table (test 23): the outer host column's width flows down as
    //     the inner table's available width. The real engine recurses through
    //     `layout_cell`; here the inner table is rendered separately at the
    //     width the outer layout assigned to its host column.
    println!("=== nested table: inner table laid out at the host column's width ===");
    let (mut outer, outer_root) = MockTable::new(60.0)
        .body_row(vec![cell("nested table below"), cell("plain cell")])
        .into_tree();
    if compute_table_layout(&mut outer, outer_root, 60.0, None).is_ok() {
        println!("{}", render_tree(&outer, outer_root));
        let host = outer.nodes_with_role(TableRole::Cell)[0];
        if let Some(layout) = outer.layout(host) {
            let host_w = layout.size.width;
            println!("inner table, rendered at the host column's {host_w}px:");
            let out = MockTable::new(host_w)
                .body_row(vec![cell("in"), cell("ner")])
                .body_row(vec![cell("ta"), cell("ble")])
                .render();
            println!("{out}");
        }
    }
}
