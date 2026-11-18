//! An absolutely positioned box written inside a line resolves its insets and percentage sizes
//! against its containing block (CSS 2 §10.1, §10.5), not against the line box it sits in.

use crate::common::geo::Rect;
use crate::layering::layer::LayerList;
use crate::layouter::LayoutElementId;

use super::overflow_clip::lay_out;

fn border_box(layers: &LayerList, id: LayoutElementId) -> Rect {
    layers
        .layout_tree
        .get_node_by_id(id)
        .expect("laid out")
        .box_model
        .border_box
}

/// GOV.UK's search button: the icon covers the button, whatever the text in front of it does.
#[test]
fn an_icon_covers_the_button_it_is_written_in() {
    let (layers, [button, icon, span]) = lay_out(
        r#"<html><body style="margin: 0">
          <button id="button" style="position: relative; overflow: hidden; width: 40px; height: 40px;
            padding: 0; border: 0; text-indent: -5000px">Search
            <svg id="icon" style="position: absolute; top: 0; left: 0; width: 100%; height: 100%"
              width="27" height="27" viewBox="0 0 27 27"><rect width="27" height="27"/></svg>
            <span id="span" style="position: absolute; top: 0; left: 0; width: 100%; height: 100%"></span>
          </button>
        </body></html>"#,
        ["button", "icon", "span"],
    );
    let button = border_box(&layers, button);
    assert_eq!((button.width, button.height), (40.0, 40.0));
    assert_eq!(border_box(&layers, icon), button);
    assert_eq!(border_box(&layers, span), button);
}

#[test]
fn a_percentage_size_is_of_the_containing_block_not_the_line() {
    let (layers, [cb, with_text, alone]) = lay_out(
        r#"<html><body style="margin: 0">
          <div id="cb" style="position: relative; width: 100px; height: 40px">Search
            <span id="with-text" style="position: absolute; top: 0; left: 0; width: 100%; height: 100%"></span>
          </div>
          <div style="position: relative; width: 100px; height: 40px">
            <span id="alone" style="position: absolute; top: 0; left: 0; width: 100%; height: 50%"></span>
          </div>
        </body></html>"#,
        ["cb", "with-text", "alone"],
    );
    assert_eq!(border_box(&layers, with_text), border_box(&layers, cb));
    // No text: the line box is empty, and the percentage used to resolve to 0.
    let alone = border_box(&layers, alone);
    assert_eq!((alone.width, alone.height), (100.0, 20.0));
}

/// Opposing insets stretch the box between the containing block's edges, inside a plain inline too.
#[test]
fn opposing_insets_stretch_across_the_containing_block() {
    let (layers, [cb1, direct, cb2, nested]) = lay_out(
        r#"<html><body style="margin: 0">
          <div id="cb1" style="position: relative; width: 100px; height: 40px">Search
            <span id="direct" style="position: absolute; top: 5px; bottom: 5px; left: 0; right: 0"></span>
          </div>
          <div id="cb2" style="position: relative; width: 100px; height: 40px">Foo <a>bar
            <span id="nested" style="position: absolute; top: 5px; bottom: 5px; left: 0; right: 0"></span>
            baz</a> qux
          </div>
        </body></html>"#,
        ["cb1", "direct", "cb2", "nested"],
    );
    for (cb, id) in [(cb1, direct), (cb2, nested)] {
        let cb = border_box(&layers, cb);
        assert_eq!(border_box(&layers, id), Rect::new(cb.x, cb.y + 5.0, 100.0, 30.0));
    }
}

/// A block whose padding box is the containing block still holds the line in its content box:
/// the parent matching the containing block says nothing about the line.
#[test]
fn a_padded_parent_does_not_hide_the_narrower_line() {
    let (layers, [cb, abspos]) = lay_out(
        r#"<html><body style="margin: 0">
          <p id="cb" style="position: relative; margin: 0; padding: 3px; width: 94px; height: 34px">Foo <b>x</b>
            <span id="abspos" style="position: absolute; top: 0; left: 0; width: 100%; height: 100%"></span>
          </p>
        </body></html>"#,
        ["cb", "abspos"],
    );
    let padding_box = layers.layout_tree.get_node_by_id(cb).unwrap().box_model.padding_box;
    assert_eq!(border_box(&layers, abspos), padding_box);
}

/// Taffy has no `calc()`: the size is resolved against the containing block wherever the box sits,
/// also where the line, or the parent, spans the containing block exactly.
#[test]
fn a_calc_size_is_resolved_against_the_containing_block() {
    let (layers, [in_line, direct]) = lay_out(
        r#"<html><body style="margin: 0">
          <div style="position: relative; width: 200px; height: 40px">
            <div style="padding: 0 10px; margin: 0 -10px">Foo
              <span id="in-line" style="position: absolute; top: 0; left: 0; height: 10px;
                width: calc(50% + 10px)"></span>
            </div>
          </div>
          <div style="position: relative; width: 200px; height: 40px">
            <div id="direct" style="position: absolute; top: 0; left: 0; width: calc(50% + 10px);
              height: calc(50% - 4px)"></div>
          </div>
        </body></html>"#,
        ["in-line", "direct"],
    );
    let in_line = border_box(&layers, in_line);
    assert_eq!((in_line.width, in_line.height), (110.0, 10.0));
    let direct = border_box(&layers, direct);
    assert_eq!((direct.width, direct.height), (110.0, 16.0));
}
