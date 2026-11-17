//! Cell sizing: min-content under an explicit width, a cell's `height` as a minimum, percentage
//! heights under a cell (css-tables-3 §3.10.6), a table's `overflow` and its captions, and
//! `empty-cells: hide` (§5.2).

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

fn content_box(layers: &LayerList, id: LayoutElementId) -> Rect {
    layers
        .layout_tree
        .get_node_by_id(id)
        .expect("laid out")
        .box_model
        .content_box
}

/// A box with an explicit width is as wide as that in the cell's min-content, whatever it holds,
/// `width: 0` included.
#[test]
fn an_explicit_width_is_the_min_content_of_what_it_holds() {
    let (layers, [zero, fifty]) = lay_out(
        r#"<html><body>
          <table style="border-spacing: 0"><tr>
            <td id="zero" style="padding: 0"><div style="width: 0"><div style="width: 100px; height: 10px"></div></div></td>
          </tr></table>
          <table style="border-spacing: 0"><tr>
            <td id="fifty" style="padding: 0"><div style="width: 50px"><div style="width: 500px; height: 10px"></div></div></td>
          </tr></table>
        </body></html>"#,
        ["zero", "fifty"],
    );
    assert_eq!(border_box(&layers, zero).width, 0.0);
    assert_eq!(border_box(&layers, fifty).width, 50.0);
}

/// A table's `overflow` clips its grid, not its captions.
#[test]
fn table_overflow_does_not_clip_its_caption() {
    let (layers, [caption, inner]) = lay_out(
        r#"<html><body>
          <table style="overflow: hidden; border-spacing: 0">
            <caption id="caption" style="caption-side: bottom">caption</caption>
            <tr><td style="padding: 0"><div style="width: 50px; height: 50px">
              <div id="inner" style="width: 500px; height: 500px"></div>
            </div></td></tr>
          </table>
        </body></html>"#,
        ["caption", "inner"],
    );
    assert_eq!(layers.clip(caption), None);
    assert!(layers.clip(inner).is_some(), "the cells are clipped");
}

/// A generated caption gets no anonymous table, so it sits in the block, and the block's
/// `overflow` clips it.
#[test]
fn a_caption_outside_a_table_is_clipped() {
    let (layers, [block]) = lay_out(
        r#"<html><head><style>
          #block::before { content: "x"; display: table-caption; height: 100px }
        </style></head><body>
          <div id="block" style="overflow: hidden; height: 10px"></div>
        </body></html>"#,
        ["block"],
    );
    let before = layers.layout_tree.get_node_by_id(block).expect("laid out").children[0];
    assert!(layers.clip(before).is_some());
}

/// A cell's `height` is a minimum: the row grows to the content, with or without `overflow`, and
/// `max-height` does not cap it.
#[test]
fn a_cell_height_is_a_minimum() {
    let (layers, [hidden, capped, short]) = lay_out(
        r#"<html><body>
          <table><tr><td id="hidden" style="height: 20px; overflow: hidden; padding: 0">
            <div style="height: 300px"></div>
          </td></tr></table>
          <table><tr><td id="capped" style="height: 20px; max-height: 20px; padding: 0">
            <div style="height: 300px"></div>
          </td></tr></table>
          <table><tr><td id="short" style="height: 40px; padding: 0">
            <div style="height: 10px"></div>
          </td></tr></table>
        </body></html>"#,
        ["hidden", "capped", "short"],
    );
    assert_eq!(border_box(&layers, hidden).height, 300.0);
    assert_eq!(border_box(&layers, capped).height, 300.0);
    assert_eq!(border_box(&layers, short).height, 40.0);
}

/// With a table height, a percentage height under a cell resolves against the cell's final
/// height, and the cell's own length height is what the percentage resolves against when the
/// row is sized.
#[test]
fn percentage_heights_resolve_against_the_cell() {
    let (layers, [cell, child, scroller_cell, scroller]) = lay_out(
        r#"<html><body>
          <div style="display: table; width: 150px; height: 100px">
            <div id="cell" style="display: table-cell; padding: 5px 0 10px">
              <div id="child" style="height: 100%"></div>
            </div>
          </div>
          <div id="scroller-cell" style="display: table-cell; width: 100px; height: 100px">
            <div id="scroller" style="overflow: auto; height: 100%"><div style="height: 200px"></div></div>
          </div>
        </body></html>"#,
        ["cell", "child", "scroller-cell", "scroller"],
    );
    let cell_content = content_box(&layers, cell);
    assert_eq!(border_box(&layers, cell).height, 100.0);
    assert_eq!(border_box(&layers, child).height, cell_content.height);
    assert_eq!(border_box(&layers, child).y, cell_content.y);

    assert_eq!(border_box(&layers, scroller_cell).height, 100.0);
    assert_eq!(border_box(&layers, scroller).height, 100.0);
}

/// Measured for the row, a percentage height under a cell counts as auto, and as 0 for a
/// scroller; the second pass resolves it against the row's height.
#[test]
fn percentage_heights_count_as_auto_for_the_row() {
    let (layers, [visible, visible_child, scroller, scroller_child]) = lay_out(
        r#"<html><body>
          <table><tr><td id="visible" style="height: 20px; overflow: hidden; padding: 0">
            <div id="visible-child" style="height: 100%"><div style="height: 300px"></div></div>
          </td></tr></table>
          <table><tr><td id="scroller" style="height: 20px; padding: 0">
            <div id="scroller-child" style="height: 100%; overflow: auto"><div style="height: 300px"></div></div>
          </td></tr></table>
        </body></html>"#,
        ["visible", "visible-child", "scroller", "scroller-child"],
    );
    assert_eq!(border_box(&layers, visible).height, 300.0);
    assert_eq!(border_box(&layers, visible_child).height, 300.0);
    assert_eq!(border_box(&layers, scroller).height, 20.0);
    assert_eq!(border_box(&layers, scroller_child).height, 20.0);
}

/// A percentage height on a line resolves against the cell's final height, not its `height`.
#[test]
fn a_percentage_height_on_a_line_resolves_against_the_row() {
    let (layers, [cell, item]) = lay_out(
        r#"<html><body>
          <table style="border-spacing: 0"><tr>
            <td id="cell" style="height: 20px; padding: 0"><span id="item" style="display: inline-block; width: 10px; height: 100%"></span></td>
            <td style="padding: 0"><div style="height: 100px"></div></td>
          </tr></table>
        </body></html>"#,
        ["cell", "item"],
    );
    assert_eq!(border_box(&layers, cell).height, 100.0);
    assert_eq!(border_box(&layers, item).height, 100.0);
}

/// A blank line from a `<br>` takes up room in the cell when it is aligned.
#[test]
fn a_blank_line_counts_when_a_cell_is_aligned() {
    let (layers, [one, two]) = lay_out(
        r#"<html><body>
          <table style="height: 100px; line-height: 10px"><tr>
            <td id="one" style="vertical-align: bottom; padding: 0"><div style="height: 0%"></div>x</td>
            <td id="two" style="vertical-align: bottom; padding: 0"><div style="height: 0%"></div>x<br><br></td>
          </tr></table>
        </body></html>"#,
        ["one", "two"],
    );
    let text_y = |cell| {
        let cell = layers.layout_tree.get_node_by_id(cell).expect("laid out");
        let text = cell.children[1];
        border_box(&layers, text).y
    };
    assert_eq!(text_y(one) - text_y(two), 10.0);
}

/// An absolutely positioned child takes up no room in the cell when it is aligned.
#[test]
fn an_absolutely_positioned_child_does_not_count_when_a_cell_is_aligned() {
    let (layers, [cell]) = lay_out(
        r#"<html><body>
          <table style="height: 100px; line-height: 10px; border-spacing: 0"><tr>
            <td id="cell" style="position: relative; vertical-align: bottom; padding: 0"><div style="height: 0%"></div>x<div style="position: absolute; top: 0; height: 300px"></div></td>
          </tr></table>
        </body></html>"#,
        ["cell"],
    );
    let el = layers.layout_tree.get_node_by_id(cell).expect("laid out");
    let text = border_box(&layers, el.children[1]);
    assert_eq!(
        text.y + 10.0,
        el.box_model.border_box.y + el.box_model.border_box.height
    );
}

/// A percentage table height counts as specified even where it behaves as auto (§3.10.6), so a
/// percentage child resolves against the cell's height from the first pass.
#[test]
fn a_percentage_table_height_makes_the_cell_height_specified() {
    let (layers, [cell, child]) = lay_out(
        r#"<html><body>
          <div style="display: table; height: 100%; width: 100px">
            <div id="cell" style="display: table-cell">
              <div id="child" style="height: 50%"><div style="height: 100px"></div></div>
            </div>
          </div>
        </body></html>"#,
        ["cell", "child"],
    );
    assert_eq!(border_box(&layers, cell).height, 100.0);
    assert_eq!(border_box(&layers, child).height, 50.0);
}

/// `empty-cells: hide` drops the background and borders of an empty cell in the separated
/// borders model. Whitespace that collapses away and an absolutely positioned child leave a cell
/// empty; under `border-collapse: collapse` the property does nothing.
#[test]
fn empty_cells_hide_paints_nothing_for_an_empty_cell() {
    let (layers, [empty, spaces, positioned, filled, collapsed]) = lay_out(
        r#"<html><body>
          <table style="empty-cells: hide"><tr>
            <td id="empty" style="background: red; border: 2px solid red; width: 10px; height: 10px"></td>
            <td id="spaces" style="background: red; width: 10px; height: 10px">  </td>
            <td id="positioned" style="background: red; width: 10px; height: 10px"><span style="position: absolute"></span></td>
            <td id="filled" style="background: green; width: 10px; height: 10px">x</td>
          </tr></table>
          <table style="empty-cells: hide; border-collapse: collapse"><tr>
            <td id="collapsed" style="background: green; width: 10px; height: 10px"></td>
          </tr></table>
        </body></html>"#,
        ["empty", "spaces", "positioned", "filled", "collapsed"],
    );
    let painter = Painter::new(Arc::clone(&layers), None);
    let paints_box = |id| {
        painter
            .paint_element(id, &state())
            .iter()
            .any(|command| matches!(command, PaintCommand::Rectangle(_)))
    };
    assert!(!paints_box(empty));
    assert!(!paints_box(spaces));
    assert!(!paints_box(positioned));
    assert!(paints_box(filled));
    assert!(paints_box(collapsed));
}
