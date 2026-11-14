//! Row, row group, column and column group backgrounds show only under the cells, positioned
//! against the part's own box (CSS 2.1 §17.5.1, css-tables-3 §5.3.2).

use std::sync::Arc;

use super::overflow_clip::{lay_out, state};
use crate::common::geo::Rect;
use crate::layering::layer::LayerList;
use crate::layouter::LayoutElementId;
use crate::painter::commands::PaintCommand;
use crate::painter::Painter;

fn border_box(layers: &LayerList, id: LayoutElementId) -> Rect {
    layers
        .layout_tree
        .get_node_by_id(id)
        .expect("laid out")
        .box_model
        .border_box
}

/// The areas of every background rectangle in `commands`, with the clip each is painted in.
fn backgrounds(commands: &[PaintCommand]) -> Vec<(Option<Rect>, Rect)> {
    let mut clip = None;
    let mut out = Vec::new();
    for command in commands {
        match command {
            PaintCommand::PushClip(rect) => clip = Some(*rect),
            PaintCommand::PopClip => clip = None,
            PaintCommand::Rectangle(r) if r.background().is_some() => out.push((clip, r.rect())),
            _ => {}
        }
    }
    out
}

#[test]
fn row_and_column_backgrounds_paint_under_each_cell() {
    let (layers, [row, a, b]) = lay_out(
        r#"<html><body>
          <table style="border-spacing: 4px">
            <colgroup><col style="background: blue"></colgroup>
            <tr id="row" style="background: red; border: 2px solid black">
              <td id="a" style="width: 40px; height: 20px; padding: 0"></td>
              <td id="b" style="width: 40px; height: 20px; padding: 0; background: green"></td>
            </tr>
          </table>
        </body></html>"#,
        ["row", "a", "b"],
    );
    let painter = Painter::new(Arc::clone(&layers), None);
    let (row_box, a_box, b_box) = (border_box(&layers, row), border_box(&layers, a), border_box(&layers, b));

    // The row spans its cells and the gap between them, not the gutters outside them.
    assert_eq!((row_box.x, row_box.width), (a_box.x, b_box.x + b_box.width - a_box.x));
    // Separated borders: the row paints neither its background nor its border itself.
    let row_commands = painter.paint_element(row, &state());
    assert!(backgrounds(&row_commands).is_empty(), "{row_commands:?}");
    assert!(
        !row_commands.iter().any(|c| matches!(c, PaintCommand::Rectangle(_))),
        "{row_commands:?}"
    );

    // Cell a: the column's colour, then the row's, each clipped to the cell, then its own.
    let a_paint = backgrounds(&painter.paint_element(a, &state()));
    assert_eq!(a_paint, vec![(Some(a_box), a_box), (Some(a_box), a_box), (None, a_box)]);

    // Cell b: no column of its own; the row's colour, then its own on top.
    let b_paint = backgrounds(&painter.paint_element(b, &state()));
    assert_eq!(b_paint, vec![(Some(b_box), b_box), (None, b_box)]);
}

/// A row's colour fills every cell that starts in it, all the way down a cell spanning rows; its
/// gradient is laid out over the row.
#[test]
fn a_row_spanning_cell_takes_its_first_rows_background() {
    let (layers, [first, tall]) = lay_out(
        r#"<html><body>
          <table style="border-spacing: 0">
            <tr id="first" style="background: red linear-gradient(blue, blue) no-repeat">
              <td id="tall" rowspan="2" style="width: 40px; padding: 0"></td>
              <td style="width: 40px; height: 20px; padding: 0"></td>
            </tr>
            <tr><td style="width: 40px; height: 20px; padding: 0"></td></tr>
          </table>
        </body></html>"#,
        ["first", "tall"],
    );
    let painter = Painter::new(Arc::clone(&layers), None);
    let (row_box, tall_box) = (border_box(&layers, first), border_box(&layers, tall));
    assert!(tall_box.height > row_box.height, "{tall_box:?} spans past {row_box:?}");

    let paint = backgrounds(&painter.paint_element(tall, &state()));
    assert_eq!(paint[0], (Some(tall_box), tall_box), "the colour fills the whole cell");
    assert_eq!(
        paint[1],
        (Some(tall_box), row_box),
        "the gradient is laid out over the row"
    );
}
