/// Console table renderer — run with:
///   cargo run --bin table_console -p gosub_lattice
use gosub_lattice::mock::{cell, MockTable};

fn main() {
    // ------------------------------------------------------------------
    // 1. Simple 3-column table, no spanning
    // ------------------------------------------------------------------
    println!("=== Simple 3-column table ===");
    let out = MockTable::new(60.0)
        .header_row(vec![cell("Name"), cell("Age"), cell("City")])
        .body_row(vec![cell("Alice"), cell("30"), cell("Amsterdam")])
        .body_row(vec![cell("Bob"), cell("25"), cell("Berlin")])
        .body_row(vec![cell("Carol"), cell("42"), cell("Brussels")])
        .footer_row(vec![cell("3 people"), cell(""), cell("")])
        .render();
    println!("{out}");

    // ------------------------------------------------------------------
    // 2. colspan: Bob's name spans 2 columns
    // ------------------------------------------------------------------
    println!("=== colspan demo ===");
    let out = MockTable::new(60.0)
        .header_row(vec![cell("Name"), cell("Info")])
        .body_row(vec![cell("Alice"), cell("Amsterdam")])
        .body_row(vec![
            cell("Bob — long name that spans both columns").colspan(2),
        ])
        .body_row(vec![cell("Carol"), cell("Brussels")])
        .render();
    println!("{out}");

    // ------------------------------------------------------------------
    // 3. rowspan: Bob spans 2 body rows (clamped to the body section)
    // ------------------------------------------------------------------
    println!("=== rowspan demo (clamped to body section) ===");
    let out = MockTable::new(60.0)
        .header_row(vec![cell("Name"), cell("Score")])
        .body_row(vec![cell("Alice (rowspan=2)").rowspan(2), cell("100")])
        .body_row(vec![cell("80")]) // col 0 occupied by rowspan
        .body_row(vec![cell("Carol"), cell("90")])
        .footer_row(vec![cell("Total"), cell("270")])
        .render();
    println!("{out}");

    // ------------------------------------------------------------------
    // 4. rowspan that would cross thead→tbody boundary — must be clamped
    // ------------------------------------------------------------------
    println!("=== rowspan clamped at section boundary ===");
    let out = MockTable::new(60.0)
        .header_row(vec![
            cell("Header rowspan=99").rowspan(99), // only 1 row in thead → clamped to 1
            cell("H2"),
        ])
        .body_row(vec![cell("Body A"), cell("Body B")])
        .render();
    println!("{out}");

    // ------------------------------------------------------------------
    // 5. thead/tbody/tfoot all present, rowspan within tbody
    // ------------------------------------------------------------------
    println!("=== thead + rowspan in tbody + tfoot ===");
    let out = MockTable::new(60.0)
        .header_row(vec![cell("Col A"), cell("Col B"), cell("Col C")])
        .body_row(vec![
            cell("Spans 2 rows").rowspan(2),
            cell("B1"),
            cell("C1"),
        ])
        .body_row(vec![cell("B2"), cell("C2")]) // col 0 spanned
        .body_row(vec![cell("A3"), cell("B3"), cell("C3")])
        .footer_row(vec![cell("Footer").colspan(3)])
        .render();
    println!("{out}");
}
