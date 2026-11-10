//! What clips paint: `overflow` clips descendants to the padding box along the containing block
//! chain; `clip` and `clip-path: inset()` clip the box itself and everything under it.

use std::sync::Arc;

use gosub_css3::system::Css3System;
use gosub_fontmanager::ParleyFontSystem;
use gosub_html5::document::document_impl::DocumentImpl;
use gosub_html5::html_compile;
use gosub_html5::parser::Html5Parser;
use gosub_interface::config::ModuleConfiguration;
use gosub_interface::css3::CssSystem as _;
use gosub_interface::document::Document as _;
use gosub_interface::font_system::FontSystem;
use gosub_shared::node::NodeId;
use parking_lot::Mutex;

use crate::common::browser_state::{BrowserState, WireframeState};
use crate::common::document::pipeline_doc::GosubDocumentAdapter;
use crate::common::geo::{Dimension, Rect};
use crate::common::media::MediaStore;
use crate::layering::layer::LayerList;
use crate::layouter::taffy::TaffyLayouter;
use crate::layouter::{CanLayout as _, LayoutElementId};
use crate::painter::commands::PaintCommand;
use crate::painter::Painter;
use crate::rendertree_builder::tree::RenderTree;

#[derive(Clone, Debug, PartialEq)]
struct Config;

impl ModuleConfiguration for Config {
    type CssSystem = Css3System;
    type Document = DocumentImpl<Self>;
    type HtmlParser = Html5Parser<'static, Self>;
}

/// `html` laid out in an 800x600 viewport, as a layer list, with the layout element of each `id`.
fn lay_out<const N: usize>(html: &str, ids: [&str; N]) -> (Arc<LayerList>, [LayoutElementId; N]) {
    let mut doc = html_compile::<Config>(html);
    doc.add_stylesheet(Css3System::load_default_useragent_stylesheet());
    let adapter = GosubDocumentAdapter::<Config>::new(Arc::new(doc));
    let nodes = ids.map(|id| {
        fn find(a: &GosubDocumentAdapter<Config>, node: NodeId, id: &str) -> Option<NodeId> {
            if a.doc.attribute(node, "id") == Some(id) {
                return Some(node);
            }
            a.doc.children(node).iter().find_map(|&child| find(a, child, id))
        }
        find(&adapter, adapter.doc.root(), id).unwrap_or_else(|| panic!("no element with id {id}"))
    });
    let mut render_tree = RenderTree::new(Arc::new(adapter));
    render_tree.parse().expect("render tree");
    let font_system: Arc<Mutex<dyn FontSystem>> = Arc::new(Mutex::new(ParleyFontSystem::new()));
    let mut layouter = TaffyLayouter::with_font_system_and_media_store(font_system, Arc::new(MediaStore::new()));
    let tree = layouter.layout(render_tree, Some(Dimension::new(800.0, 600.0)), 1.0);
    let elements = nodes.map(|node| {
        tree.arena
            .values()
            .find(|el| el.dom_node_id == node)
            .expect("the element is laid out")
            .id
    });
    (Arc::new(LayerList::new(Arc::new(tree))), elements)
}

fn padding_box(layers: &LayerList, id: LayoutElementId) -> Rect {
    layers
        .layout_tree
        .get_node_by_id(id)
        .expect("laid out")
        .box_model
        .padding_box
}

fn state() -> BrowserState {
    BrowserState {
        visible_layer_list: vec![true; 8],
        wireframed: WireframeState::None,
        debug_hover: false,
        current_hovered_element: None,
        show_tilegrid: false,
        debug_table_cells: false,
        viewport: Rect::new(0.0, 0.0, 800.0, 600.0),
        tile_list: None,
        dpi_scale_factor: 1.0,
    }
}

/// The visually hidden pattern: a 1px box that hides its text. The text keeps its layout and
/// paints inside a 1x1 clip.
#[test]
fn visually_hidden_text_is_clipped_to_its_box() {
    let (layers, [sr, inner]) = lay_out(
        r#"<html><body><p>
          <span id="sr" style="position: absolute; width: 1px; height: 1px; overflow: hidden;
            white-space: nowrap"><span id="inner">Skip to content</span></span>
        </p></body></html>"#,
        ["sr", "inner"],
    );
    let clip = layers.clip(inner).expect("the text is clipped");
    assert_eq!(clip, padding_box(&layers, sr));
    assert_eq!((clip.width, clip.height), (1.0, 1.0));
    assert!(layers.clip(sr).is_none(), "a box is not clipped by its own overflow");

    let painter = Painter::new(Arc::clone(&layers), None);
    let words: Vec<_> = layers.layout_tree.get_node_by_id(inner).unwrap().children.clone();
    let commands: Vec<_> = words
        .iter()
        .flat_map(|&id| painter.paint_element(id, &state()))
        .collect();
    assert!(
        matches!(commands.first(), Some(PaintCommand::PushClip(rect)) if *rect == clip),
        "{commands:?}"
    );
    assert!(matches!(commands.last(), Some(PaintCommand::PopClip)));
}

/// An absolutely positioned box escapes a clipping box below its containing block, not one that
/// is (or is above) its containing block. A fixed box escapes both.
#[test]
fn positioned_boxes_follow_their_containing_block() {
    let (layers, [static_clip, escapes, positioned_clip, stays, fixed, in_flow]) = lay_out(
        r#"<html><body>
          <div id="static-clip" style="overflow: hidden; width: 100px; height: 100px">
            <div id="escapes" style="position: absolute; top: 300px; width: 50px; height: 50px"></div>
            <div id="in-flow" style="width: 500px; height: 20px"></div>
          </div>
          <div id="positioned-clip" style="position: relative; overflow: hidden; width: 100px; height: 100px">
            <div style="overflow: visible">
              <div id="stays" style="position: absolute; top: 300px; width: 50px; height: 50px"></div>
            </div>
            <div id="fixed" style="position: fixed; top: 0; width: 50px; height: 50px"></div>
          </div>
        </body></html>"#,
        ["static-clip", "escapes", "positioned-clip", "stays", "fixed", "in-flow"],
    );
    assert_eq!(layers.clip(in_flow), Some(padding_box(&layers, static_clip)));
    assert_eq!(layers.clip(escapes), None);
    assert_eq!(layers.clip(stays), Some(padding_box(&layers, positioned_clip)));
    assert_eq!(layers.clip(fixed), None);
}

/// Nested clips intersect, and a clip on one axis leaves the other open.
#[test]
fn nested_clips_intersect_per_axis() {
    let (layers, [outer, inner, leaf]) = lay_out(
        r#"<html><body>
          <div id="outer" style="overflow-x: clip; width: 100px; height: 40px">
            <div id="inner" style="overflow-y: clip; width: 200px; height: 20px">
              <div id="leaf" style="width: 400px; height: 400px"></div>
            </div>
          </div>
        </body></html>"#,
        ["outer", "inner", "leaf"],
    );
    let (o, i) = (padding_box(&layers, outer), padding_box(&layers, inner));
    let x_only = layers.clip(inner).expect("outer clips x");
    assert_eq!((x_only.x, x_only.width), (o.x, o.width));
    assert!(x_only.height > 1e8, "outer leaves y open: {x_only:?}");

    let both = layers.clip(leaf).expect("both clip");
    assert_eq!(both, Rect::new(o.x, i.y, o.width, i.height));
}

/// `overflow` on the body (with a visible root) belongs to the window: the body clips nothing.
/// With the root's own overflow set, the body clips like any other box.
#[test]
fn the_overflow_the_window_takes_clips_nothing() {
    let (layers, [child]) = lay_out(
        r#"<html><body style="overflow-x: hidden"><div id="child" style="width: 2000px"></div></body></html>"#,
        ["child"],
    );
    assert_eq!(layers.clip(child), None);

    let (layers, [body, child]) = lay_out(
        r#"<html style="overflow: hidden"><body id="body" style="overflow-x: hidden">
          <div id="child" style="width: 2000px"></div></body></html>"#,
        ["body", "child"],
    );
    let clip = layers.clip(child).expect("the body clips");
    let body_box = padding_box(&layers, body);
    assert_eq!((clip.x, clip.width), (body_box.x, body_box.width));
}

/// A point on a box, but outside what its clip shows, does not hit it.
#[test]
fn hit_testing_ignores_the_clipped_part() {
    let (layers, [clipper, wide]) = lay_out(
        r#"<html><body style="margin: 0">
          <div id="clipper" style="overflow: hidden; width: 100px; height: 100px">
            <div id="wide" style="width: 500px; height: 50px"></div>
          </div>
        </body></html>"#,
        ["clipper", "wide"],
    );
    assert_eq!(layers.find_element_at(10.0, 10.0, 0.0, 0.0), Some(wide));
    assert_ne!(layers.find_element_at(300.0, 10.0, 0.0, 0.0), Some(wide));
    assert_ne!(layers.find_element_at(300.0, 10.0, 0.0, 0.0), Some(clipper));
}

/// A box clipped to nothing paints nothing.
#[test]
fn a_box_clipped_to_nothing_paints_nothing() {
    let (layers, [hidden]) = lay_out(
        r#"<html><body>
          <div style="overflow: hidden; height: 0">
            <div id="hidden" style="height: 20px; background: red"></div>
          </div>
        </body></html>"#,
        ["hidden"],
    );
    let painter = Painter::new(Arc::clone(&layers), None);
    assert!(painter.paint_element(hidden, &state()).is_empty());
}

/// `clip: rect()` clips an absolutely positioned box and what it holds, from its border box's
/// top-left corner, `auto` being the border edge. On a box that is not absolutely positioned
/// it does nothing.
#[test]
fn clip_rect_clips_an_absolutely_positioned_box() {
    let (layers, [abs, inner, hidden, static_box]) = lay_out(
        r#"<html><body>
          <div id="abs" style="position: absolute; top: 10px; left: 10px; width: 100px; height: 50px;
            clip: rect(5px, auto, 20px, 30px)"><div id="inner" style="height: 200px"></div></div>
          <div id="hidden" style="position: absolute; width: 10px; height: 10px; clip: rect(0 0 0 0)"></div>
          <div id="static" style="width: 10px; height: 10px; clip: rect(0 0 0 0)"></div>
        </body></html>"#,
        ["abs", "inner", "hidden", "static"],
    );
    let b = layers.layout_tree.get_node_by_id(abs).unwrap().box_model.border_box;
    let expected = Rect::new(b.x + 30.0, b.y + 5.0, b.width - 30.0, 15.0);
    assert_eq!(layers.clip(abs), Some(expected), "the box itself");
    assert_eq!(layers.clip(inner), Some(expected), "and what it holds");

    let painter = Painter::new(Arc::clone(&layers), None);
    assert!(painter.paint_element(hidden, &state()).is_empty());
    assert_eq!(layers.clip(static_box), None);
}

/// `clip-path: inset()` takes one to four insets from the border box, a percentage of its width
/// or height, and clips fixed descendants too. `inset(50%)` leaves nothing.
#[test]
fn clip_path_inset_clips_the_box_and_all_under_it() {
    let (layers, [boxed, fixed, gone]) = lay_out(
        r#"<html><body>
          <div id="boxed" style="width: 200px; height: 100px; clip-path: inset(10px 10% round 4px)">
            <div id="fixed" style="position: fixed; top: 0; left: 0; width: 50px; height: 50px"></div>
          </div>
          <div id="gone" style="height: 20px; clip-path: inset(50%)"></div>
        </body></html>"#,
        ["boxed", "fixed", "gone"],
    );
    let b = layers.layout_tree.get_node_by_id(boxed).unwrap().box_model.border_box;
    let expected = Rect::new(b.x + 20.0, b.y + 10.0, 160.0, 80.0);
    assert_eq!(layers.clip(boxed), Some(expected));
    assert_eq!(layers.clip(fixed), Some(expected), "a fixed box does not escape it");

    let clip = layers.clip(gone).expect("clipped");
    assert_eq!((clip.width, clip.height), (0.0, 0.0));
    let painter = Painter::new(Arc::clone(&layers), None);
    assert!(painter.paint_element(gone, &state()).is_empty());
}
