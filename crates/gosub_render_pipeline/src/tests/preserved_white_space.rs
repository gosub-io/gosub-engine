//! Preserved white space (`white-space: pre` and `pre-wrap`): segment breaks between inline
//! elements, spaces at the end of one, and tabs (css-text-3 §4.1).

use std::sync::Arc;

use cow_utils::CowUtils as _;
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

use crate::common::document::pipeline_doc::GosubDocumentAdapter;
use crate::common::geo::{Dimension, Rect};
use crate::common::media::MediaStore;
use crate::layouter::taffy::TaffyLayouter;
use crate::layouter::{CanLayout as _, ElementContext, LayoutTree};
use crate::rendertree_builder::tree::RenderTree;

#[derive(Clone, Debug, PartialEq)]
struct Config;

impl ModuleConfiguration for Config {
    type CssSystem = Css3System;
    type Document = DocumentImpl<Self>;
    type HtmlParser = Html5Parser<'static, Self>;
}

/// `html` laid out in an 800x600 viewport, and the node of each `id` asked for.
fn lay_out<const N: usize>(html: &str, ids: [&str; N]) -> (LayoutTree, [NodeId; N]) {
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
    (tree, nodes)
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

/// Every piece of text inside `node` with its border box, in reading order.
fn text_pieces(tree: &LayoutTree, node: NodeId) -> Vec<(String, Rect)> {
    let mut pieces: Vec<(String, Rect)> = tree
        .arena
        .values()
        .filter_map(|el| match &el.context {
            ElementContext::Text(text) if within(tree, el.dom_node_id, node) => {
                Some((text.text.clone(), el.box_model.border_box))
            }
            _ => None,
        })
        .collect();
    pieces.sort_by(|(_, a), (_, b)| a.y.total_cmp(&b.y).then(a.x.total_cmp(&b.x)));
    pieces
}

/// The pieces of text inside `node`, grouped into the lines they sit on.
fn lines(tree: &LayoutTree, node: NodeId) -> Vec<Vec<(String, Rect)>> {
    let mut lines: Vec<Vec<(String, Rect)>> = Vec::new();
    for piece in text_pieces(tree, node) {
        match lines.last_mut() {
            Some(line) if (line[0].1.y - piece.1.y).abs() < 1.0 => line.push(piece),
            _ => lines.push(vec![piece]),
        }
    }
    lines
}

/// The words of each line, as a line of text.
fn line_texts(tree: &LayoutTree, node: NodeId) -> Vec<String> {
    lines(tree, node)
        .iter()
        .map(|line| {
            line.iter()
                .map(|(text, _)| text.cow_replace('\u{a0}', " ").into_owned())
                .collect::<String>()
                .trim()
                .to_string()
        })
        .collect()
}

#[test]
fn a_preserved_newline_between_inline_elements_breaks_the_line() {
    let (tree, [mixed, plain]) = lay_out(
        "<body style=\"margin: 0\">\
         <pre id=\"mixed\">one <b>bold</b>\ntwo <i>it</i>\nthree</pre>\
         <pre id=\"plain\">plain one\nplain two</pre></body>",
        ["mixed", "plain"],
    );
    assert_eq!(line_texts(&tree, mixed), ["one bold", "two it", "three"]);
    let lines = lines(&tree, mixed);
    let start = lines[0][0].1.x;
    for line in &lines {
        assert!(
            (line[0].1.x - start).abs() < 0.5,
            "each line starts at the left edge: {lines:?}"
        );
    }
    let height = lines[1][0].1.y - lines[0][0].1.y;
    assert!(height > 0.0);
    assert!(
        (lines[2][0].1.y - lines[1][0].1.y - height).abs() < 0.5,
        "the lines are evenly spaced: {lines:?}"
    );
    assert_eq!(text_pieces(&tree, plain).len(), 1, "a pre of only text stays one run");
}

#[test]
fn a_preserved_newline_inside_an_inline_element_breaks_the_line() {
    let (tree, [pre]) = lay_out(
        "<body style=\"margin: 0\"><pre id=\"pre\">one <b>bold\nnext</b> more\nlast</pre></body>",
        ["pre"],
    );
    assert_eq!(line_texts(&tree, pre), ["one bold", "next more", "last"]);
}

#[test]
fn two_preserved_newlines_leave_a_blank_line() {
    let (tree, [pre]) = lay_out(
        "<body style=\"margin: 0\"><pre id=\"pre\" style=\"line-height: 20px\">one <b>a</b>\ntwo <b>b</b>\n\nfour</pre></body>",
        ["pre"],
    );
    let lines = lines(&tree, pre);
    assert_eq!(line_texts(&tree, pre), ["one a", "two b", "four"]);
    let pitch = lines[1][0].1.y - lines[0][0].1.y;
    assert!(
        (lines[2][0].1.y - lines[1][0].1.y - 2.0 * pitch).abs() < 0.5,
        "an empty line sits between two and four: {lines:?}"
    );
}

#[test]
fn a_preserved_newline_breaks_the_line_under_pre_wrap() {
    let (tree, [pre]) = lay_out(
        "<body style=\"margin: 0\"><pre id=\"pre\" style=\"white-space: pre-wrap\">one <b>bold</b>\ntwo <i>it</i>\nthree</pre></body>",
        ["pre"],
    );
    assert_eq!(line_texts(&tree, pre), ["one bold", "two it", "three"]);
}

#[test]
fn pre_wrap_text_among_inline_elements_still_wraps() {
    let (tree, [pre]) = lay_out(
        "<body style=\"margin: 0\"><pre id=\"pre\" style=\"white-space: pre-wrap; width: 120px\">\
         <b>a</b> one two three four five six seven\nend</pre></body>",
        ["pre"],
    );
    let pieces = text_pieces(&tree, pre);
    let long = pieces
        .iter()
        .find(|(text, _)| text.contains("seven"))
        .expect("the long piece");
    let end = pieces.iter().find(|(text, _)| text == "end").expect("the last line");
    assert!(long.1.width <= 120.0, "the long piece wraps inside the pre: {pieces:?}");
    assert!(
        long.1.height > end.1.height * 1.5,
        "onto more than one line: {pieces:?}"
    );
    assert!(
        end.1.y >= long.1.y + long.1.height - 0.5,
        "and the line after it starts below it"
    );
}

#[test]
fn a_newline_among_inline_elements_still_collapses_under_normal() {
    let (tree, [div]) = lay_out(
        "<body style=\"margin: 0\"><div id=\"div\">one <b>bold</b>\ntwo <i>it</i>\nthree</div></body>",
        ["div"],
    );
    assert_eq!(line_texts(&tree, div), ["one bold two it three"]);
}

/// The left edge of the first piece of text inside `node` that holds `word`.
fn x_of(tree: &LayoutTree, node: NodeId, word: &str) -> f64 {
    text_pieces(tree, node)
        .into_iter()
        .find(|(text, _)| text.contains(word))
        .unwrap_or_else(|| panic!("no text holding {word}"))
        .1
        .x
}

#[test]
fn preserved_spaces_at_the_end_of_an_inline_element_are_kept() {
    for white_space in ["pre", "pre-wrap"] {
        let (tree, [menu, column]) = lay_out(
            &format!(
                "<body style=\"margin: 0\">\
                 <pre id=\"menu\" style=\"white-space: {white_space}\"><span>DIR  </span>Next\n<span>FIND </span>Find</pre>\
                 <pre id=\"column\">12345</pre></body>"
            ),
            ["menu", "column"],
        );
        let advance = text_pieces(&tree, column)[0].1.width / 5.0;
        let next = x_of(&tree, menu, "Next");
        let find = x_of(&tree, menu, "Find");
        assert!(
            (next - 5.0 * advance).abs() < 1.0,
            "{white_space}: Next is in column 6 at {next}"
        );
        assert!(
            (find - 5.0 * advance).abs() < 1.0,
            "{white_space}: Find is in column 6 at {find}"
        );
    }
}

/// The advance of one character of the UA's monospace font, from a run of `n` of them.
fn monospace_advance(tree: &LayoutTree, column: NodeId, n: f64) -> f64 {
    text_pieces(tree, column)[0].1.width / n
}

#[test]
fn a_preserved_tab_advances_to_the_next_tab_stop() {
    let (tree, [pre, column]) = lay_out(
        "<body style=\"margin: 0\">\
         <pre id=\"pre\">bob\tName\n12345678\tx</pre>\
         <pre id=\"column\">12345678901234567890</pre></body>",
        ["pre", "column"],
    );
    let advance = monospace_advance(&tree, column, 20.0);
    assert_eq!(line_texts(&tree, pre).len(), 2, "{:?}", text_pieces(&tree, pre));
    let name = x_of(&tree, pre, "Name");
    let x = x_of(&tree, pre, "x");
    assert!((name - 8.0 * advance).abs() < 1.0, "Name is in column 9 at {name}");
    assert!((x - 16.0 * advance).abs() < 1.0, "x is in column 17 at {x}");
}

#[test]
fn tab_size_sets_the_distance_between_tab_stops() {
    let (tree, [number, length, column]) = lay_out(
        "<body style=\"margin: 0\">\
         <pre id=\"number\" style=\"tab-size: 4\">ab\tc</pre>\
         <pre id=\"length\" style=\"tab-size: 50px\">ab\tc</pre>\
         <pre id=\"column\">12345678901234567890</pre></body>",
        ["number", "length", "column"],
    );
    let advance = monospace_advance(&tree, column, 20.0);
    let c = x_of(&tree, number, "c");
    assert!((c - 4.0 * advance).abs() < 1.0, "c is in column 5 at {c}");
    let c = x_of(&tree, length, "c");
    assert!((c - 50.0).abs() < 1.0, "c is at 50px, at {c}");
}

#[test]
fn a_tab_stop_is_measured_from_the_start_of_the_line_among_inline_elements() {
    let (tree, [pre, column]) = lay_out(
        "<body style=\"margin: 0\">\
         <pre id=\"pre\">ab<b>c</b>\td\n<i>abcdefghi</i>\tz</pre>\
         <pre id=\"column\">12345678901234567890</pre></body>",
        ["pre", "column"],
    );
    let advance = monospace_advance(&tree, column, 20.0);
    let d = x_of(&tree, pre, "d");
    let z = x_of(&tree, pre, "z");
    assert!((d - 8.0 * advance).abs() < 1.0, "d is in column 9 at {d}");
    assert!((z - 16.0 * advance).abs() < 1.0, "z is in column 17 at {z}");
}

#[test]
fn spaces_before_a_tab_count_at_their_laid_out_width() {
    let (tree, [pre, column]) = lay_out(
        "<body style=\"margin: 0\">\
         <pre id=\"pre\">ab<b>c</b>   \tz</pre>\
         <pre id=\"column\">12345678901234567890</pre></body>",
        ["pre", "column"],
    );
    let advance = monospace_advance(&tree, column, 20.0);
    let z = x_of(&tree, pre, "z");
    assert!((z - 8.0 * advance).abs() < 1.0, "z is in column 9 at {z}");
}

#[test]
fn a_tab_is_not_drawn_as_a_glyph() {
    let (tree, [pre]) = lay_out("<body style=\"margin: 0\"><pre id=\"pre\">a\tb</pre></body>", ["pre"]);
    assert!(
        text_pieces(&tree, pre).iter().all(|(text, _)| !text.contains('\t')),
        "{:?}",
        text_pieces(&tree, pre)
    );
}

#[test]
fn a_tab_collapses_to_a_space_under_normal() {
    let (tree, [div, column]) = lay_out(
        "<body style=\"margin: 0; font-family: monospace\">\
         <div id=\"div\">ab\tc</div><div id=\"column\">12345678901234567890</div></body>",
        ["div", "column"],
    );
    let advance = monospace_advance(&tree, column, 20.0);
    let pieces = text_pieces(&tree, div);
    assert_eq!(pieces.len(), 1, "{pieces:?}");
    assert_eq!(pieces[0].0, "ab c");
    assert!(
        (pieces[0].1.width - 4.0 * advance).abs() < 1.0,
        "four columns wide: {pieces:?}"
    );
}

#[test]
fn a_tab_at_the_end_of_a_pre_wrap_line_hangs() {
    let (tree, [pre]) = lay_out(
        "<body style=\"margin: 0\"><div id=\"pre\" style=\"white-space: pre-wrap; width: 100px; font-family: monospace\">ab\t\tcd</div></body>",
        ["pre"],
    );
    let lines = lines(&tree, pre);
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert_eq!(lines[1].len(), 1, "only cd wraps: {lines:?}");
    assert_eq!(lines[1][0].0, "cd");
    assert!(lines[1][0].1.x.abs() < 0.5, "at the start of its line: {lines:?}");
}
