//! Cell sizing: min-content under an explicit width, a cell's `height` as a minimum, and a
//! table's `overflow` and its captions.

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
