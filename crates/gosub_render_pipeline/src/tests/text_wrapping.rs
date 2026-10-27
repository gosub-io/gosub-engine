//! Breaking inside words (`overflow-wrap`, `word-break`), lines that may not wrap, and
//! `text-overflow: ellipsis`, laid out and painted through Parley.

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
use crate::layouter::{CanLayout as _, ElementContext, LayoutTree};
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

/// A long word with no break opportunity in it, far wider than the boxes it is put in.
const LONG_WORD: &str = "Supercalifragilisticexpialidocious";

/// `html` laid out in an 800x600 viewport, the font system it was measured with, and the node of
/// each `id` asked for.
fn lay_out<const N: usize>(html: &str, ids: [&str; N]) -> (LayoutTree, Arc<Mutex<dyn FontSystem>>, [NodeId; N]) {
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
    let mut layouter =
        TaffyLayouter::with_font_system_and_media_store(Arc::clone(&font_system), Arc::new(MediaStore::new()));
    let tree = layouter.layout(render_tree, Some(Dimension::new(800.0, 600.0)), 1.0);
    (tree, font_system, nodes)
}

fn border_box(tree: &LayoutTree, node: NodeId) -> Rect {
    tree.arena
        .values()
        .find(|el| el.dom_node_id == node)
        .expect("the element is laid out")
        .box_model
        .border_box
}

/// Whether `node` is `ancestor` or inside it.
fn within(tree: &LayoutTree, mut node: NodeId, ancestor: NodeId) -> bool {
    loop {
        if node == ancestor {
            return true;
        }
        match tree.render_tree.doc.parent(node) {
            Some(parent) => node = parent,
            None => return false,
        }
    }
}

/// The border boxes of every piece of text inside `node`, in reading order.
fn text_boxes(tree: &LayoutTree, node: NodeId) -> Vec<Rect> {
    let mut boxes: Vec<Rect> = tree
        .arena
        .values()
        .filter(|el| matches!(el.context, ElementContext::Text(_)) && within(tree, el.dom_node_id, node))
        .map(|el| el.box_model.border_box)
        .collect();
    // The arena is a map, so its own order is not reading order.
    boxes.sort_by(|a, b| a.y.total_cmp(&b.y).then(a.x.total_cmp(&b.x)));
    boxes
}

/// The texts painted for the text inside the element with this `id`, in reading order.
fn painted_texts(html: &str, id: &str) -> Vec<String> {
    let (tree, font_system, [node]) = lay_out(html, [id]);
    let mut pieces: Vec<_> = tree
        .arena
        .iter()
        .filter(|(_, el)| matches!(el.context, ElementContext::Text(_)) && within(&tree, el.dom_node_id, node))
        .map(|(id, el)| (*id, el.box_model.border_box))
        .collect();
    pieces.sort_by(|(_, a), (_, b)| a.y.total_cmp(&b.y).then(a.x.total_cmp(&b.x)));

    let layer_list = Arc::new(LayerList::new(Arc::new(tree)));
    let state = BrowserState {
        visible_layer_list: vec![true; layer_list.layer_ids.read().len()],
        wireframed: WireframeState::None,
        debug_hover: false,
        current_hovered_element: None,
        show_tilegrid: false,
        debug_table_cells: false,
        viewport: Rect::new(0.0, 0.0, 800.0, 600.0),
        tile_list: None,
        dpi_scale_factor: 1.0,
    };
    let painter = Painter::new(layer_list, Some(font_system));
    pieces
        .into_iter()
        .flat_map(|(id, _)| painter.paint_element(id, &state))
        .filter_map(|command| match command {
            PaintCommand::Text(text) => Some(text.text),
            _ => None,
        })
        .collect()
}

#[test]
fn a_long_word_overflows_its_box_by_default() {
    let (tree, _, [block]) = lay_out(
        &format!(r#"<body style="margin: 0"><div id="block" style="width: 100px">{LONG_WORD}</div></body>"#),
        ["block"],
    );
    let text = text_boxes(&tree, block);
    assert_eq!(text.len(), 1);
    assert!(text[0].width > 150.0, "the word is not broken: {:?}", text[0]);
}

#[test]
fn a_long_word_breaks_where_the_style_allows_it() {
    for style in [
        "overflow-wrap: break-word",
        "overflow-wrap: anywhere",
        "word-wrap: break-word",
        "word-break: break-word",
        "word-break: break-all",
    ] {
        let (tree, _, [node]) = lay_out(
            &format!(
                r#"<body style="margin: 0"><div id="block" style="width: 100px; {style}">{LONG_WORD}</div></body>"#
            ),
            ["block"],
        );
        let block = border_box(&tree, node);
        let text = text_boxes(&tree, node);
        for piece in &text {
            assert!(
                piece.x + piece.width <= block.x + block.width + 0.5,
                "{style}: the text stays in its box: {piece:?} in {block:?}"
            );
        }
        // Three lines at least, at the default 16px.
        let height: f64 = text.iter().map(|piece| piece.height).sum();
        assert!(height > 40.0, "{style}: the word takes several lines: {text:?}");
    }
}

/// `break-word` breaks a word only to keep it inside its line; the word still counts whole
/// towards the min-content width. `anywhere` and `break-all` breaks count.
#[test]
fn only_anywhere_and_break_all_shrink_the_min_content_width() {
    let (tree, _, [normal, break_word, anywhere, break_all]) = lay_out(
        &format!(
            r#"<table style="width: 1px; border-spacing: 0"><tr>
                <td id="normal" style="padding: 0">{LONG_WORD}</td>
                <td id="break-word" style="padding: 0; overflow-wrap: break-word">{LONG_WORD}</td>
                <td id="anywhere" style="padding: 0; overflow-wrap: anywhere">{LONG_WORD}</td>
                <td id="break-all" style="padding: 0; word-break: break-all">{LONG_WORD}</td>
            </tr></table>"#
        ),
        ["normal", "break-word", "anywhere", "break-all"],
    );
    let width = |node| border_box(&tree, node).width;
    assert!(width(normal) > 150.0, "normal: {}", width(normal));
    assert_eq!(width(break_word), width(normal), "break-word keeps the word whole");
    assert!(width(anywhere) < 30.0, "anywhere is a letter wide: {}", width(anywhere));
    assert!(
        width(break_all) < 30.0,
        "break-all is a letter wide: {}",
        width(break_all)
    );
}

/// Text that is itself a flex item is not in a line box: its automatic minimum is its min-content
/// width, so under `break-word` it keeps the word whole and overflows, as in other browsers.
#[test]
fn a_flex_item_keeps_its_min_content_width() {
    let (tree, _, [break_word, anywhere]) = lay_out(
        &format!(
            r#"<body style="margin: 0">
                <div id="break-word" style="display: flex; width: 100px; overflow-wrap: break-word">{LONG_WORD}</div>
                <div id="anywhere" style="display: flex; width: 100px; overflow-wrap: anywhere">{LONG_WORD}</div>
            </body>"#
        ),
        ["break-word", "anywhere"],
    );
    let widest = |node| {
        text_boxes(&tree, node)
            .iter()
            .map(|piece| piece.width)
            .fold(0.0, f64::max)
    };
    assert!(widest(break_word) > 150.0, "break-word: {}", widest(break_word));
    assert!(widest(anywhere) <= 100.5, "anywhere: {}", widest(anywhere));
}

/// `nowrap` keeps a block's line together even where it is made of several inline boxes.
#[test]
fn a_nowrap_block_keeps_mixed_inline_content_on_one_line() {
    let (tree, _, [wraps, nowrap]) = lay_out(
        r#"<body style="margin: 0">
            <div id="wraps" style="width: 100px">With <b>bold part</b> and <i>italic part</i> after</div>
            <div id="nowrap" style="width: 100px; white-space: nowrap">With <b>bold part</b> and <i>italic part</i> after</div>
        </body>"#,
        ["wraps", "nowrap"],
    );
    let rows = |node| {
        let mut tops: Vec<i64> = text_boxes(&tree, node)
            .iter()
            .map(|piece| piece.y.round() as i64)
            .collect();
        tops.dedup();
        tops.len()
    };
    assert!(rows(wraps) > 1, "without nowrap the line wraps");
    assert_eq!(rows(nowrap), 1, "with nowrap it does not");
}

#[test]
fn ellipsis_cuts_the_text_at_the_end_edge() {
    let ellipsis = "white-space: nowrap; overflow: hidden; text-overflow: ellipsis";
    let html = |style: &str, text: &str| {
        format!(r#"<body style="margin: 0"><div id="block" style="width: 120px; {style}">{text}</div></body>"#)
    };
    let long = "A rather long line of text that gets cut off";

    let painted = painted_texts(&html(ellipsis, long), "block");
    assert_eq!(painted.len(), 1);
    assert!(painted[0].ends_with('\u{2026}'), "the cut is marked: {painted:?}");
    assert!(long.starts_with(painted[0].trim_end_matches('\u{2026}')));
    assert!(painted[0].chars().count() < long.chars().count());

    // Text that fits is left alone.
    assert_eq!(painted_texts(&html(ellipsis, "Short"), "block"), ["Short"]);
    // `clip`, and an ellipsis on a box whose overflow shows, change nothing.
    let clip = "white-space: nowrap; overflow: hidden; text-overflow: clip";
    assert_eq!(painted_texts(&html(clip, long), "block"), [long]);
    let visible = "white-space: nowrap; text-overflow: ellipsis";
    assert_eq!(painted_texts(&html(visible, long), "block"), [long]);
}

/// Across several inline boxes, the run at the edge is cut and those past it are not painted.
#[test]
fn ellipsis_cuts_a_line_of_several_inline_boxes() {
    let painted = painted_texts(
        r#"<body style="margin: 0"><div id="block" style="width: 120px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis">With <b>bold part</b> and <i>italic part</i> after</div></body>"#,
        "block",
    );
    let joined = painted.concat();
    assert!(
        joined.ends_with('\u{2026}'),
        "the line ends in the ellipsis: {painted:?}"
    );
    assert!(
        !joined.contains("after"),
        "the runs past the edge are gone: {painted:?}"
    );
    assert!(joined.starts_with("With"), "{painted:?}");
}

/// A run that wrapped is left whole even where one word in it overflows: the ellipsis marks a
/// line cut at the edge, and cutting the run there would drop its other lines.
#[test]
fn ellipsis_leaves_a_wrapped_run_whole() {
    let text = format!("a few words then {LONG_WORD} and more after");
    let painted = painted_texts(
        &format!(
            r#"<body style="margin: 0"><div id="block" style="width: 100px; overflow: hidden; text-overflow: ellipsis">{text}</div></body>"#
        ),
        "block",
    );
    assert_eq!(painted, [text]);
}

/// A long word among inline boxes is a box of its own; it breaks like a word in a run of plain
/// text does.
#[test]
fn a_long_word_breaks_among_inline_boxes() {
    let (tree, _, [node]) = lay_out(
        &format!(
            r#"<body style="margin: 0"><div id="block" style="width: 100px; overflow-wrap: break-word">See <b>this</b> {LONG_WORD} now</div></body>"#
        ),
        ["block"],
    );
    let block = border_box(&tree, node);
    for piece in text_boxes(&tree, node) {
        assert!(
            piece.x + piece.width <= block.x + block.width + 0.5,
            "the text stays in its box: {piece:?} in {block:?}"
        );
    }
}

/// A `nowrap` line is not split between the bands beside a float and below it: too wide for the
/// room beside the float, the whole line goes below it.
#[test]
fn a_nowrap_line_beside_a_float_stays_whole() {
    let (tree, _, [block]) = lay_out(
        r#"<body style="margin: 0"><div id="block" style="width: 300px; white-space: nowrap"><div style="float: left; width: 150px; height: 60px"></div>With <b>bold part</b> and <i>italic part</i> after that and more</div></body>"#,
        ["block"],
    );
    let text = text_boxes(&tree, block);
    assert!(text.len() > 1, "{text:?}");
    for piece in &text {
        assert_eq!(piece.y, text[0].y, "one line: {text:?}");
    }
    assert!(text[0].y >= 60.0, "below the float: {:?}", text[0]);
}

/// Text items that touch are kept on one line together, but one of them that may break inside
/// its words still breaks: the parentheses around a long link stay whole beside it.
#[test]
fn a_long_word_breaks_beside_the_text_it_touches() {
    let (tree, _, [block]) = lay_out(
        &format!(
            r#"<body style="margin: 0"><div id="block" style="width: 100px; overflow-wrap: anywhere">(<a>{LONG_WORD}</a>)</div></body>"#
        ),
        ["block"],
    );
    let block_box = border_box(&tree, block);
    let text = text_boxes(&tree, block);
    assert_eq!(text.len(), 3, "{text:?}");
    for piece in &text {
        assert!(
            piece.x + piece.width <= block_box.x + block_box.width + 0.5,
            "the text stays in its box: {piece:?} in {block_box:?}"
        );
    }
    assert!(text[1].height > 40.0, "the word takes several lines: {:?}", text[1]);
    // A parenthesis is one grapheme: it cannot be squeezed.
    assert_eq!(text[0].width, text[2].width, "{text:?}");
    assert!(text[0].width > 4.0, "{:?}", text[0]);
}

/// A `nowrap` line holding an inline-block cannot be measured before layout; it stays beside the
/// float rather than being sent below it.
#[test]
fn a_nowrap_line_with_an_inline_block_stays_beside_a_float() {
    let (tree, _, [block]) = lay_out(
        r#"<body style="margin: 0"><div id="block" style="width: 300px; white-space: nowrap"><div style="float: left; width: 150px; height: 60px"></div>Go <span style="display: inline-block; width: 20px; height: 10px"></span> on</div></body>"#,
        ["block"],
    );
    let text = text_boxes(&tree, block);
    assert!(!text.is_empty());
    for piece in &text {
        assert!(piece.y < 60.0, "beside the float: {text:?}");
        assert!(piece.x >= 150.0, "beside the float: {text:?}");
    }
}

/// The same line with text too wide for the room beside the float: the text that can be
/// measured is already too wide, so the line goes below the float.
#[test]
fn a_nowrap_line_too_wide_beside_a_float_goes_below_it_with_an_inline_block() {
    let (tree, _, [block]) = lay_out(
        r#"<body style="margin: 0"><div id="block" style="width: 300px; white-space: nowrap"><div style="float: left; width: 250px; height: 60px"></div>Go on and on and on <span style="display: inline-block; width: 20px; height: 10px"></span> on</div></body>"#,
        ["block"],
    );
    let text = text_boxes(&tree, block);
    assert!(!text.is_empty());
    for piece in &text {
        assert!(piece.y >= 60.0, "below the float: {text:?}");
    }
}
