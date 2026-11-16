//! Cell sizing: min-content under an explicit width, and a table's `overflow` and its captions.

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
