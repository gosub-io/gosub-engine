//! Cell widths and what the boxes inside a cell resolve against (CSS 2 §17.5.2).

use super::overflow_clip::lay_out;
use crate::common::geo::Rect;
use crate::layering::layer::LayerList;
use crate::layouter::LayoutElementId;

fn border_box(layers: &LayerList, id: LayoutElementId) -> Rect {
    layers
        .layout_tree
        .get_node_by_id(id)
        .expect("laid out")
        .box_model
        .border_box
}

/// GOV.UK's search button: a `width: 1%` cell beside a greedy one. The cell is never narrower
/// than its content (CSS 2 §17.5.2.2), and the fixed-width child keeps its own 50px.
#[test]
fn fixed_width_child_of_a_percentage_cell_keeps_its_width() {
    let (layers, [table, cell, child]) = lay_out(
        r#"<html><body style="margin: 0">
          <div style="width: 630px">
            <div id="t" style="display: table; width: 100%">
              <div style="display: table-cell"><input style="width: 100%; height: 50px; border: 0; box-sizing: border-box"></div>
              <div id="c" style="display: table-cell; width: 1%">
                <span id="s" style="display: inline-block; width: 50px; height: 50px; background: #d2e2f1"></span>
              </div>
            </div>
          </div>
        </body></html>"#,
        ["t", "c", "s"],
    );
    let (table, cell, child) = (
        border_box(&layers, table),
        border_box(&layers, cell),
        border_box(&layers, child),
    );
    assert_eq!(table.width, 630.0);
    assert_eq!((child.width, child.height), (50.0, 50.0), "child {child:?}");
    assert_eq!(cell.width, 50.0, "cell {cell:?}");
    assert_eq!(cell.x + cell.width, 630.0, "cell {cell:?}");
    assert_eq!(child.x, cell.x, "child {child:?} cell {cell:?}");
}

/// The same with GOV.UK's own styles on top: a `<button>` that hides its label with
/// `overflow: hidden` and a large `text-indent`, in `position: relative` cells.
#[test]
fn hidden_label_button_in_a_percentage_cell_keeps_its_width() {
    let (layers, [cell, button]) = lay_out(
        r#"<html><body style="margin: 0">
          <div style="width: 630px">
            <div style="display: table; width: 100%">
              <div style="display: table-cell; position: relative"><input style="width: 100%; height: 50px; border: 0; box-sizing: border-box"></div>
              <div id="c" style="display: table-cell; width: 1%; position: relative">
                <button id="b" style="width: 50px; height: 50px; border: 0; padding: 0; margin: 0; overflow: hidden; text-indent: -5000px; position: relative">Search</button>
              </div>
            </div>
          </div>
        </body></html>"#,
        ["c", "b"],
    );
    let (cell, button) = (border_box(&layers, cell), border_box(&layers, button));
    assert_eq!((button.width, button.height), (50.0, 50.0), "button {button:?}");
    assert_eq!(cell.width, 50.0, "cell {cell:?}");
    assert_eq!(cell.x + cell.width, 630.0, "cell {cell:?}");
    assert_eq!(button.x, cell.x, "button {button:?} cell {cell:?}");
}

/// Captions are measured through the same re-layout as cells, but their own width is not
/// lattice's to settle: a block child of a `width: 40px` caption is 40px wide.
#[test]
fn caption_width_still_limits_its_content() {
    let (layers, [child]) = lay_out(
        r#"<html><body style="margin: 0">
          <table style="width: 200px; border-spacing: 0">
            <caption style="width: 40px; padding: 0"><div id="d" style="width: 100%; height: 10px"></div></caption>
            <tr><td style="padding: 0">x</td></tr>
          </table>
        </body></html>"#,
        ["d"],
    );
    let child = border_box(&layers, child);
    assert_eq!(child.width, 40.0, "child {child:?}");
}

fn content_box(layers: &LayerList, id: LayoutElementId) -> Rect {
    layers
        .layout_tree
        .get_node_by_id(id)
        .expect("laid out")
        .box_model
        .content_box
}

/// The cell's width from lattice is a border box; a content-box cell's padding and border sit
/// inside it, so a `width: 100%` child fills the content box and no more.
#[test]
fn padded_cell_child_fills_the_content_box() {
    let (layers, [cell, child]) = lay_out(
        r#"<html><body style="margin: 0">
          <table style="width: 200px; border-spacing: 0">
            <tr><td id="c" style="padding: 10px; border: 5px solid black"><div id="d" style="width: 100%; height: 10px"></div></td></tr>
          </table>
        </body></html>"#,
        ["c", "d"],
    );
    let (cell, inner, child) = (
        border_box(&layers, cell),
        content_box(&layers, cell),
        border_box(&layers, child),
    );
    assert_eq!(cell.width, 200.0, "cell {cell:?}");
    assert_eq!(inner.width, 170.0, "content {inner:?}");
    assert_eq!(
        (child.x, child.width),
        (inner.x, inner.width),
        "child {child:?} content {inner:?}"
    );
}

/// Percentage padding on a content-box cell: what is left between the paddings is the
/// child's, and it does not run past the cell's right edge.
#[test]
fn percentage_padding_cell_child_stays_inside_the_cell() {
    let (layers, [cell, child]) = lay_out(
        r#"<html><body style="margin: 0">
          <table style="width: 100px; table-layout: fixed; border-spacing: 0">
            <tr><td id="c" style="padding: 0 50% 0 10%; border: 0"><div id="d" style="width: 100%; height: 10px"></div></td></tr>
          </table>
        </body></html>"#,
        ["c", "d"],
    );
    let (cell, child) = (border_box(&layers, cell), border_box(&layers, child));
    assert_eq!(cell.width, 100.0, "cell {cell:?}");
    assert_eq!((child.x, child.width), (10.0, 40.0), "child {child:?}");
}

/// Collapsed borders: the cell keeps half of each boundary, and its content is what is left.
#[test]
fn collapsed_cell_child_fills_the_content_box() {
    let (layers, [cell, child]) = lay_out(
        r#"<html><body style="margin: 0">
          <table style="width: 200px; border-collapse: collapse">
            <tr><td id="c" style="padding: 0; border: 10px solid black"><div id="d" style="width: 100%; height: 10px"></div></td></tr>
          </table>
        </body></html>"#,
        ["c", "d"],
    );
    let (inner, child) = (content_box(&layers, cell), border_box(&layers, child));
    assert_eq!(
        (child.x, child.width),
        (inner.x, inner.width),
        "child {child:?} content {inner:?}"
    );
}

/// Pinning the width must leave the height alone: a content-box cell's `height` is still its
/// content height, so a `height: 100%` child is that tall.
#[test]
fn pinned_width_keeps_a_content_box_height() {
    let (layers, [cell, child]) = lay_out(
        r#"<html><body style="margin: 0">
          <table style="width: 200px; border-spacing: 0">
            <tr><td id="c" style="height: 100px; padding: 10px"><div id="d" style="height: 100%"></div></td></tr>
          </table>
        </body></html>"#,
        ["c", "d"],
    );
    let (inner, child) = (content_box(&layers, cell), border_box(&layers, child));
    assert_eq!(child.height, 100.0, "child {child:?} content {inner:?}");
}

/// The same for a `border-box` cell: its padding and border stay inside the width lattice gave it.
#[test]
fn border_box_cell_child_fills_the_content_box() {
    let (layers, [cell, child]) = lay_out(
        r#"<html><body style="margin: 0">
          <table style="width: 200px; border-spacing: 0">
            <tr><td id="c" style="box-sizing: border-box; height: 100px; padding: 10px; border: 5px solid black"><div id="d" style="width: 100%; height: 100%"></div></td></tr>
          </table>
        </body></html>"#,
        ["c", "d"],
    );
    let (cell, inner, child) = (
        border_box(&layers, cell),
        content_box(&layers, cell),
        border_box(&layers, child),
    );
    assert_eq!((cell.width, cell.height), (200.0, 100.0), "cell {cell:?}");
    assert_eq!(
        (child.x, child.width, child.height),
        (inner.x, inner.width, 70.0),
        "child {child:?} content {inner:?}"
    );
}
